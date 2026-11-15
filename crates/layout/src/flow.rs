//! Block, inline and table layout on one continuous canvas.
//!
//! Content is laid out top to bottom as if the page were infinitely tall,
//! producing [`Slice`]s: unbreakable horizontal bands (a line of text, a
//! table row) in continuous coordinates. [`crate::paginate`] then cuts the
//! canvas into pages between slices.

use std::ops::Range;
use std::sync::Arc;

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily as FamilyStack, FontStyle, FontWeight,
    GenericFamily, GlyphRun, Language, LayoutContext, PositionedLayoutItem, StyleProperty,
    TextWrapMode,
};
use presswork_dom::{Document, NodeId, NodeKind};
use presswork_style::{ComputedStyle, Display, FontFamily, TextAlign};

use crate::fragment::{Glyph, Item, TextRun};
use crate::geometry::{Point, Rect};

/// An unbreakable band of content.
#[derive(Debug)]
pub(crate) struct Slice {
    pub top: f32,
    pub bottom: f32,
    pub items: Vec<Item>,
}

/// Space between table cells and around them (`border-spacing: 2px`).
const CELL_SPACING: f32 = 1.5;
/// Thickness of an `<hr>`.
const RULE_THICKNESS: f32 = 0.75;

pub(crate) struct Flow<'a> {
    fonts: &'a mut FontContext,
    generics: &'a GenericFonts,
    texts: &'a mut LayoutContext<()>,
    doc: &'a Document,
    pub slices: Vec<Slice>,
    /// Top of the next content, before any pending margin.
    y: f32,
    /// Collapsed margin waiting to be added before the next content.
    margin: f32,
}

/// Inline content gathered for one paragraph.
#[derive(Default)]
struct Inline {
    text: String,
    spans: Vec<(Range<usize>, ComputedStyle)>,
}

impl Inline {
    fn push_text(&mut self, text: &str, style: &ComputedStyle) {
        let start = self.text.len();
        if style.preformatted {
            self.text.push_str(&text.replace("\r\n", "\n"));
        } else {
            for ch in text.chars() {
                if ch.is_ascii_whitespace() {
                    let after_space = self.text.is_empty()
                        || self.text.ends_with(' ')
                        || self.text.ends_with('\n');
                    if !after_space {
                        self.text.push(' ');
                    }
                } else {
                    self.text.push(ch);
                }
            }
        }
        if self.text.len() > start {
            self.spans.push((start..self.text.len(), style.clone()));
        }
    }

    fn push_break(&mut self, style: &ComputedStyle) {
        let start = self.text.len();
        if self.text.ends_with(' ') {
            self.text.pop();
        }
        self.text.push('\n');
        self.spans.push((
            start.min(self.text.len() - 1)..self.text.len(),
            style.clone(),
        ));
    }

    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// The font each generic family resolves to.
///
/// fontconfig maps a generic family such as `serif` to a long list of fonts
/// covering many scripts, in an order that is not script-aware: Noto Serif
/// Bengali can come before Noto Serif Devanagari, and then wins characters
/// both contain, such as the danda (।). Browsers instead map each generic
/// family to one font and pick fallback fonts by script, so do the same.
#[derive(Debug, Default)]
pub(crate) struct GenericFonts {
    serif: Option<String>,
    sans_serif: Option<String>,
    monospace: Option<String>,
}

impl GenericFonts {
    pub fn resolve(fonts: &mut FontContext) -> Self {
        let mut first = |generic| {
            let id = fonts.collection.generic_families(generic).next()?;
            fonts.collection.family_name(id).map(str::to_owned)
        };
        Self {
            serif: first(GenericFamily::Serif),
            sans_serif: first(GenericFamily::SansSerif),
            monospace: first(GenericFamily::Monospace),
        }
    }

    fn stack(&self, family: FontFamily) -> FamilyStack<'_> {
        let (name, generic) = match family {
            FontFamily::Serif => (&self.serif, GenericFamily::Serif),
            FontFamily::SansSerif => (&self.sans_serif, GenericFamily::SansSerif),
            FontFamily::Monospace => (&self.monospace, GenericFamily::Monospace),
        };
        match name {
            Some(name) => FamilyStack::named(name),
            None => generic.into(),
        }
    }
}

fn push_font_properties<'s>(
    style: &ComputedStyle,
    generics: &'s GenericFonts,
    mut push: impl FnMut(StyleProperty<'s, ()>),
) {
    push(StyleProperty::FontFamily(generics.stack(style.font_family)));
    push(StyleProperty::FontSize(style.font_size));
    push(StyleProperty::FontWeight(FontWeight::new(
        style.font_weight,
    )));
    push(StyleProperty::FontStyle(if style.italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    }));
    let language = style.lang.as_deref().and_then(|l| Language::parse(l).ok());
    push(StyleProperty::Locale(language));
}

