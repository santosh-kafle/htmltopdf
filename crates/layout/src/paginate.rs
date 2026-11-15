//! Cutting the continuous canvas into pages.

use crate::flow::Slice;
use crate::fragment::Page;
use crate::geometry::{EdgeSizes, Size};

/// Places slices onto pages of `size` with `margins`.
///
/// A slice that does not fit on the current page starts a new one, unless it
/// is already at the top of a page (it is then too tall for any page and is
/// left to overflow). The space before a slice that starts a page is dropped,
/// so margins do not carry over a page break.
pub(crate) fn paginate(slices: Vec<Slice>, size: Size, margins: EdgeSizes) -> Vec<Page> {
    let available = (size.height - margins.top - margins.bottom).max(1.0);
    let new_page = || Page {
        size,
        items: Vec::new(),
    };

    let mut pages = vec![new_page()];
    // Canvas y-coordinate of the top of the current page's content area.
    let mut offset = 0.0;
    for slice in slices {
        if slice.bottom - offset > available && slice.top > offset {
            pages.push(new_page());
            offset = slice.top;
        }
        let page = pages.last_mut().expect("at least one page");
        let (dx, dy) = (margins.left, margins.top - offset);
        page.items
            .extend(slice.items.into_iter().map(|item| item.translate(dx, dy)));
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::Item;
    use crate::geometry::Rect;

    fn slice(top: f32, bottom: f32) -> Slice {
        Slice {
            top,
            bottom,
            items: vec![Item::Rule(Rect::new(0.0, top, 10.0, bottom - top))],
        }
    }

    fn tops(page: &Page) -> Vec<f32> {
        page.items
            .iter()
            .map(|item| match item {
                Item::Rule(r) => r.origin.y,
                Item::Text(t) => t.origin.y,
            })
            .collect()
    }

    #[test]
    fn breaks_before_a_slice_that_does_not_fit() {
        let size = Size {
            width: 100.0,
            height: 120.0,
        };
        let margins = EdgeSizes::uniform(10.0);
        // 100pt of content height per page.
        let pages = paginate(
            vec![slice(0.0, 40.0), slice(50.0, 90.0), slice(95.0, 135.0)],
            size,
            margins,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(tops(&pages[0]), [10.0, 60.0]);
        // The gap before the third slice is dropped at the page break.
        assert_eq!(tops(&pages[1]), [10.0]);
    }

    #[test]
    fn oversized_slice_at_page_top_stays() {
        let size = Size {
            width: 100.0,
            height: 50.0,
        };
        let pages = paginate(vec![slice(0.0, 500.0)], size, EdgeSizes::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn empty_document_has_one_page() {
        let pages = paginate(Vec::new(), Size::default(), EdgeSizes::default());
        assert_eq!(pages.len(), 1);
    }
}
