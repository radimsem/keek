use super::screen::{Sample, Screen};
use super::*;
use crate::checklist::IncompleteReason;
impl Actor {
    pub(super) async fn hand_back(
        &self,
        driver: &mut dyn Driver,
        span: gate::Span,
        baseline: &Option<Sample>,
        opener: &mut Option<watch::Receiver<()>>,
        deadline: &mut tokio::time::Instant,
    ) -> Result<Response, IncompleteReason> {
        let mut ended = driver.ended();
        let capture = self.capture_hand_back(driver, span, baseline);
        tokio::pin!(capture);
        tokio::select! { biased;
                () = &mut ended => Err(IncompleteReason::DesktopStopped),
                response = &mut capture => {
                    *deadline = tokio::time::Instant::now() + self.idle_limit;
                    Ok(response)
                },
                () = self.opener_gone(opener) => Err(IncompleteReason::OpenerLeft),
        }
    }
    async fn capture_hand_back(
        &self,
        driver: &mut dyn Driver,
        mut span: gate::Span,
        baseline: &Option<Sample>,
    ) -> Response {
        let _permit = loop {
            match self.control.gate.enter().await {
                gate::Entry::Run(permit) => break permit,
                gate::Entry::HandBack(current) => span = current,
            }
        };
        let capture = call_driver(driver, "get_desktop_state", json!({}), DRIVER_LIMIT)
            .await
            .and_then(|result| {
                if result.get("isError").and_then(Value::as_bool) == Some(true) {
                    Err(DriverError::Call {
                        tool: "get_desktop_state".into(),
                        reason: result.to_string(),
                    })
                } else {
                    Ok(result)
                }
            });
        match capture {
            Ok(result) => {
                let (mut result, unchanged) = self.compare_hand_back(&span, baseline, result).await;
                let note = hand_back_note(&span, unchanged);
                let note = format!(
                    "{note}\n{}",
                    screenshot_scale(&result, self.placement.geometry.window)
                );
                let mut content = vec![json!({"type":"text", "text":note})];
                if let Some(items) = result.get_mut("content").and_then(Value::as_array_mut) {
                    content.append(items);
                }
                if let Some(object) = result.as_object_mut() {
                    object.insert("content".into(), json!(content));
                } else {
                    result = json!({"content":content});
                }
                Response::ToolResult(result)
            }
            Err(error) => {
                tracing::warn!(%error, "Could not take a fresh screenshot after Takeover");
                let note = format!("{}\n{UNKNOWN_SCALE}", hand_back_note(&span, false));
                Response::ToolResult(
                    json!({"content":[{"type":"text", "text":note}, {"type":"text", "text":"keek couldn't take a fresh screenshot. Take one before you act."}]}),
                )
            }
        }
    }

    async fn compare_hand_back(
        &self,
        span: &gate::Span,
        baseline: &Option<Sample>,
        result: Value,
    ) -> (Value, bool) {
        let before = baseline
            .as_ref()
            .filter(|sample| sample.start == self.control_time(span.start))
            .and_then(|sample| sample.screen.as_ref());
        let Some(before) = before else {
            return (result, false);
        };
        let result = Arc::new(result);
        let captured = Arc::clone(&result);
        let after = tokio::task::spawn_blocking(move || Screen::decode(&captured))
            .await
            .ok()
            .flatten();
        let unchanged = after.is_some_and(|after| before.looks_unchanged(&after));
        if unchanged {
            self.control.mark_unanswered(span.start).await;
            let end = self.control_time(span.end);
            self.events.send_if_modified(|snapshot| {
                if let crate::ipc::ControlState::HandBack {
                    end_ms,
                    result: card,
                    ..
                } = &mut snapshot.control
                    && *end_ms == end
                    && *card != card.unchanged()
                {
                    *card = card.unchanged();
                    return true;
                }
                false
            });
        }
        (Arc::unwrap_or_clone(result), unchanged)
    }
}

