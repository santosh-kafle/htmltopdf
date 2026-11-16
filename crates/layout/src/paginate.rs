//! Cutting the continuous canvas into pages.

use std::rc::Rc;

use presswork_style::Side;

use crate::flow::{Canvas, RepeatedHeader};
use crate::fragment::{Item, Page};
use crate::geometry::{EdgeSizes, Rect, Size};

/// A page under construction.
struct Sheet {
    /// Canvas position shown at the top of the page's content area, below
    /// any repeated header.
    offset: f64,
    /// Height of the table header repeated at the top, if any.
    header: f32,
    boxes: Vec<Item>,
    content: Vec<Item>,
}

/// A repeated header taller than this fraction of the page is not
/// repeated: it would leave too little room for the rows themselves.
const MAX_HEADER_FRACTION: f32 = 0.5;

/// Places the canvas's content onto pages of `size` with `margins`.
///
/// A slice that does not fit on the current page starts a new one, unless
/// it is the first on its page (it is then too tall for any page and is
/// left to overflow). The space before a slice that starts a page is
/// dropped, so margins do not carry over a page break. Backgrounds and
/// borders are split where the page breaks fall.
pub(crate) fn paginate(canvas: Canvas, size: Size, margins: EdgeSizes) -> Vec<Page> {
    let available = (size.height - margins.top - margins.bottom).max(1.0);
    let sheet = |offset: f64, header: f32| Sheet {
        offset,
        header,
        boxes: Vec::new(),
        content: Vec::new(),
    };

    let mut sheets = vec![sheet(0.0, 0.0)];
    let mut empty = true;
    for slice in canvas.slices {
        let current = sheets.last().expect("at least one page");
        let bottom = (slice.top - current.offset) as f32 + current.header + slice.height;
        if !empty && (slice.break_before || bottom > available) {
            let header: Option<Rc<RepeatedHeader>> = slice
                .header
                .clone()
                .filter(|header| header.height <= available * MAX_HEADER_FRACTION);
            let mut next = sheet(slice.top, header.as_ref().map_or(0.0, |h| h.height));
            if let Some(header) = header {
                next.content.extend(
                    header
                        .items
                        .iter()
                        .cloned()
                        .map(|item| item.translate(margins.left, margins.top)),
                );
            }
            sheets.push(next);
        }
        let current = sheets.last_mut().expect("at least one page");
        let y = margins.top + current.header + (slice.top - current.offset) as f32;
        current.content.extend(
            slice
                .items
                .into_iter()
                .map(|item| item.translate(margins.left, y)),
        );
        empty = false;
    }

    // Split each decoration across the pages it spans.
    for decoration in &canvas.decorations {
        for index in 0..sheets.len() {
            let start = sheets[index].offset;
            let page_end = start + f64::from(available - sheets[index].header);
            let end = match sheets.get(index + 1) {
                Some(next) => page_end.min(next.offset),
                None => page_end,
            };
            let top = decoration.top.max(start);
            let bottom = decoration.bottom.min(end);
            if bottom <= top {
                continue;
            }
            let mut paint = decoration.paint;
            if decoration.top < start {
                paint = paint.without_border(Side::Top);
            }
            if decoration.bottom > end {
                paint = paint.without_border(Side::Bottom);
            }
            let sheet = &mut sheets[index];
            let rect = Rect::new(
                margins.left + decoration.x,
                margins.top + sheet.header + (top - start) as f32,
                decoration.width,
                (bottom - top) as f32,
            );
            paint.fills(rect, &mut sheet.boxes);
        }
    }

    sheets
        .into_iter()
        .map(|sheet| {
            let mut items = sheet.boxes;
            items.extend(sheet.content);
            Page { size, items }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{BoxPaint, Decoration, Slice};
    use crate::fragment::Fill;
    use presswork_style::Color;

    const RED: Color = Color::rgb(255, 0, 0);

    fn slice(top: f64, height: f32) -> Slice {
        Slice {
            top,
            height,
            items: vec![Item::Fill(Fill {
                rect: Rect::new(0.0, 0.0, 10.0, height),
                color: Color::BLACK,
            })],
            break_before: false,
            header: None,
        }
    }

    fn canvas(slices: Vec<Slice>) -> Canvas {
        Canvas {
            slices,
            decorations: Vec::new(),
        }
    }

    fn fills(page: &Page) -> Vec<(f32, f32, Color)> {
        page.items
            .iter()
            .filter_map(|item| match item {
                Item::Fill(fill) => Some((fill.rect.origin.y, fill.rect.size.height, fill.color)),
                Item::Text(_) | Item::Image(_) => None,
            })
            .collect()
    }

    fn tops(page: &Page) -> Vec<f32> {
        fills(page).into_iter().map(|(y, _, _)| y).collect()
    }

    /// 100pt of content height per page.
    const SIZE: Size = Size {
        width: 100.0,
        height: 120.0,
    };
    const MARGINS: EdgeSizes = EdgeSizes::uniform(10.0);

    #[test]
    fn breaks_before_a_slice_that_does_not_fit() {
        let pages = paginate(
            canvas(vec![slice(0.0, 40.0), slice(50.0, 40.0), slice(95.0, 40.0)]),
            SIZE,
            MARGINS,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(tops(&pages[0]), [10.0, 60.0]);
        // The gap before the third slice is dropped at the page break.
        assert_eq!(tops(&pages[1]), [10.0]);
    }

    #[test]
    fn oversized_slice_at_page_top_stays() {
        let pages = paginate(canvas(vec![slice(0.0, 500.0)]), SIZE, MARGINS);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn empty_document_has_one_page() {
        let pages = paginate(Canvas::default(), SIZE, MARGINS);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn forced_break_starts_a_new_page() {
        let mut second = slice(20.0, 10.0);
        second.break_before = true;
        let mut first = slice(0.0, 10.0);
        // A forced break before the very first slice does nothing.
        first.break_before = true;
        let pages = paginate(canvas(vec![first, second]), SIZE, MARGINS);
        assert_eq!(pages.len(), 2);
        assert_eq!(tops(&pages[1]), [10.0]);
    }

    #[test]
    fn positions_stay_exact_far_down_a_long_document() {
        // 100,000 pages down, an f32 canvas would be off by whole points.
        let top = 100_000.0 * 100.0 + 0.25;
        let pages = paginate(
            canvas(vec![
                slice(0.0, 10.0),
                slice(top, 10.0),
                slice(top + 12.5, 10.0),
            ]),
            SIZE,
            MARGINS,
        );
        assert_eq!(tops(&pages[1]), [10.0, 22.5]);
    }

    #[test]
    fn table_header_repeats_on_the_next_page() {
        let header = Rc::new(RepeatedHeader {
            height: 15.0,
            items: vec![Item::Fill(Fill {
                rect: Rect::new(0.0, 0.0, 10.0, 15.0),
                color: RED,
            })],
        });
        let mut rows: Vec<Slice> = (0..4)
            .map(|i| slice(15.0 + 30.0 * f64::from(i), 30.0))
            .collect();
        for row in &mut rows {
            row.header = Some(Rc::clone(&header));
        }
        let mut slices = vec![slice(0.0, 15.0)];
        slices.extend(rows);
        let pages = paginate(canvas(slices), SIZE, MARGINS);
        assert_eq!(pages.len(), 2);
        // Second page: the header, then the two remaining rows below it.
        assert_eq!(
            fills(&pages[1]),
            [
                (10.0, 15.0, RED),
                (25.0, 30.0, Color::BLACK),
                (55.0, 30.0, Color::BLACK)
            ]
        );
    }

    #[test]
    fn decorations_split_at_page_breaks_and_paint_first() {
        let paint = BoxPaint {
            background: RED,
            border: Side::ALL.map(|side| (side, 0.0, Color::BLACK)),
        };
        let mut content = canvas(vec![slice(10.0, 60.0), slice(70.0, 60.0)]);
        content.decorations.push(Decoration {
            top: 5.0,
            bottom: 135.0,
            x: 0.0,
            width: 50.0,
            paint,
        });
        let pages = paginate(content, SIZE, MARGINS);
        assert_eq!(pages.len(), 2);
        // Page 1: the box from its top to the break, then the content.
        assert_eq!(fills(&pages[0])[0], (15.0, 65.0, RED));
        // Page 2: the rest of the box, starting at the top of the page.
        assert_eq!(fills(&pages[1])[0], (10.0, 65.0, RED));
        assert_eq!(fills(&pages[1])[1].2, Color::BLACK);
    }
}
