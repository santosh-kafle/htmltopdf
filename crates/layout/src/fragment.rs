//! The output of layout: positioned content, one list per page.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

pub use parley::FontData;
pub use presswork_style::Color;

use crate::geometry::{Point, Rect, Size};

/// One laid-out page. Coordinates are in points from the page's top-left.
/// Items are in paint order: later items draw over earlier ones.
#[derive(Debug, Clone)]
pub struct Page {
    pub size: Size,
    pub items: Vec<Item>,
}

/// A piece of positioned content.
#[derive(Debug, Clone)]
pub enum Item {
    Text(TextRun),
    Fill(Fill),
}

impl Item {
    pub(crate) fn translate(mut self, dx: f32, dy: f32) -> Self {
        let origin = match &mut self {
            Item::Text(run) => &mut run.origin,
            Item::Fill(fill) => &mut fill.rect.origin,
        };
        origin.x += dx;
        origin.y += dy;
        self
    }
}

/// A filled rectangle: a background or one edge of a border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    pub rect: Rect,
    pub color: Color,
}

/// Shaped glyphs in a single font, size and color, drawn left to right
/// from `origin`, which is on the baseline.
#[derive(Clone)]
pub struct TextRun {
    pub font: FontData,
    /// Font size in points.
    pub font_size: f32,
    pub color: Color,
    pub origin: Point,
    pub glyphs: Vec<Glyph>,
    /// The text the glyphs' `text_range`s index into, so the PDF can map
    /// glyphs back to characters for copying and search.
    pub text: Arc<str>,
}

impl TextRun {
    /// The source text this run covers.
    pub fn covered_text(&self) -> &str {
        let start = self.glyphs.iter().map(|g| g.text_range.start).min();
        let end = self.glyphs.iter().map(|g| g.text_range.end).max();
        match (start, end) {
            (Some(start), Some(end)) => &self.text[start..end],
            _ => "",
        }
    }
}

impl fmt::Debug for TextRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: String = self.covered_text().chars().take(40).collect();
        f.debug_struct("TextRun")
            .field("font_size", &self.font_size)
            .field("color", &self.color)
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
