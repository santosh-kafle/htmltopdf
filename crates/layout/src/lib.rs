//! Layout and pagination.
//!
//! Pipeline stages owned by this crate:
//!
//! 1. **Layout** ([`flow`]): block, inline (text) and table formatting on
//!    one continuous canvas, using the computed styles from
//!    `presswork-style`. Text shaping, line breaking and font fallback come
//!    from `parley`.
//! 2. **Fragmentation** ([`paginate`]): cut the canvas into pages between
//!    lines and table rows.
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
use presswork_style::Stylist;

pub use fragment::{Color, Fill, FontData, Glyph, Item, Page, TextRun};
pub use geometry::{EdgeSizes, Point, Rect, Size};
pub use page::{Orientation, PageSize};

/// Page geometry for a layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutOptions {
    pub page_size: Size,
    pub margins: EdgeSizes,
}

/// wkhtmltopdf's default page margin: 10mm.
pub const DEFAULT_MARGIN: f32 = 10.0 * 72.0 / 25.4;

impl Default for LayoutOptions {
    /// A4 portrait with wkhtmltopdf's default 10mm margins.
    fn default() -> Self {
        Self {
            page_size: PageSize::A4.size(Orientation::Portrait),
            margins: EdgeSizes::uniform(DEFAULT_MARGIN),
        }
    }
}

/// Lays out documents. Holds the font database, which is expensive to
/// build, so reuse one engine for many documents.
pub struct Engine {
    fonts: FontContext,
    generics: flow::GenericFonts,
    texts: LayoutContext<flow::Ink>,
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

    /// Whether the system provided any fonts. Without them, text is laid
    /// out but no glyphs can be drawn.
    ///
    /// On Linux this is false when fontconfig is not installed: it is
    /// loaded at runtime, not linked.
    pub fn has_system_fonts(&self) -> bool {
        self.generics.any()
    }

    /// Lays out `doc`, styled by `styles`, into pages.
    pub fn layout(
        &mut self,
        doc: &Document,
        styles: &Stylist,
        options: &LayoutOptions,
    ) -> Vec<Page> {
        let margins = options.margins;
        let width = (options.page_size.width - margins.left - margins.right).max(0.0);

        let mut context = flow::Context {
            fonts: &mut self.fonts,
            generics: &self.generics,
            texts: &mut self.texts,
            doc,
            styles,
        };
        let mut flow = flow::Flow::new(&mut context);
        if let Some(root) = doc.document_element() {
            let style = styles.style(doc, root, None);
            flow.block(root, &style, 0.0, width);
        }
        let (canvas, _height) = flow.finish();
        paginate::paginate(canvas, options.page_size, margins)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod engine_tests {
    use super::Engine;

    /// The test machines have fonts; if this fails, every text test would
    /// fail in confusing ways, so fail here first with a clear message.
    #[test]
    fn system_fonts_are_available() {
        assert!(
            Engine::new().has_system_fonts(),
            "no system fonts found: install fontconfig and a font package"
        );
    }
}
