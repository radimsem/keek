#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
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
    if w <= 0 || card_h <= FOOTER_HEIGHT - FOOTER_OVERLAP {
        return vec![];
    }
    (0..n)
        .map(|i| Slot {
            x: left + outer + if stacked { 0 } else { i * (w + gap) },
            y: top + outer + if stacked { i * (card_h + gap) } else { 0 },
            width: w,
            height: card_h - (FOOTER_HEIGHT - FOOTER_OVERLAP),
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
                    width: 1691,
                    height: 1298
                },
                Slot {
                    x: 1725,
                    y: 50,
                    width: 1691,
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
                width: 931,
                height: 938
            }
        );
        assert_eq!(s[1].x, 965);
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
}