impl<'a> Flow<'a> {
    pub fn new(
        fonts: &'a mut FontContext,
        generics: &'a GenericFonts,
        texts: &'a mut LayoutContext<()>,
        doc: &'a Document,
    ) -> Self {
        Self {
            fonts,
            generics,
            texts,
            doc,
            slices: Vec::new(),
            y: 0.0,
            margin: 0.0,
        }
    }

    /// A flow for a nested formatting context (a table cell) starting at `y`.
    fn nested(&mut self, y: f32) -> Flow<'_> {
        Flow {
            fonts: self.fonts,
            generics: self.generics,
            texts: self.texts,
            doc: self.doc,
            slices: Vec::new(),
            y,
            margin: 0.0,
        }
    }

    fn add_margin(&mut self, margin: f32) {
        // Adjacent vertical margins collapse to the largest one.
        self.margin = self.margin.max(margin);
    }

    fn settle_margin(&mut self) {
        self.y += self.margin;
        self.margin = 0.0;
    }

    /// Lays out the block-level element `id` with the given style, in a
    /// containing block starting at `x` that is `width` wide.
    pub fn block(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        let m = style.margin;
        let p = style.padding;
        let x = x + m.left + p.left;
        let width = (width - m.left - m.right - p.left - p.right).max(0.0);

        self.add_margin(m.top);
        if p.top > 0.0 {
            self.settle_margin();
            self.y += p.top;
        }

        let is_rule = self.doc.element(id).is_some_and(|e| e.name == "hr");
        if is_rule {
            self.settle_margin();
            let rule = Rect::new(x, self.y, width, RULE_THICKNESS);
            self.slices.push(Slice {
                top: self.y,
                bottom: self.y + RULE_THICKNESS,
                items: vec![Item::Rule(rule)],
            });
            self.y += RULE_THICKNESS;
        } else if style.display == Display::Table {
            self.table(id, style, x, width);
        } else {
            self.block_contents(id, style, x, width);
        }

        if p.bottom > 0.0 {
            self.settle_margin();
            self.y += p.bottom;
        }
        self.add_margin(m.bottom);
    }

    fn block_contents(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        let mut inline = Inline::default();
        if style.display == Display::ListItem
            && let Some(marker) = self.list_marker(id)
        {
            inline.push_text(
                &marker,
                &ComputedStyle {
                    preformatted: true,
                    ..style.clone()
                },
            );
        }
        self.children(id, style, x, width, &mut inline);
        self.paragraph(inline, style, x, width);
    }

    /// `• ` for unordered lists, `3. ` for ordered ones.
    fn list_marker(&self, item: NodeId) -> Option<String> {
        let parent = self.doc.node(item).parent?;
        let list = self.doc.element(parent)?;
        if list.name != "ol" {
            return Some("• ".into());
        }
        let start: i64 = list
            .attr("start")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1);
        let position = self
            .doc
            .children(parent)
            .filter(|&c| self.doc.element(c).is_some_and(|e| e.name == "li"))
            .position(|c| c == item)?;
        Some(format!("{}. ", start + position as i64))
    }

    /// Walks the children of `id`, gathering inline content into `inline`
    /// and laying out block-level children as they come.
    fn children(
        &mut self,
        id: NodeId,
        style: &ComputedStyle,
        x: f32,
        width: f32,
        inline: &mut Inline,
    ) {
        let doc = self.doc;
        for child in doc.children(id) {
            match &doc.node(child).kind {
                NodeKind::Text(text) => inline.push_text(text, style),
                NodeKind::Element(element) => {
                    let child_style = ComputedStyle::for_element(style, element);
                    match child_style.display {
                        Display::None => {}
                        Display::Inline if element.name == "br" => inline.push_break(&child_style),
                        Display::Inline => self.children(child, &child_style, x, width, inline),
                        _ => {
                            self.paragraph(std::mem::take(inline), style, x, width);
                            self.block(child, &child_style, x, width);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Shapes and line-breaks a paragraph, adding one slice per line.
    fn paragraph(&mut self, inline: Inline, style: &ComputedStyle, x: f32, width: f32) {
        if inline.is_blank() {
            return;
        }
        self.settle_margin();

        let text: Arc<str> = inline.text.into();
        let mut builder = self.texts.ranged_builder(self.fonts, &text, 1.0, false);
        let generics = self.generics;
        push_font_properties(style, generics, |p| builder.push_default(p));
        if style.preformatted {
            builder.push_default(StyleProperty::TextWrapMode(TextWrapMode::NoWrap));
        }
        for (range, span) in &inline.spans {
            push_font_properties(span, generics, |p| builder.push(p, range.clone()));
        }
        let mut layout = builder.build(&text);
        let alignment = match style.text_align {
            TextAlign::Left => Alignment::Left,
            TextAlign::Right => Alignment::Right,
            TextAlign::Center => Alignment::Center,
        };
        layout.set_alignment(alignment, AlignmentOptions::default());
        layout.break_all_lines(Some(width));

        for line in layout.lines() {
            let metrics = line.metrics();
            let mut items = Vec::new();
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    items.push(Item::Text(text_run(&run, x, self.y, &text)));
                }
            }
            self.slices.push(Slice {
                top: self.y + metrics.block_min_coord,
                bottom: self.y + metrics.block_max_coord,
                items,
            });
        }
        self.y += layout.height();
    }

    /// A simple table layout: equal-width columns (honoring `colspan`),
    /// cells aligned to the top, each row kept on one page.
    fn table(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        let doc = self.doc;
        let mut rows = Vec::new();
        for child in doc.children(id) {
            let Some(element) = doc.element(child) else {
                continue;
            };
            let child_style = ComputedStyle::for_element(style, element);
            match child_style.display {
                Display::TableCaption => self.block(child, &child_style, x, width),
                Display::TableRow => rows.push((child, child_style)),
                Display::TableRowGroup => {
                    for row in doc.children(child) {
                        if let Some(row_element) = doc.element(row) {
                            let row_style = ComputedStyle::for_element(&child_style, row_element);
                            if row_style.display == Display::TableRow {
                                rows.push((row, row_style));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let rows: Vec<Vec<(NodeId, ComputedStyle, usize)>> = rows
            .iter()
            .map(|(row, row_style)| {
                doc.children(*row)
                    .filter_map(|cell| {
                        let element = doc.element(cell)?;
                        let cell_style = ComputedStyle::for_element(row_style, element);
                        (cell_style.display == Display::TableCell).then(|| {
                            let span = element
                                .attr("colspan")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                                .unwrap_or(1)
                                .clamp(1, 1000);
                            (cell, cell_style, span)
                        })
                    })
                    .collect()
            })
            .collect();
        let columns = rows
            .iter()
            .map(|cells| cells.iter().map(|(_, _, span)| span).sum::<usize>())
            .max()
            .unwrap_or(0)
            .max(1);
        let column_width =
            ((width - CELL_SPACING * (columns as f32 + 1.0)) / columns as f32).max(0.0);

        self.settle_margin();
        self.y += CELL_SPACING;
        for cells in rows {
            let top = self.y;
            let mut bottom = top;
            let mut items = Vec::new();
            let mut column = 0;
            for (cell, cell_style, span) in cells {
                let span = span.min(columns - column.min(columns)).max(1);
                let cell_x = x + CELL_SPACING + column as f32 * (column_width + CELL_SPACING);
                let cell_width = column_width * span as f32 + CELL_SPACING * (span as f32 - 1.0);
                let p = cell_style.padding;

                let mut nested = self.nested(top + p.top);
                nested.block_contents(
                    cell,
                    &cell_style,
                    cell_x + p.left,
                    (cell_width - p.left - p.right).max(0.0),
                );
                nested.settle_margin();
                bottom = bottom.max(nested.y + p.bottom);
                items.extend(nested.slices.into_iter().flat_map(|s| s.items));
                column += span;
            }
            self.slices.push(Slice { top, bottom, items });
            self.y = bottom + CELL_SPACING;
        }
    }
}

/// Converts a parley glyph run into a [`TextRun`] at the given offset.
fn text_run(run: &GlyphRun<'_, ()>, x: f32, y: f32, text: &Arc<str>) -> TextRun {
    let shaped = run.run();

    // parley's glyph runs do not expose which characters each glyph came
    // from, so map glyphs to clusters through the run's clusters.
    let mut glyphs: Vec<Glyph> = shaped
        .visual_clusters()
        .filter(|cluster| cluster.style_index() == run.style_index())
        .flat_map(|cluster| {
            let range = cluster.text_range();
            cluster.glyphs().map(move |g| Glyph {
                id: g.id,
                advance: g.advance,
                x_offset: g.x,
                y_offset: g.y,
                text_range: range.clone(),
            })
        })
        .collect();

    let expected: Vec<_> = run.glyphs().collect();
    let matches =
        glyphs.len() == expected.len() && glyphs.iter().zip(&expected).all(|(a, b)| a.id == b.id);
    if !matches {
        // Fall back to attributing the whole run's text to every glyph,
        // which keeps the output correct but makes text selection coarse.
        let range = shaped.text_range();
        glyphs = expected
            .into_iter()
            .map(|g| Glyph {
                id: g.id,
                advance: g.advance,
                x_offset: g.x,
                y_offset: g.y,
                text_range: range.clone(),
            })
            .collect();
    }

    TextRun {
        font: shaped.font().clone(),
        font_size: shaped.font_size(),
        origin: Point {
            x: x + run.offset(),
            y: y + run.baseline(),
        },
        glyphs,
        text: Arc::clone(text),
    }
}
