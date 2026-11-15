//! Inline layout: gathering text, shaping it and breaking it into lines.

use std::ops::Range;
use std::sync::Arc;

use parley::{
    Alignment, AlignmentOptions, FontStyle, FontWeight, GlyphRun, Language, Layout,
    LineHeight as ParleyLineHeight, PositionedLayoutItem, StyleProperty, TextWrapMode,
};
use presswork_dom::{Document, NodeId};
use presswork_style::{Color, ComputedStyle, LineHeight, ListStyleType, TextAlign, WhiteSpace};

use super::{Context, Flow, GenericFonts};
use crate::fragment::{Glyph, Item, TextRun};
use crate::geometry::Point;

/// The text color, as parley's per-span "brush".
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ink(pub Color);

impl Default for Ink {
    fn default() -> Self {
        Ink(Color::BLACK)
    }
}

/// Inline content gathered for one paragraph: the text after white-space
/// processing, and the style of each stretch of it.
#[derive(Default)]
pub(super) struct Inline {
    text: String,
    spans: Vec<(Range<usize>, ComputedStyle)>,
}

/// Tab stops in preformatted text, in characters.
const TAB_SIZE: usize = 8;

impl Inline {
    /// Appends text, applying the `white-space` rules of `style`.
    pub fn push_text(&mut self, text: &str, style: &ComputedStyle) {
        let white_space = style.white_space;
        let start = self.text.len();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\r' if chars.peek() == Some(&'\n') => {}
                '\n' | '\r' if white_space.preserves_newlines() => self.push_newline(),
                '\t' if !white_space.collapses_spaces() => {
                    let column = self
                        .text
                        .rsplit('\n')
                        .next()
                        .map_or(0, |line| line.chars().count());
                    let spaces = TAB_SIZE - column % TAB_SIZE;
                    self.text.extend(std::iter::repeat_n(' ', spaces));
                }
                _ if ch.is_ascii_whitespace() && white_space.collapses_spaces() => {
                    let after_space = self.text.is_empty()
                        || self.text.ends_with(' ')
                        || self.text.ends_with('\n');
                    if !after_space {
                        self.text.push(' ');
                    }
                }
                _ => self.text.push(ch),
            }
        }
        if self.text.len() > start {
            self.spans.push((start..self.text.len(), style.clone()));
        }
    }

    /// Appends a forced line break (`<br>`).
    pub fn push_break(&mut self, style: &ComputedStyle) {
        self.push_newline();
        let end = self.text.len();
        self.spans.push((end - 1..end, style.clone()));
    }

    fn push_newline(&mut self) {
        // A collapsible space before a line break is removed.
        if self.text.ends_with(' ')
            && self
                .spans
                .last()
                .is_none_or(|(_, style)| style.white_space.collapses_spaces())
        {
            self.text.pop();
            if let Some((range, _)) = self.spans.last_mut() {
                range.end = range.end.min(self.text.len());
            }
        }
        self.text.push('\n');
    }

    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// The parley style properties for text with the given computed style.
fn text_properties<'a>(
    generics: &'a GenericFonts,
    style: &'a ComputedStyle,
) -> [StyleProperty<'a, Ink>; 8] {
    let language = style.lang.as_deref().and_then(|l| Language::parse(l).ok());
    let line_height = match style.line_height {
        LineHeight::Normal => ParleyLineHeight::MetricsRelative(1.0),
        LineHeight::Number(factor) => ParleyLineHeight::FontSizeRelative(factor),
        LineHeight::Percent(fraction) => ParleyLineHeight::FontSizeRelative(fraction),
        LineHeight::Length(length) => {
            ParleyLineHeight::Absolute(length.to_pt(style.font_size, style.root_font_size))
        }
    };
    let wrap = if style.white_space.wraps() {
        TextWrapMode::Wrap
    } else {
        TextWrapMode::NoWrap
    };
    [
        StyleProperty::FontFamily(generics.stack(&style.font_family)),
        StyleProperty::FontSize(style.font_size),
        StyleProperty::FontWeight(FontWeight::new(style.font_weight)),
        StyleProperty::FontStyle(if style.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }),
        StyleProperty::Locale(language),
        StyleProperty::Brush(Ink(style.color)),
        StyleProperty::LineHeight(line_height),
        StyleProperty::TextWrapMode(wrap),
    ]
}

