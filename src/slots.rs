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

pub const RAIL_GAP: i32 = 11;
pub const RAIL_CARD_CHROME: i32 = 128;
pub const WIDE_MONITOR: i32 = 1920;
pub const FOOTER_HEIGHT: i32 = 76;
pub const FOOTER_OVERLAP: i32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailShape {
    Column,
    Row,
}

impl RailShape {
    pub fn for_width(width: i32) -> Self {
        if width >= WIDE_MONITOR {
            Self::Column
        } else {
            Self::Row
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub reserved: [i32; 4],
    pub outer: i32,
    pub gap: i32,
    pub count: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLayout {
    pub focus: Slot,
    pub alone: Slot,
    pub rail: Vec<Slot>,
}

pub fn workspace(input: Layout) -> Option<WorkspaceLayout> {
    let Layout {
        width,
        height,
        reserved: [left, top, right, bottom],
        outer,
        gap,
        count,
    } = input;
    let n = i32::try_from(count.checked_sub(1)?).ok()?;
    let w = width - left - right - 2 * outer;
    let h = height - top - bottom - 2 * outer;
    let footer = FOOTER_HEIGHT - FOOTER_OVERLAP;
    let alone = Slot {
        x: left + outer,
        y: top + outer,
        // libx264 requires even dimensions for yuv420p video.
        width: w & !1,
        height: (h - footer) & !1,
    };
    if alone.width <= 0 || alone.height <= 0 {
        return None;
    }
    if n == 0 {
        return Some(WorkspaceLayout {
            focus: alone,
            alone,
            rail: vec![],
        });
    }
    let wide = RailShape::for_width(width) == RailShape::Column;
    let rail_gaps = i64::from(n - 1) * i64::from(RAIL_GAP);
    let (fw, fh, card_w) = if wide {
        let card_w = w / 6 / 20 * 20;
        ((w - gap - card_w) & !1, alone.height, card_w)
    } else {
        let card_w = i32::try_from((i64::from(w) - rail_gaps) / i64::from(n)).ok()?;
        if card_w <= 0 {
            return None;
        }
        let available =
            i64::from(h) - i64::from(footer) - i64::from(gap) - i64::from(RAIL_CARD_CHROME);
        let width = i64::from(w);
        let divisor = width + i64::from(card_w) - 4;
        if divisor <= 0 {
            return None;
        }
        let fh = i32::try_from(available * width / divisor).ok()? & !1;
        (alone.width, fh, card_w)
    };
    if fw <= 0 || fh <= 0 || card_w <= 0 {
        return None;
    }
    let a = i64::from(card_w - 4) * i64::from(fh);
    let b = i64::from(fw);
    let mut card_h = (2 * a + b) / (2 * b) + i64::from(RAIL_CARD_CHROME);
    if wide && card_h > (i64::from(h) - rail_gaps) / i64::from(n) {
        card_h = (i64::from(h) - rail_gaps) / i64::from(n);
    }
    let card_h = i32::try_from(card_h).ok()?;
    if card_h <= 0 {
        return None;
    }
    let focus = Slot {
        width: fw,
        height: fh,
        ..alone
    };
    let rail = (0..n)
        .map(|i| Slot {
            x: alone.x
                + if wide {
                    fw + gap
                } else {
                    i * (card_w + RAIL_GAP)
                },
            y: alone.y
                + if wide {
                    i * (card_h + RAIL_GAP)
                } else {
                    fh + footer + gap
                },
            width: card_w,
            height: card_h,
        })
        .collect();
    Some(WorkspaceLayout { focus, alone, rail })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn wide(places: usize) -> Option<WorkspaceLayout> {
        workspace(Layout {
            width: 3440,
            height: 1440,
            reserved: [0, 26, 0, 0],
            outer: 24,
            gap: 10,
            count: places,
        })
    }
    fn slot(x: i32, y: i32, width: i32, height: i32) -> Slot {
        Slot {
            x,
            y,
            width,
            height,
        }
    }
    #[test]
    fn a_wide_monitor_puts_the_rail_right_of_the_focus_slot_as_the_canvas_draws_it() {
        let layout = wide(4).unwrap();
        assert_eq!(layout.focus, slot(24, 50, 2822, 1298));
        assert_eq!(
            layout.rail,
            vec![
                slot(2856, 50, 560, 384),
                slot(2856, 445, 560, 384),
                slot(2856, 840, 560, 384)
            ]
        );
    }
    #[test]
    fn without_a_rail_the_focus_slot_spans_the_workspace() {
        let layout = wide(4).unwrap();
        assert_eq!(layout.alone, slot(24, 50, 3392, 1298));
        assert_eq!(wide(1).unwrap().rail, vec![]);
    }
    #[test]
    fn full_hd_is_wide_enough_for_a_rail_column() {
        let layout = workspace(Layout {
            width: 1920,
            height: 1080,
            reserved: [0, 26, 0, 0],
            outer: 24,
            gap: 10,
            count: 4,
        })
        .unwrap();
        assert_eq!(layout.focus, slot(24, 50, 1562, 938));
        assert_eq!(layout.rail[0], slot(1596, 50, 300, 306));
        assert_eq!(layout.rail[1].y, 50 + 306 + RAIL_GAP);
    }
    #[test]
    fn a_narrow_monitor_puts_the_rail_below_the_focus_slot_as_a_row() {
        let layout = workspace(Layout {
            width: 1280,
            height: 1024,
            reserved: [0, 26, 0, 0],
            outer: 24,
            gap: 10,
            count: 4,
        })
        .unwrap();
        assert_eq!(layout.focus, slot(24, 50, 1232, 560));
        assert_eq!(layout.alone, slot(24, 50, 1232, 882));
        assert_eq!(
            layout.rail,
            vec![
                slot(24, 688, 403, 309),
                slot(438, 688, 403, 309),
                slot(852, 688, 403, 309)
            ]
        );
    }
    #[test]
    fn rail_cards_shrink_to_fit_a_cap_above_four() {
        let layout = wide(8).unwrap();
        assert_eq!(layout.rail.len(), 7);
        let last = layout.rail.last().unwrap();
        assert!(last.y + last.height <= 50 + 1298 + FOOTER_HEIGHT - FOOTER_OVERLAP);
        assert!(
            layout
                .rail
                .windows(2)
                .all(|pair| pair[1].y == pair[0].y + pair[0].height + RAIL_GAP)
        );
    }
    #[test]
    fn focus_and_alone_slots_have_even_sizes_so_the_video_of_a_full_slot_window_encodes() {
        for (width, height) in [(1920, 1080), (1366, 768), (3440, 1440), (1280, 1024)] {
            for count in 1..=4 {
                let layout = workspace(Layout {
                    width,
                    height,
                    reserved: [0, 26, 1, 3],
                    outer: 23,
                    gap: 9,
                    count,
                })
                .unwrap();
                for slot in [layout.focus, layout.alone] {
                    assert_eq!(slot.width % 2, 0, "{width}x{height} x{count}: {slot:?}");
                    assert_eq!(slot.height % 2, 0, "{width}x{height} x{count}: {slot:?}");
                }
            }
        }
    }
    #[test]
    fn no_place_or_no_room_gives_no_layout() {
        assert_eq!(wide(0), None);
        assert_eq!(
            workspace(Layout {
                width: 100,
                height: 100,
                reserved: [0, 26, 0, 0],
                outer: 24,
                gap: 10,
                count: 2,
            }),
            None
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
