use super::{Actor, call_driver, hand_back::decode_base64, run::LoopState};
use crate::ipc::ControlState;
use crate::timeline::Millis;
#[cfg(test)]
use gtk4::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk4::gdk_pixbuf::{PixbufLoader, prelude::PixbufLoaderExt};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

// Ignore small channel shifts from JPEG compression.
const CHANNEL_TOLERANCE: u8 = 16;
// The caret, Cua cursor and animated chart changed about 100 pixels at a time at 1440 × 960.
const PIXELS_PER_ALLOWED_CHANGE: usize = 5000;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Screen {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Screen {
    pub(super) fn decode(result: &Value) -> Option<Self> {
        let item = result["content"].as_array()?.iter().find(|item| {
            item["type"] == "image"
                && matches!(item["mimeType"].as_str(), Some("image/png" | "image/jpeg"))
        })?;
        let bytes = decode_base64(item["data"].as_str()?)?;
        let loader = PixbufLoader::with_mime_type(item["mimeType"].as_str()?).ok()?;
        let written = loader.write(&bytes);
        let closed = loader.close();
        written.ok()?;
        closed.ok()?;
        let pixbuf = loader.pixbuf()?;
        let width = pixbuf.width() as usize;
        let height = pixbuf.height() as usize;
        let stride = pixbuf.rowstride() as usize;
        let channels = pixbuf.n_channels() as usize;
        let mut pixels = Vec::with_capacity(width * height * 3);
        for row in pixbuf.read_pixel_bytes().chunks(stride).take(height) {
            for pixel in row[..width * channels].chunks_exact(channels) {
                pixels.extend_from_slice(&pixel[..3]);
            }
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    pub(super) fn looks_unchanged(&self, after: &Self) -> bool {
        if self.width != after.width || self.height != after.height {
            return false;
        }
        let allowed = self.width * self.height / PIXELS_PER_ALLOWED_CHANGE;
        self.pixels
            .chunks_exact(3)
            .zip(after.pixels.chunks_exact(3))
            .filter(|(before, after)| {
                before
                    .iter()
                    .zip(*after)
                    .any(|(a, b)| a.abs_diff(*b) > CHANNEL_TOLERANCE)
            })
            .nth(allowed)
            .is_none()
    }
}

pub(super) struct Sample {
    pub(super) start: Millis,
    pub(super) screen: Option<Screen>,
}

impl Actor {
    pub(super) fn control_time(&self, time: Instant) -> Millis {
        Millis(
            time.saturating_duration_since(self.opened_at)
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        )
    }

    pub(super) async fn sample_takeover(&self, state: &mut LoopState, control: &ControlState) {
        let ControlState::Takeover { start_ms, .. } = control else {
            return;
        };
        if state
            .takeover_screen
            .as_ref()
            .is_some_and(|sample| sample.start == *start_ms)
        {
            return;
        }
        let screen = match call_driver(
            state.driver.as_mut(),
            "get_desktop_state",
            json!({}),
            Duration::from_secs(10),
        )
        .await
        {
            Ok(result) if result["isError"] != true => {
                tokio::task::spawn_blocking(move || Screen::decode(&result))
                    .await
                    .ok()
                    .flatten()
            }
            Ok(_) | Err(_) => None,
        };
        if screen.is_none() {
            tracing::warn!("Could not capture the screen at Take control");
        }
        state.takeover_screen = Some(Sample {
            start: *start_ms,
            screen,
        });
    }
}

#[cfg(test)]
pub(super) fn fake_screenshot(width: i32, height: i32, paint: impl FnOnce(&Pixbuf)) -> Value {
    fake_image(width, height, "png", paint)
}

#[cfg(test)]
fn fake_image(width: i32, height: i32, format: &str, paint: impl FnOnce(&Pixbuf)) -> Value {
    let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, width, height).unwrap();
    paint(&pixbuf);
    let png = pixbuf.save_to_bufferv(format, &[]).unwrap();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut data = String::new();
    for chunk in png.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        data.push(char::from(alphabet[usize::from(a >> 2)]));
        data.push(char::from(alphabet[usize::from((a & 3) << 4 | b >> 4)]));
        data.push(if chunk.len() > 1 {
            char::from(alphabet[usize::from((b & 15) << 2 | c >> 6)])
        } else {
            '='
        });
        data.push(if chunk.len() > 2 {
            char::from(alphabet[usize::from(c & 63)])
        } else {
            '='
        });
    }
    json!({"content":[{"type":"image","mimeType":format!("image/{format}"),"data":data}]})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black() -> Screen {
        Screen {
            width: 200,
            height: 100,
            pixels: vec![0; 200 * 100 * 3],
        }
    }

    #[test]
    fn identical_screens_look_unchanged() {
        assert!(black().looks_unchanged(&black()));
    }

    #[test]
    fn two_changed_pixels_look_unchanged() {
        let mut after = black();
        after.pixels[..6].fill(255);
        assert!(black().looks_unchanged(&after));
    }

    #[test]
    fn a_changed_block_counts_as_a_change() {
        let mut after = black();
        for row in after.pixels.chunks_exact_mut(200 * 3).take(10) {
            row[..20 * 3].fill(255);
        }
        assert!(!black().looks_unchanged(&after));
    }

    #[test]
    fn different_dimensions_count_as_a_change() {
        let mut after = black();
        std::mem::swap(&mut after.width, &mut after.height);
        assert!(!black().looks_unchanged(&after));
    }

    #[test]
    fn a_channel_difference_of_sixteen_is_ignored_and_seventeen_counts() {
        let mut after = black();
        after.pixels.fill(16);
        assert!(black().looks_unchanged(&after));
        for pixel in after.pixels.chunks_exact_mut(3).take(5) {
            pixel[2] = 17;
        }
        assert!(!black().looks_unchanged(&after));
    }

    #[test]
    fn a_png_decodes_to_packed_rgb_pixels() {
        let screenshot = fake_screenshot(2, 2, |pixbuf| {
            pixbuf.fill(0x123456ff);
            pixbuf.new_subpixbuf(0, 1, 2, 1).fill(0xabcdefff);
        });
        assert_eq!(
            Screen::decode(&screenshot),
            Some(Screen {
                width: 2,
                height: 2,
                pixels: vec![
                    0x12, 0x34, 0x56, 0x12, 0x34, 0x56, 0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef
                ]
            })
        );
    }

    #[test]
    fn a_jpeg_decodes_to_its_size() {
        let screenshot = fake_image(16, 8, "jpeg", |pixbuf| pixbuf.fill(0x808080ff));
        let screen = Screen::decode(&screenshot).unwrap();
        assert_eq!((screen.width, screen.height), (16, 8));
        assert!(screen.pixels.iter().all(|value| value.abs_diff(0x80) <= 2));
    }

    #[test]
    fn missing_and_invalid_images_do_not_decode() {
        assert_eq!(Screen::decode(&json!({"content":[{"type":"text"}]})), None);
        assert_eq!(
            Screen::decode(
                &json!({"content":[{"type":"image","mimeType":"image/png","data":"AA=="}]})
            ),
            None
        );
    }
}