impl Context<'_> {
    /// Shapes `text` into an unbroken parley layout. `style` is the block's
    /// style; `spans` override it for stretches of the text.
    fn shape(
        &mut self,
        text: &str,
        style: &ComputedStyle,
        spans: &[(Range<usize>, ComputedStyle)],
    ) -> Layout<Ink> {
        let mut builder = self.texts.ranged_builder(self.fonts, text, 1.0, false);
        for property in text_properties(self.generics, style) {
            builder.push_default(property);
        }
        for (range, span) in spans {
            for property in text_properties(self.generics, span) {
                builder.push(property, range.clone());
            }
        }
        builder.build(text)
    }
}

impl Flow<'_, '_> {
    /// Shapes and line-breaks a paragraph, adding one slice per line.
    pub(super) fn paragraph(&mut self, inline: Inline, style: &ComputedStyle, x: f32, width: f32) {
        if inline.is_blank() {
            return;
        }
        let text: Arc<str> = inline.text.into();
        let mut layout = self.cx.shape(&text, style, &inline.spans);

        if self.is_measuring() {
            let widths = layout.calculate_content_widths();
            self.record_extent(widths.min, widths.max);
            return;
        }

        self.settle_margin();
        let alignment = match style.text_align {
            TextAlign::Left => Alignment::Left,
            TextAlign::Right => Alignment::Right,
            TextAlign::Center => Alignment::Center,
            TextAlign::Justify => Alignment::Justify,
        };
        layout.set_alignment(alignment, AlignmentOptions::default());
        layout.break_all_lines(Some(width));

        let marker = self.marker.take();
        for (index, line) in layout.lines().enumerate() {
            let metrics = line.metrics();
            let top = metrics.block_min_coord;
            let mut items = Vec::new();
            let mut cursor = ClusterCursor::default();
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    let origin = Point {
                        x: x + run.offset(),
                        y: run.baseline() - top,
                    };
                    items.push(Item::Text(text_run(&run, origin, &text, &mut cursor)));
                }
            }
            if index == 0
                && let Some(marker) = &marker
            {
                items.extend(self.marker_items(marker, style, x, metrics.baseline - top));
            }
            self.push_slice(
                self.y + f64::from(top),
                metrics.block_max_coord - top,
                items,
            );
        }
        self.y += f64::from(layout.height());
    }

    /// The glyphs of a list marker, ending just left of `x` on the
    /// baseline at `baseline`.
    fn marker_items(
        &mut self,
        marker: &str,
        style: &ComputedStyle,
        x: f32,
        baseline: f32,
    ) -> Vec<Item> {
        let style = ComputedStyle {
            white_space: WhiteSpace::Pre,
            ..style.clone()
        };
        let text: Arc<str> = marker.into();
        let mut layout = self.cx.shape(&text, &style, &[]);
        layout.break_all_lines(None);
        let gap = style.font_size * 0.4;
        let start = x - layout.width() - gap;

        let mut items = Vec::new();
        if let Some(line) = layout.lines().next() {
            let mut cursor = ClusterCursor::default();
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    let origin = Point {
                        x: start + run.offset(),
                        y: baseline,
                    };
                    items.push(Item::Text(text_run(&run, origin, &text, &mut cursor)));
                }
            }
        }
        items
    }
}

/// The marker text for the list item `item`, e.g. `•` or `3.`.
pub(super) fn list_marker(doc: &Document, item: NodeId, style: &ComputedStyle) -> Option<String> {
    let ordinal = || -> i64 {
        let Some(parent) = doc.node(item).parent else {
            return 1;
        };
        let start = doc
            .element(parent)
            .and_then(|list| list.attr("start"))
            .and_then(|start| start.trim().parse().ok())
            .unwrap_or(1);
        let position = doc
            .children(parent)
            .filter(|&c| doc.element(c).is_some_and(|e| e.name == "li"))
            .position(|c| c == item)
            .unwrap_or(0);
        start + position as i64
    };
    Some(match style.list_style_type {
        ListStyleType::None => return None,
        ListStyleType::Disc => "•".into(),
        ListStyleType::Circle => "◦".into(),
        ListStyleType::Square => "▪".into(),
        ListStyleType::Decimal => format!("{}.", ordinal()),
        ListStyleType::LowerAlpha => format!("{}.", alphabetic(ordinal(), false)),
        ListStyleType::UpperAlpha => format!("{}.", alphabetic(ordinal(), true)),
        ListStyleType::LowerRoman => format!("{}.", roman(ordinal(), false)),
        ListStyleType::UpperRoman => format!("{}.", roman(ordinal(), true)),
    })
}

