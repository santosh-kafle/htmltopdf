//! Text in the page margins, as wkhtmltopdf's `--header-*` and `--footer-*`
//! options draw it.
//!
//! wkhtmltopdf draws a header as one line of text just above the content
//! area, with up to three parts (left, center, right) over the content
//! width, and a footer the same way just below it. `--header-spacing` moves
//! the text away from the content, and `--header-line` rules a line between
//! the two. The text does not wrap, and the page margins are not enlarged
//! to make room: callers set `--margin-top` to fit their header.

use std::collections::HashMap;
use std::sync::Arc;

use presswork_dom::Document;
use presswork_dom::resource::Fetcher;
use presswork_style::{
    Color, ComputedStyle, FontFamilyName, GenericFamily, Stylist, TextAlign, WhiteSpace,
};

use crate::Engine;
use crate::flow;
use crate::fragment::{Fill, Item, Page};
use crate::geometry::{EdgeSizes, Rect};

/// Which margin a line of text goes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// Above the content area.
    Header,
    /// Below the content area.
    Footer,
}

/// How a header or footer is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct BandStyle {
    /// Tried first; the default sans-serif font fills in for it and for
    /// characters it lacks.
    pub font_family: String,
    /// In points.
    pub font_size: f32,
    /// Whether to rule a line between the text and the content area.
    pub line: bool,
    /// Distance between the content area and the text (and the line), in
    /// points. Negative values move the text into the content area.
    pub spacing: f32,
}

/// What a header or footer says on one page. Each part may have several
/// lines; empty parts are not drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BandText<'a> {
    pub left: &'a str,
    pub center: &'a str,
    pub right: &'a str,
}

/// Thickness of the header and footer lines. wkhtmltopdf draws a hairline,
/// which PDF viewers show as thin as the screen allows.
const RULE: f32 = 0.5;

