//! The output of layout: positioned content, one list per page.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

pub use parley::FontData;

use crate::geometry::{Point, Rect, Size};

/// One laid-out page. Coordinates are in points from the page's top-left.
#[derive(Debug, Clone)]
pub struct Page {
    pub size: Size,
    pub items: Vec<Item>,
}

/// A piece of positioned content.
#[derive(Debug, Clone)]
pub enum Item {
    Text(TextRun),
    /// A filled rectangle, e.g. an `<hr>`.
    Rule(Rect),
}

impl Item {
    pub(crate) fn translate(mut self, dx: f32, dy: f32) -> Self {
        match &mut self {
            Item::Text(run) => {
                run.origin.x += dx;
                run.origin.y += dy;
            }
            Item::Rule(rect) => {
                rect.origin.x += dx;
                rect.origin.y += dy;
            }
        }
        self
    }
}

/// Shaped glyphs in a single font and size, drawn left to right from
/// `origin`, which is on the baseline.
#[derive(Clone)]
pub struct TextRun {
    pub font: FontData,
    /// Font size in points.
    pub font_size: f32,
    pub origin: Point,
    pub glyphs: Vec<Glyph>,
    /// The text the glyphs' `text_range`s index into, so the PDF can map
    /// glyphs back to characters for copying and search.
    pub text: Arc<str>,
}

impl fmt::Debug for TextRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: String = self
            .glyphs
            .first()
            .zip(self.glyphs.last())
            .map(|(a, b)| &self.text[a.text_range.start.min(b.text_range.start)..])
            .unwrap_or_default()
            .chars()
            .take(40)
            .collect();
        f.debug_struct("TextRun")
            .field("font_size", &self.font_size)
            .field("origin", &self.origin)
            .field("glyphs", &self.glyphs.len())
            .field("text", &text)
            .finish()
    }
}

/// One glyph. Lengths are in points; `y_offset` points down.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    /// Byte range in [`TextRun::text`] of the cluster this glyph belongs to.
    pub text_range: Range<usize>,
}