/// `1` → `a`, `26` → `z`, `27` → `aa`. Falls back to decimal below 1.
fn alphabetic(mut n: i64, upper: bool) -> String {
    if n < 1 {
        return n.to_string();
    }
    let base = if upper { b'A' } else { b'a' };
    let mut letters = Vec::new();
    while n > 0 {
        n -= 1;
        letters.push(base + (n % 26) as u8);
        n /= 26;
    }
    letters.reverse();
    String::from_utf8(letters).expect("ASCII letters")
}

/// Roman numerals for 1 to 3999; decimal outside that range.
fn roman(mut n: i64, upper: bool) -> String {
    if !(1..=3999).contains(&n) {
        return n.to_string();
    }
    const NUMERALS: [(i64, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, numeral) in NUMERALS {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    if upper { out.to_uppercase() } else { out }
}

/// Tracks how many clusters of a shaped run earlier glyph runs on the same
/// line have used.
///
/// parley splits a shaped run into glyph runs where the style changes
/// (e.g. the color), but only exposes clusters, which know their text
/// range, per shaped run. Glyph runs of one shaped run come in order, so
/// each takes the next clusters.
#[derive(Default)]
struct ClusterCursor {
    run: Option<usize>,
    used: usize,
}

/// Converts a parley glyph run into a [`TextRun`] starting at `origin`.
fn text_run(
    run: &GlyphRun<'_, Ink>,
    origin: Point,
    text: &Arc<str>,
    cursor: &mut ClusterCursor,
) -> TextRun {
    let shaped = run.run();
    if cursor.run != Some(shaped.index()) {
        *cursor = ClusterCursor {
            run: Some(shaped.index()),
            used: 0,
        };
    }

    let expected: Vec<_> = run.glyphs().collect();
    let mut glyphs = Vec::with_capacity(expected.len());
    let mut used = 0;
    for cluster in shaped.visual_clusters().skip(cursor.used) {
        if glyphs.len() >= expected.len() {
            break;
        }
        used += 1;
        let range = cluster.text_range();
        glyphs.extend(cluster.glyphs().map(|g| Glyph {
            id: g.id,
            advance: g.advance,
            x_offset: g.x,
            y_offset: g.y,
            text_range: range.clone(),
        }));
    }
    cursor.used += used;

    let matches =
        glyphs.len() == expected.len() && glyphs.iter().zip(&expected).all(|(a, b)| a.id == b.id);
    if !matches {
        // Fall back to attributing the whole run's text to every glyph,
        // which draws correctly but makes text selection coarse.
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
        color: run.style().brush.0,
        origin,
        glyphs,
        text: Arc::clone(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collapse(parts: &[(&str, WhiteSpace)]) -> String {
        let mut inline = Inline::default();
        for (text, white_space) in parts {
            let style = ComputedStyle {
                white_space: *white_space,
                ..ComputedStyle::initial()
            };
            inline.push_text(text, &style);
        }
        inline.text
    }

    #[test]
    fn normal_collapses_white_space_across_spans() {
        use WhiteSpace::Normal;
        assert_eq!(collapse(&[("  Hello \n\t world ", Normal)]), "Hello world ");
        assert_eq!(collapse(&[("a ", Normal), (" b", Normal)]), "a b");
    }

    #[test]
    fn pre_keeps_spaces_and_newlines_and_expands_tabs() {
        assert_eq!(
            collapse(&[("a  b\r\nc\td", WhiteSpace::Pre)]),
            "a  b\nc       d"
        );
    }

    #[test]
    fn pre_line_keeps_only_newlines() {
        assert_eq!(collapse(&[("a   b \n  c", WhiteSpace::PreLine)]), "a b\nc");
    }

    #[test]
    fn breaks_trim_the_space_before_them() {
        let style = ComputedStyle::initial();
        let mut inline = Inline::default();
        inline.push_text("one ", &style);
        inline.push_break(&style);
        inline.push_text(" two", &style);
        assert_eq!(inline.text, "one\ntwo");
        for (range, _) in &inline.spans {
            assert!(range.end <= inline.text.len() && range.start <= range.end);
        }
    }

    #[test]
    fn list_counters() {
        assert_eq!(alphabetic(1, false), "a");
        assert_eq!(alphabetic(27, true), "AA");
        assert_eq!(roman(1994, true), "MCMXCIV");
        assert_eq!(roman(4, false), "iv");
        assert_eq!(roman(0, false), "0");
    }
}
