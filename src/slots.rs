#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardGeometry {
    pub body: Slot,
    pub window: Slot,
    pub footer: Slot,
}

pub fn geometry(slot: Slot, size: Option<(i32, i32)>) -> CardGeometry {
    let (width, height) = size.unwrap_or((slot.width, slot.height));
    let window = Slot {
        x: slot.x + (slot.width - width) / 2,
        y: slot.y + (slot.height - height) / 2,
        width,
        height,
    };
    CardGeometry {
        body: slot,
        window,
        footer: Slot {
            y: slot.y + slot.height - FOOTER_OVERLAP,
            height: FOOTER_HEIGHT,
            ..slot
        },
    }
}

pub const MIN_COLUMN_WIDTH: i32 = 960;
pub const FOOTER_HEIGHT: i32 = 76;
pub const FOOTER_OVERLAP: i32 = 8;
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub reserved: [i32; 4],
    pub outer: i32,
    pub gap: i32,
    pub count: usize,
}
pub fn layout(input: Layout) -> Vec<Slot> {
    let Layout {
        width,
        height,
        reserved,
        outer,
        gap,
        count,
    } = input;
    let Ok(n) = i32::try_from(count) else {
        return vec![];
    };
    if n <= 0 {
        return vec![];
    }
    let [left, top, right, bottom] = reserved;
    let usable_width = width - left - right - 2 * outer;
    let usable_height = height - top - bottom - 2 * outer;
    let stacked = width / n < MIN_COLUMN_WIDTH;
    let w = if stacked {
        usable_width
    } else {
        (usable_width - gap * (n - 1)) / n
    };
    let card_h = if stacked {
        (usable_height - gap * (n - 1)) / n
    } else {
        usable_height
    };
    let footer = FOOTER_HEIGHT - FOOTER_OVERLAP;
    // libx264 requires even dimensions for yuv420p video.
    let w = w & !1;
    let h = card_h.saturating_sub(footer) & !1;
    if w <= 0 || h <= 0 {
        return vec![];
    }
    (0..n)
        .map(|i| Slot {
            x: left + outer + if stacked { 0 } else { i * (w + gap) },
            y: top + outer + if stacked { i * (h + footer + gap) } else { 0 },
            width: w,
            height: h,
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ultrawide_matches_the_working_comp() {
        assert_eq!(
            layout(Layout {
                width: 3440,
                height: 1440,
                reserved: [0, 26, 0, 0],
                outer: 24,
                gap: 10,
                count: 2
            }),
            vec![
                Slot {
                    x: 24,
                    y: 50,
                    width: 1690,
                    height: 1298
                },
                Slot {
                    x: 1724,
                    y: 50,
                    width: 1690,
                    height: 1298
                }
            ]
        );
    }
    #[test]
    fn full_hd_uses_two_columns() {
        let s = layout(Layout {
            width: 1920,
            height: 1080,
            reserved: [0, 26, 0, 0],
            outer: 24,
            gap: 10,
            count: 2,
        });
        assert_eq!(s.len(), 2);
        assert_eq!(
            s[0],
            Slot {
                x: 24,
                y: 50,
                width: 930,
                height: 938
            }
        );
        assert_eq!(s[1].x, 964);
    }
    #[test]
    fn slots_have_even_sizes_so_the_video_of_a_full_slot_window_encodes() {
        for (width, height) in [(1920, 1080), (1366, 768), (3440, 1440), (1280, 1024)] {
            for count in 1..=4 {
                for slot in layout(Layout {
                    width,
                    height,
                    reserved: [0, 26, 1, 3],
                    outer: 23,
                    gap: 9,
                    count,
                }) {
                    assert_eq!(slot.width % 2, 0, "{width}x{height} x{count}: {slot:?}");
                    assert_eq!(slot.height % 2, 0, "{width}x{height} x{count}: {slot:?}");
                }
            }
        }
    }
    #[test]
    fn narrow_monitors_stack_cards() {
        let s = layout(Layout {
            width: 1280,
            height: 1024,
            reserved: [0, 26, 0, 0],
            outer: 24,
            gap: 10,
            count: 2,
        });
        assert_eq!(
            s,
            vec![
                Slot {
                    x: 24,
                    y: 50,
                    width: 1232,
                    height: 402
                },
                Slot {
                    x: 24,
                    y: 530,
                    width: 1232,
                    height: 402
                }
            ]
        );
    }
    #[test]
    fn a_phone_window_is_centred_above_a_full_width_footer() {
        let body = Slot {
            x: 24,
            y: 50,
            width: 1712,
            height: 1300,
        };
        let card = geometry(body, Some((390, 844)));
        assert_eq!(card.body, body);
        assert_eq!(
            card.window,
            Slot {
                x: 685,
                y: 278,
                width: 390,
                height: 844
            }
        );
        assert_eq!(
            card.footer,
            Slot {
                x: 24,
                y: 1342,
                width: 1712,
                height: 76
            }
        );
    }

    #[test]
    fn a_window_that_fills_its_slot_keeps_its_placement_and_footer_overlap() {
        let body = Slot {
            x: 24,
            y: 50,
            width: 1712,
            height: 1300,
        };
        let card = geometry(body, Some((1712, 1300)));
        assert_eq!(card.body, body);
        assert_eq!(card.window, body);
        assert_eq!(
            card.footer,
            Slot {
                x: 24,
                y: 1342,
                width: 1712,
                height: 76
            }
        );
        assert_eq!(geometry(body, None), card);
    }

    #[test]
    fn odd_leftovers_split_without_overflowing_the_slot() {
        let body = Slot {
            x: 10,
            y: 20,
            width: 101,
            height: 101,
        };
        let card = geometry(body, Some((98, 96)));
        assert_eq!(
            card.window,
            Slot {
                x: 11,
                y: 22,
                width: 98,
                height: 96
            }
        );
        assert_eq!(body.x + body.width - card.window.x - card.window.width, 2);
        assert_eq!(body.y + body.height - card.window.y - card.window.height, 3);
        assert_eq!(
            card.footer,
            Slot {
                x: 10,
                y: 113,
                width: 101,
                height: 76
            }
        );
    }
}
