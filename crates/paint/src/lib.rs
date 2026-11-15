//! PDF output.
//!
//! Draws the pages produced by `presswork-layout` with `krilla`, which embeds
//! subsetted fonts and maps glyphs back to text so the PDF can be searched
//! and copied from.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use krilla::Document;
use krilla::geom::{PathBuilder, Point, Rect};
use krilla::metadata::Metadata;
use krilla::page::PageSettings;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use presswork_layout::{FontData, Item, Page, TextRun};

/// Document-level options for the generated PDF.
#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    /// `Title` entry in the PDF metadata.
    pub title: Option<String>,
    /// `Author` entry in the PDF metadata.
    pub author: Option<String>,
}

/// Writing the PDF failed.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to write PDF: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// Renders laid-out pages to PDF bytes.
pub fn render_pdf(pages: &[Page], options: &PdfOptions) -> Result<Vec<u8>, Error> {
    let mut document = Document::new();
    let mut fonts = FontCache::default();

    for page in pages {
        let settings = PageSettings::from_wh(page.size.width, page.size.height)
            .ok_or_else(|| Error(format!("invalid page size {:?}", page.size)))?;
        let mut pdf_page = document.start_page_with(settings);
        let mut surface = pdf_page.surface();
        for item in &page.items {
            match item {
                Item::Text(run) => {
                    if let Some(font) = fonts.get(&run.font) {
                        draw_text(&mut surface, run, font);
                    }
                }
                Item::Rule(rect) => {
                    let mut path = PathBuilder::new();
                    if let Some(rect) = Rect::from_xywh(
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                    ) {
                        path.push_rect(rect);
                    }
                    if let Some(path) = path.finish() {
                        surface.draw_path(&path);
                    }
                }
            }
        }
        surface.finish();
        pdf_page.finish();
    }

    let mut metadata = Metadata::new().creator(format!("presswork {}", env!("CARGO_PKG_VERSION")));
    if let Some(title) = &options.title {
        metadata = metadata.title(title.clone());
    }
    if let Some(author) = &options.author {
        metadata = metadata.authors(vec![author.clone()]);
    }
    document.set_metadata(metadata);
    document.finish().map_err(|e| Error(e.to_string()))
}

fn draw_text(surface: &mut krilla::surface::Surface<'_>, run: &TextRun, font: Font) {
    let size = run.font_size;
    if size <= 0.0 {
        return;
    }
    // krilla wants advances and offsets as fractions of the font size, with
    // y pointing up.
    let glyphs: Vec<KrillaGlyph> = run
        .glyphs
        .iter()
        .map(|g| {
            KrillaGlyph::new(
                GlyphId::new(g.id),
                g.advance / size,
                g.x_offset / size,
                -g.y_offset / size,
                0.0,
                g.text_range.clone(),
                None,
            )
        })
        .collect();
    surface.draw_glyphs(
        Point::from_xy(run.origin.x, run.origin.y),
        &glyphs,
        font,
        &run.text,
        size,
        false,
    );
}

/// Parsed fonts, keyed by the font blob and collection index.
#[derive(Default)]
struct FontCache {
    fonts: HashMap<(u64, u32), Option<Font>>,
}

impl FontCache {
    fn get(&mut self, data: &FontData) -> Option<Font> {
        self.fonts
            .entry((data.data.id(), data.index))
            .or_insert_with(|| {
                let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(data.data.clone());
                Font::new(bytes.into(), data.index)
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presswork_layout::{Engine, LayoutOptions};

    #[test]
    fn writes_a_pdf_with_text() {
        let doc = presswork_dom::parse_html(b"<p>Hello</p>");
        let pages = Engine::new().layout(&doc, &LayoutOptions::default());
        let pdf = render_pdf(
            &pages,
            &PdfOptions {
                title: Some("Test".into()),
                author: None,
            },
        )
        .unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.windows(4).any(|w| w == b"Font"), "font is embedded");
    }
}