impl Engine {
    /// Draws a header or footer on `page`, whose content area is inside
    /// `margins`. Draws over whatever is already there.
    ///
    /// Header text hangs above the content area: its first line ends
    /// `style.spacing` above it, and further lines (from line breaks in
    /// the text) run down toward the content. Footer text stands below
    /// the content area the same way: its last line starts `spacing`
    /// below it. This is where wkhtmltopdf's Qt `drawText` puts them.
    pub fn draw_band(
        &mut self,
        page: &mut Page,
        band: Band,
        style: &BandStyle,
        text: BandText<'_>,
        margins: EdgeSizes,
    ) {
        let width = (page.size.width - margins.left - margins.right).max(0.0);
        let edge = match band {
            Band::Header => margins.top - style.spacing,
            Band::Footer => page.size.height - margins.bottom + style.spacing,
        };
        if style.line {
            page.items.push(Item::Fill(Fill {
                rect: Rect::new(margins.left, edge - RULE / 2.0, width, RULE),
                color: Color::BLACK,
            }));
        }
        let parts = [
            (text.left, TextAlign::Left),
            (text.center, TextAlign::Center),
            (text.right, TextAlign::Right),
        ];
        if parts.iter().all(|(text, _)| text.is_empty()) {
            return;
        }

        let doc = Document::new();
        let styles = Stylist::new();
        let fetcher = Fetcher::data_only();
        let mut cx = flow::Context {
            fonts: &mut self.fonts,
            generics: &self.generics,
            texts: &mut self.texts,
            doc: &doc,
            styles: &styles,
            fetcher: &fetcher,
            web_fonts: flow::WebFonts::new(&[], 0),
            load_images: false,
            images: HashMap::new(),
        };
        let mut css = ComputedStyle::initial();
        css.font_family = Arc::new([
            FontFamilyName::Named(style.font_family.clone()),
            FontFamilyName::Generic(GenericFamily::SansSerif),
        ]);
        css.font_size = style.font_size;
        // Qt draws the text as given: spaces kept, lines broken only at
        // line breaks.
        css.white_space = WhiteSpace::Pre;

        for (text, align) in parts {
            if text.is_empty() {
                continue;
            }
            css.text_align = align;
            let canvas = flow::text_block(&mut cx, text, &css, width);
            let (Some(first), Some(last)) = (canvas.slices.first(), canvas.slices.last()) else {
                continue;
            };
            let line = first.height;
            let height = (last.top - first.top) as f32 + last.height;
            let top = match band {
                Band::Header => edge - line,
                Band::Footer => edge + line - height,
            } - first.top as f32;
            canvas.into_items(margins.left, top, &mut page.items);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Size;

    const SIZE: Size = Size {
        width: 200.0,
        height: 300.0,
    };
    const MARGINS: EdgeSizes = EdgeSizes::uniform(30.0);

    fn style(line: bool, spacing: f32) -> BandStyle {
        BandStyle {
            font_family: "Arial".into(),
            font_size: 12.0,
            line,
            spacing,
        }
    }

    fn blank() -> Page {
        Page {
            size: SIZE,
            items: Vec::new(),
        }
    }

    /// Text runs as (text, x, baseline).
    fn runs(page: &Page) -> Vec<(String, f32, f32)> {
        page.items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) => {
                    let start = run.glyphs.first()?.text_range.start;
                    let end = run.glyphs.iter().map(|g| g.text_range.end).max()?;
                    Some((run.text[start..end].to_owned(), run.origin.x, run.origin.y))
                }
                Item::Fill(_) | Item::Image(_) => None,
            })
            .collect()
    }

    fn fills(page: &Page) -> Vec<Rect> {
        page.items
            .iter()
            .filter_map(|item| match item {
                Item::Fill(fill) => Some(fill.rect),
                Item::Text(_) | Item::Image(_) => None,
            })
            .collect()
    }

    #[test]
    fn header_sits_above_the_content_in_three_parts() {
        let mut engine = Engine::new();
        let mut page = blank();
        let text = BandText {
            left: "Invoice",
            center: "ACME",
            right: "1/3",
        };
        engine.draw_band(&mut page, Band::Header, &style(false, 0.0), text, MARGINS);
        let runs = runs(&page);
        assert_eq!(runs.len(), 3, "{runs:?}");
        let [(left, lx, ly), (center, cx, cy), (right, rx, ry)] = &runs[..] else {
            unreachable!()
        };
        assert_eq!((&**left, &**center, &**right), ("Invoice", "ACME", "1/3"));
        // One baseline, inside the top margin, close to the content.
        assert!(ly == cy && cy == ry, "{runs:?}");
        assert!(*ly > 30.0 - 18.0 && *ly < 30.0, "baseline {ly}");
        // Left at the content's left edge, the others centered and flush
        // right within the content width.
        assert_eq!(*lx, 30.0);
        assert!(*cx > 80.0 && *cx < 100.0, "center at {cx}");
        assert!(*rx > 140.0 && *rx < 170.0, "right at {rx}");
    }

    #[test]
    fn footer_sits_below_the_content() {
        let mut engine = Engine::new();
        let mut page = blank();
        let text = BandText {
            center: "Page 2",
            ..BandText::default()
        };
        engine.draw_band(&mut page, Band::Footer, &style(false, 0.0), text, MARGINS);
        let [(_, _, baseline)] = runs(&page)[..] else {
            panic!("one run expected")
        };
        // The text's line starts at the content's bottom edge; fonts'
        // ascent is often a little more than their size.
        assert!(
            baseline > 270.0 + 6.0 && baseline < 270.0 + 18.0,
            "{baseline}"
        );
    }

    #[test]
    fn spacing_moves_text_and_line_away_from_the_content() {
        let mut engine = Engine::new();
        let text = BandText {
            left: "x",
            ..BandText::default()
        };
        let mut close = blank();
        engine.draw_band(&mut close, Band::Header, &style(true, 0.0), text, MARGINS);
        let mut spaced = blank();
        engine.draw_band(&mut spaced, Band::Header, &style(true, 5.0), text, MARGINS);
        assert_eq!(runs(&close)[0].2 - runs(&spaced)[0].2, 5.0);
        assert_eq!(
            fills(&close),
            [Rect::new(30.0, 30.0 - RULE / 2.0, 140.0, RULE)]
        );
        assert_eq!(fills(&spaced)[0].origin.y, 25.0 - RULE / 2.0);

        let mut footer = blank();
        engine.draw_band(&mut footer, Band::Footer, &style(true, 5.0), text, MARGINS);
        assert_eq!(fills(&footer)[0].origin.y, 275.0 - RULE / 2.0);
    }

    #[test]
    fn line_alone_draws_without_text() {
        let mut engine = Engine::new();
        let mut page = blank();
        let empty = BandText::default();
        engine.draw_band(&mut page, Band::Footer, &style(true, 0.0), empty, MARGINS);
        assert_eq!(fills(&page).len(), 1);
        assert!(runs(&page).is_empty());

        let mut page = blank();
        engine.draw_band(&mut page, Band::Footer, &style(false, 0.0), empty, MARGINS);
        assert!(page.items.is_empty());
    }

    #[test]
    fn devanagari_falls_back_to_a_font_that_has_it() {
        let mut engine = Engine::new();
        let mut page = blank();
        let text = BandText {
            right: "पृष्ठ २",
            ..BandText::default()
        };
        engine.draw_band(&mut page, Band::Footer, &style(false, 0.0), text, MARGINS);
        let glyphs: Vec<u32> = page
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) => Some(run.glyphs.iter().map(|g| g.id)),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(!glyphs.is_empty() && !glyphs.contains(&0), "{glyphs:?}");
    }
}
