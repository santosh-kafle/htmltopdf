//! Box tree construction, layout and pagination.
//!
//! Pipeline stages owned by this crate:
//!
//! 1. **Layout**: block, inline (text) and table formatting on one
//!    continuous canvas. Text shaping, line breaking and font fallback come
//!    from `parley`.
//! 2. **Fragmentation**: cut the canvas into pages between lines and table
//!    rows.
//!
//! The output is a list of positioned items per page, consumed by
//! `presswork-paint`. All geometry is expressed in typographic points
//! (1pt = 1/72 in).

mod flow;
pub mod fragment;
pub mod geometry;
pub mod page;
mod paginate;

use std::fmt;

use parley::{FontContext, LayoutContext};
use presswork_dom::Document;
use presswork_style::ComputedStyle;

pub use fragment::{FontData, Glyph, Item, Page, TextRun};
pub use geometry::{EdgeSizes, Point, Rect, Size};
pub use page::{Orientation, PageSize};

/// Page geometry for a layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutOptions {
    pub page_size: Size,
    pub margins: EdgeSizes,
}

impl Default for LayoutOptions {
    /// A4 portrait with wkhtmltopdf's default 10mm margins.
    fn default() -> Self {
        Self {
            page_size: PageSize::A4.size(Orientation::Portrait),
            margins: EdgeSizes::uniform(10.0 * 72.0 / 25.4),
        }
    }
}

/// Lays out documents. Holds the font database, which is expensive to
/// build, so reuse one engine for many documents.
pub struct Engine {
    fonts: FontContext,
    generics: flow::GenericFonts,
    texts: LayoutContext<()>,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// Creates an engine that uses the system's fonts.
    pub fn new() -> Self {
        let mut fonts = FontContext::new();
        let generics = flow::GenericFonts::resolve(&mut fonts);
        Self {
            fonts,
            generics,
            texts: LayoutContext::new(),
        }
    }

    /// Lays out `doc` into pages.
    pub fn layout(&mut self, doc: &Document, options: &LayoutOptions) -> Vec<Page> {
        let margins = options.margins;
        let width = (options.page_size.width - margins.left - margins.right).max(0.0);

        let mut flow = flow::Flow::new(&mut self.fonts, &self.generics, &mut self.texts, doc);
        if let Some(root) = doc.document_element()
            && let Some(element) = doc.element(root)
        {
            let style = ComputedStyle::for_element(&ComputedStyle::initial(), element);
            flow.block(root, &style, 0.0, width);
        }
        paginate::paginate(flow.slices, options.page_size, margins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presswork_dom::parse_html;

    fn layout(html: &str) -> Vec<Page> {
        Engine::new().layout(&parse_html(html.as_bytes()), &LayoutOptions::default())
    }

    fn texts(page: &Page) -> Vec<(String, f32)> {
        page.items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) => {
                    let start = run.glyphs.iter().map(|g| g.text_range.start).min()?;
                    let end = run.glyphs.iter().map(|g| g.text_range.end).max()?;
                    Some((run.text[start..end].to_owned(), run.origin.y))
                }
                Item::Rule(_) => None,
            })
            .collect()
    }

    #[test]
    fn lays_out_paragraphs_top_to_bottom() {
        let pages = layout("<h1>Invoice</h1><p>Hello <b>world</b></p>");
        assert_eq!(pages.len(), 1);
        let runs = texts(&pages[0]);
        let words: Vec<&str> = runs.iter().map(|(t, _)| t.trim()).collect();
        assert_eq!(words, ["Invoice", "Hello", "world"]);
        assert!(runs[0].1 < runs[1].1, "heading above paragraph");
        assert_eq!(runs[1].1, runs[2].1, "same line, same baseline");
    }

    #[test]
    fn hides_head_and_scripts() {
        let pages = layout("<title>T</title><script>x()</script><p>Body</p>");
        let words: Vec<String> = texts(&pages[0]).into_iter().map(|(t, _)| t).collect();
        assert_eq!(words, ["Body"]);
    }

    #[test]
    fn long_documents_paginate() {
        let html = "<p>line</p>".repeat(200);
        let pages = layout(&html);
        assert!(pages.len() > 1, "{} pages", pages.len());
        let bottom_limit = pages[0].size.height;
        for page in &pages {
            for (_, y) in texts(page) {
                assert!(y > 0.0 && y < bottom_limit);
            }
        }
    }

    /// Golden test: a bilingual Nepali/English invoice. Every glyph must
    /// come from a font that has it: glyph 0 is `.notdef`, the "tofu" box
    /// drawn when no font covers a character.
    ///
    /// Needs a Devanagari font on the system (e.g. `fonts-noto-core`).
    #[test]
    fn nepali_invoice_has_no_missing_glyphs() {
        let html = include_str!("../../../tests/fixtures/invoice-ne.html");
        let pages = layout(html);
        assert_eq!(pages.len(), 1);
        let mut glyphs = 0;
        for item in &pages[0].items {
            if let Item::Text(run) = item {
                for glyph in &run.glyphs {
                    glyphs += 1;
                    assert_ne!(
                        glyph.id,
                        0,
                        "missing glyph for {:?}",
                        &run.text[glyph.text_range.clone()]
                    );
                }
            }
        }
        assert!(glyphs > 500, "only {glyphs} glyphs");
    }

    #[test]
    fn glyphs_map_back_to_their_own_characters() {
        let pages = layout("<p>Hello</p>");
        let Item::Text(run) = &pages[0].items[0] else {
            panic!("expected text");
        };
        let chars: Vec<&str> = run
            .glyphs
            .iter()
            .map(|g| &run.text[g.text_range.clone()])
            .collect();
        assert_eq!(chars, ["H", "e", "l", "l", "o"]);
    }

    #[test]
    fn table_cells_share_a_row() {
        let pages = layout("<table><tr><td>Item</td><td align=right>Qty</td></tr></table>");
        let runs = texts(&pages[0]);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].1, runs[1].1);
    }
}