fn hand_back_note(span: &gate::Span, unchanged: bool) -> String {
    let seconds = span.end.saturating_duration_since(span.start).as_secs();
    let length = format!("{}:{:02}", seconds / 60, seconds % 60);
    let reason = span
        .reason
        .as_ref()
        .map(|reason| format!(" ({reason})"))
        .unwrap_or_default();
    let status = if unchanged {
        "The screen looks unchanged, so if you raised a Help Request, it may not be answered. Re-read the screen, then call request_help again or skip the Check with a comment."
    } else {
        "If you raised a Help Request, it is answered. The screen may have changed."
    };
    format!("The developer pressed Done and handed control back after {length}{reason}. {status}")
}

const UNKNOWN_SCALE: &str =
    "The screenshot's scale is unknown. Call get_window_state before acting on coordinates.";

fn screenshot_scale(result: &Value, desktop: crate::slots::Slot) -> String {
    let size = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find_map(image_size));
    let Some((width, height)) = size else {
        return UNKNOWN_SCALE.into();
    };
    let factor = |desktop: i32, image: u32| {
        (f64::from(desktop) / f64::from(image) * 1000.0).round() / 1000.0
    };
    let x = factor(desktop.width, width);
    let y = factor(desktop.height, height);
    format!(
        "The screenshot is {width} x {height} px of a {} x {} desktop, a scale of {x} by {y}. Take click coordinates from a fresh get_window_state.",
        desktop.width, desktop.height
    )
}

fn image_size(item: &Value) -> Option<(u32, u32)> {
    if item.get("type")?.as_str()? != "image" {
        return None;
    }
    let bytes = decode_base64(item.get("data")?.as_str()?)?;
    let (width, height) = match item.get("mimeType")?.as_str()? {
        "image/png" => {
            if bytes.get(..16)? != b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR" {
                return None;
            }
            (
                u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?),
                u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?),
            )
        }
        "image/jpeg" => jpeg_size(&bytes)?,
        _ => return None,
    };
    (width > 0 && height > 0).then_some((width, height))
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(..2)? != b"\xff\xd8" {
        return None;
    }
    let mut offset = 2;
    loop {
        if *bytes.get(offset)? != 0xff {
            return None;
        }
        while *bytes.get(offset)? == 0xff {
            offset += 1;
        }
        let marker = *bytes.get(offset)?;
        offset += 1;
        match marker {
            0xd9 | 0xda => return None,
            0x01 | 0xd0..=0xd7 => continue,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes(
            bytes.get(offset..offset + 2)?.try_into().ok()?,
        ));
        if length < 2 {
            return None;
        }
        let segment = bytes.get(offset..offset + length)?;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let height = u16::from_be_bytes(segment.get(3..5)?.try_into().ok()?);
            let width = u16::from_be_bytes(segment.get(5..7)?.try_into().ok()?);
            return Some((u32::from(width), u32::from(height)));
        }
        offset += length;
    }
}

pub(super) fn decode_base64(data: &str) -> Option<Vec<u8>> {
    let value = |byte| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let chunks = data.as_bytes().chunks_exact(4);
    if !chunks.remainder().is_empty() {
        return None;
    }
    let count = chunks.len();
    let mut bytes = Vec::new();
    for (index, chunk) in chunks.enumerate() {
        let [a, b, c, d] = <[u8; 4]>::try_from(chunk).ok()?;
        let (a, b) = (value(a)?, value(b)?);
        bytes.push(a << 2 | b >> 4);
        if c == b'=' {
            return (index + 1 == count && d == b'=' && b & 15 == 0).then_some(bytes);
        }
        let c = value(c)?;
        bytes.push(b << 4 | c >> 2);
        if d == b'=' {
            return (index + 1 == count && c & 3 == 0).then_some(bytes);
        }
        bytes.push(c << 6 | value(d)?);
    }
    Some(bytes)
}
