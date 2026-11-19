//! Inline layout: gathering text, shaping it and breaking it into lines.

use std::ops::Range;
use std::sync::Arc;

use parley::{
    Alignment, AlignmentOptions, FontStyle, FontWeight, GlyphRun, IndentOptions, InlineBox,
    InlineBoxKind, Language, Layout, LineHeight as ParleyLineHeight, PositionedLayoutItem,
    StyleProperty, TextWrapMode, VerticalAlign as ParleyVerticalAlign,
};
use presswork_dom::{Document, NodeId};
use presswork_style::{
    Color, ComputedStyle, FontFamilyName, LineHeight, ListStyleType, TextAlign, TextDecorationLine,
    TextDecorations, TextTransform, VerticalAlign, WhiteSpace,
};
use read_fonts::{FontRef, TableProvider};

use super::{Context, Flow, GenericFonts, WebFonts};
use crate::fragment::{Fill, FontData, Glyph, Item, TextRun};
use crate::geometry::{Point, Rect};

/// How text is painted, as parley's per-span "brush": its color and the
/// lines drawn on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ink {
    pub color: Color,
    pub decorations: TextDecorations,
}

impl Default for Ink {
    fn default() -> Self {
        Ink {
            color: Color::BLACK,
            decorations: TextDecorations::NONE,
        }
    }
}

/// Inline content gathered for one paragraph: the text after white-space
/// processing, the style of each stretch of it, and the boxes (images)
/// that flow with it.
#[derive(Default)]
pub(super) struct Inline {
    text: String,
    spans: Vec<(Range<usize>, ComputedStyle)>,
    boxes: Vec<InlineObject>,
    /// The paragraph starts its block container, so `text-indent`
    /// applies to it.
    first: bool,
}

/// An atomic box laid out inline, like a character: an image.
pub(super) struct InlineObject {
    /// Byte offset of the U+FFFC character that stands for the box in the
    /// paragraph's text. Set by [`Inline::push_box`].
    index: usize,
    /// Margin box size.
    pub width: f32,
    pub height: f32,
    /// The narrowest the box can be: zero for an image sized as a
    /// percentage of its container, which shrinks to fit; otherwise
    /// `width`. Only used when measuring.
    pub min_width: f32,
    pub vertical_align: VerticalAlign,
    /// Content relative to the top-left of the margin box.
    pub items: Vec<Item>,
}

/// Tab stops in preformatted text, in characters.
const TAB_SIZE: usize = 8;

/// Stands for an inline box in the text given to parley.
const OBJECT_REPLACEMENT: char = '\u{FFFC}';

impl InlineObject {
    pub fn new(
        width: f32,
        height: f32,
        min_width: f32,
        vertical_align: VerticalAlign,
        items: Vec<Item>,
    ) -> Self {
        Self {
            index: 0,
            width,
            height,
            min_width,
            vertical_align,
            items,
        }
    }
}

impl Inline {
    /// Content that starts a block container: `text-indent` applies to it.
    /// Taking it with `std::mem::take` leaves content that does not.
    pub fn first() -> Self {
        Self {
            first: true,
            ..Self::default()
        }
    }

    /// Appends text, applying the `white-space` rules of `style`.
    pub fn push_text(&mut self, text: &str, style: &ComputedStyle) {
        let white_space = style.white_space;
        let start = self.text.len();
        let transformed = self.transform(text, style.text_transform);
        let mut chars = transformed.chars().peekable();
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

    /// Applies `text-transform` to `text`, which follows the text gathered
    /// so far: a word that `capitalize` capitalizes may have started in an
    /// earlier span.
    fn transform(&self, text: &str, transform: TextTransform) -> String {
        match transform {
            TextTransform::None => text.to_owned(),
            TextTransform::Uppercase => text.to_uppercase(),
            TextTransform::Lowercase => text.to_lowercase(),
            TextTransform::Capitalize => {
                let mut in_word = self.text.chars().next_back().is_some_and(is_word_char);
                let mut out = String::with_capacity(text.len());
                for ch in text.chars() {
                    if !in_word && ch.is_alphabetic() {
                        out.extend(ch.to_uppercase());
                    } else {
                        out.push(ch);
                    }
                    in_word = is_word_char(ch);
                }
                out
            }
        }
    }

    /// Appends a forced line break (`<br>`).
    pub fn push_break(&mut self, style: &ComputedStyle) {
        self.push_newline();
        let end = self.text.len();
        self.spans.push((end - 1..end, style.clone()));
    }

    /// Appends an atomic inline box. Spaces around it are kept.
    pub fn push_box(&mut self, mut object: InlineObject, style: &ComputedStyle) {
        object.index = self.text.len();
        self.text.push(OBJECT_REPLACEMENT);
        self.spans
            .push((object.index..self.text.len(), style.clone()));
        self.boxes.push(object);
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

/// Whether `ch` continues a word for `text-transform: capitalize`.
fn is_word_char(ch: char) -> bool {
    // Marks continue the word they attach to (Devanagari vowel signs);
    // apostrophes keep "don't" one word.
    ch.is_alphanumeric() || matches!(ch, '\'' | '’') || is_mark(ch)
}

/// A rough test for combining marks, enough to keep them inside words.
fn is_mark(ch: char) -> bool {
    matches!(ch as u32, 0x0300..=0x036F | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963)
}

/// The parley style properties for text with the given computed style.
fn text_properties<'a>(
    generics: &'a GenericFonts,
    web: &'a WebFonts<'_>,
    style: &'a ComputedStyle,
) -> [StyleProperty<'a, Ink>; 10] {
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
        StyleProperty::FontFamily(generics.stack(web, &style.font_family)),
        StyleProperty::FontSize(style.font_size),
        StyleProperty::FontWeight(FontWeight::new(style.font_weight)),
        StyleProperty::FontStyle(if style.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }),
        StyleProperty::Locale(language),
        StyleProperty::Brush(Ink {
            color: style.color,
            decorations: style.text_decorations(),
        }),
        StyleProperty::LineHeight(line_height),
        StyleProperty::TextWrapMode(wrap),
        StyleProperty::LetterSpacing(style.letter_spacing),
        StyleProperty::WordSpacing(style.word_spacing),
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
        boxes: &[InlineObject],
    ) -> Layout<Ink> {
        let styles = std::iter::once(style).chain(spans.iter().map(|(_, span)| span));
        for style in styles {
            for family in style.font_family.iter() {
                if let FontFamilyName::Named(name) = family {
                    self.web_fonts.load(name, self.fonts, self.fetcher);
                }
            }
        }

        let mut builder = self.texts.ranged_builder(self.fonts, text, 1.0, false);
        for property in text_properties(self.generics, &self.web_fonts, style) {
            builder.push_default(property);
        }
        for (range, span) in spans {
            for property in text_properties(self.generics, &self.web_fonts, span) {
                builder.push(property, range.clone());
            }
        }
        for (id, object) in boxes.iter().enumerate() {
            builder.push_inline_box(InlineBox {
                id: id as u64,
                kind: InlineBoxKind::InFlow,
                index: object.index,
                width: object.width,
                height: object.height,
                baseline: None,
                vertical_align: vertical_align(object.vertical_align),
            });
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
        let boxes = inline.boxes;
        let mut layout = self.cx.shape(&text, style, &inline.spans, &boxes);
        if inline.first {
            let indent = style.text_indent;
            let amount = indent.length.resolve(width).unwrap_or(0.0);
            if amount != 0.0 {
                let options = IndentOptions {
                    each_line: indent.each_line,
                    hanging: indent.hanging,
                };
                layout.set_text_indent(amount, options);
            }
        }

        if self.is_measuring() {
            let max = layout.calculate_content_widths().max;
            // Boxes that can shrink count at their narrowest for the
            // minimum.
            for (index, object) in boxes.iter().enumerate() {
                if object.min_width < object.width {
                    layout.edit_inline_box(index, |b| b.width = object.min_width);
                }
            }
            let min = layout.calculate_content_widths().min;
            self.record_extent(min, max);
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
        let count = layout.len();
        // Lines that must share a page with the line before them: the
        // first `orphans` lines stay together, and so do the last `widows`.
        let (orphans, widows) = (style.orphans as usize, style.widows as usize);
        let kept = |index: usize| index > 0 && (index < orphans || index + widows > count);
        for (index, line) in layout.lines().enumerate() {
            let metrics = line.metrics();
            let top = metrics.block_min_coord;
            let mut items = Vec::new();
            let mut cursor = ClusterCursor::default();
            let count = line.items().count();
            for (position, item) in line.items().enumerate() {
                match item {
                    PositionedLayoutItem::GlyphRun(run) => {
                        let origin = Point {
                            x: x + run.offset(),
                            y: run.baseline() - top,
                        };
                        let text_run = text_run(&run, origin, &text, &mut cursor);
                        let decorations = run.style().brush.decorations;
                        if decorations.is_empty() {
                            items.push(Item::Text(text_run));
                            continue;
                        }
                        // Lines are not drawn under the spaces that end a line.
                        let trailing = if position + 1 == count && !run.run().is_rtl() {
                            trailing_space(&text_run)
                        } else {
                            0.0
                        };
                        let lines = DecorationLines::new(&run, decorations, top, trailing);
                        lines.fills(x, UNDER_TEXT, &mut items);
                        items.push(Item::Text(text_run));
                        lines.fills(x, OVER_TEXT, &mut items);
                    }
                    PositionedLayoutItem::InlineBox(placed) => {
                        let object = &boxes[placed.id as usize];
                        items.extend(
                            object
                                .items
                                .iter()
                                .cloned()
                                .map(|item| item.translate(x + placed.x, placed.y - top)),
                        );
                    }
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
            if kept(index) {
                self.keep_last_slice();
            }
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
        // Markers are not underlined with the item's text.
        let style = ComputedStyle {
            white_space: WhiteSpace::Pre,
            text_decoration_line: TextDecorationLine::NONE,
            inherited_decorations: TextDecorations::NONE,
            ..style.clone()
        };
        let text: Arc<str> = marker.into();
        let mut layout = self.cx.shape(&text, &style, &[], &[]);
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

/// The decoration lines of one glyph run, relative to its slice.
struct DecorationLines {
    decorations: TextDecorations,
    /// Left edge, relative to the paragraph's left edge, and length.
    start: f32,
    width: f32,
    /// Tops and thicknesses.
    underline: (f32, f32),
    overline: (f32, f32),
    line_through: (f32, f32),
}

impl DecorationLines {
    /// Positions the lines. `top` is the line's top; `trailing` the width
    /// of spaces at the end of the run that are left undecorated.
    ///
    /// Overlines and line-throughs are placed from the ascent of the
    /// span's primary font, as in Blink, so that they keep one height
    /// across fallback fonts: Devanagari fonts put their own strikeout
    /// much higher than Latin ones. Underlines use the run's font, whose
    /// position agrees closely enough between fonts.
    fn new(run: &GlyphRun<'_, Ink>, decorations: TextDecorations, top: f32, trailing: f32) -> Self {
        let metrics = run.run().font_metrics();
        let font_size = run.run().font_size();
        // Fonts without the metrics get a line of a typical thickness.
        let thickness = |size: f32| {
            if size > 0.0 { size } else { font_size / 16.0 }
        };
        let baseline = run.baseline() - top;
        let under = thickness(metrics.underline_size);
        // The offset is to the top of the line, upwards from the baseline.
        let underline_offset = if metrics.underline_offset != 0.0 {
            metrics.underline_offset
        } else {
            -font_size / 10.0
        };
        let ascent = match run.span_metrics().ascent {
            ascent if ascent > 0.0 => ascent,
            _ => metrics.ascent,
        };
        let strike = thickness(metrics.strikethrough_size);
        Self {
            decorations,
            start: run.offset(),
            width: (run.advance() - trailing).max(0.0),
            underline: (baseline - underline_offset, under),
            overline: (baseline - ascent, under),
            line_through: (baseline - ascent / 3.0 - strike / 2.0, strike),
        }
    }

    /// Appends the fills for the lines in `which`, shifted right by `x`.
    fn fills(&self, x: f32, which: TextDecorationLine, out: &mut Vec<Item>) {
        let lines = [
            (which.underline, self.decorations.underline, self.underline),
            (which.overline, self.decorations.overline, self.overline),
            (
                which.line_through,
                self.decorations.line_through,
                self.line_through,
            ),
        ];
        for (wanted, color, (y, thickness)) in lines {
            if let (true, Some(color)) = (wanted, color)
                && self.width > 0.0
            {
                out.push(Item::Fill(Fill {
                    rect: Rect::new(x + self.start, y, self.width, thickness),
                    color,
                }));
            }
        }
    }
}

/// Paint order: underlines and overlines go below the text, line-through
/// above it (CSS Text Decoration 3, section 2.1).
const UNDER_TEXT: TextDecorationLine = TextDecorationLine {
    underline: true,
    overline: true,
    line_through: false,
};
const OVER_TEXT: TextDecorationLine = TextDecorationLine {
    underline: false,
    overline: false,
    line_through: true,
};

/// The advance of the whitespace glyphs at the end of `run`.
fn trailing_space(run: &TextRun) -> f32 {
    run.glyphs
        .iter()
        .rev()
        .take_while(|glyph| {
            let text = &run.text[glyph.text_range.clone()];
            !text.is_empty() && text.chars().all(char::is_whitespace)
        })
        .map(|glyph| glyph.advance)
        .sum()
}

/// parley's equivalent of a `vertical-align` value.
fn vertical_align(value: VerticalAlign) -> ParleyVerticalAlign {
    match value {
        VerticalAlign::Baseline => ParleyVerticalAlign::BASELINE,
        VerticalAlign::Sub => ParleyVerticalAlign::SUB,
        VerticalAlign::Super => ParleyVerticalAlign::SUPER,
        VerticalAlign::TextTop => ParleyVerticalAlign::TEXT_TOP,
        VerticalAlign::TextBottom => ParleyVerticalAlign::TEXT_BOTTOM,
        VerticalAlign::Middle => ParleyVerticalAlign::MIDDLE,
        VerticalAlign::Top => ParleyVerticalAlign::TOP,
        VerticalAlign::Bottom => ParleyVerticalAlign::BOTTOM,
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
    // The text range of each cluster with glyphs, and its glyphs' indices
    // in `glyphs`.
    let mut clusters: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    // Ligature continuations: the characters after the first of a
    // ligature (the i of an fi ligature), which have no glyphs of their
    // own; the ligature start has them all.
    let mut continuations: Vec<Range<usize>> = Vec::new();
    let mut used = 0;
    for cluster in shaped.visual_clusters().skip(cursor.used) {
        if cluster.is_ligature_continuation() {
            used += 1;
            continuations.push(cluster.text_range());
            continue;
        }
        if glyphs.len() >= expected.len() {
            break;
        }
        used += 1;
        let first = glyphs.len();
        glyphs.extend(cluster.glyphs().map(|g| Glyph {
            id: g.id,
            advance: g.advance,
            x_offset: g.x,
            y_offset: g.y,
            text_range: 0..0,
        }));
        clusters.push((cluster.text_range(), first..glyphs.len()));
    }
    cursor.used += used;
    // A ligature's glyphs stand for all its characters: extend each start
    // over the continuations that follow it in the text. In text order, so
    // that a three-letter ligature (ffi) grows one character at a time
    // whatever the direction.
    continuations.sort_by_key(|range| range.start);
    for continuation in continuations {
        if let Some((range, _)) = clusters
            .iter_mut()
            .find(|(range, _)| range.end == continuation.start)
        {
            range.end = continuation.end;
        }
    }
    for (range, indices) in clusters {
        for glyph in &mut glyphs[indices] {
            glyph.text_range = range.clone();
        }
    }

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

    split_plain_clusters(&mut glyphs, text, shaped.font());
    TextRun {
        font: shaped.font().clone(),
        font_size: shaped.font_size(),
        color: run.style().brush.color,
        origin,
        glyphs,
        text: Arc::clone(text),
    }
}

/// Gives each glyph of a cluster its own character when the cluster is
/// just its characters' default glyphs, in order: a consonant and the vowel
/// sign under it (कृ), a letter and a combining accent.
///
/// parley maps every glyph of a cluster to the whole cluster, which a PDF
/// can only express with `/ActualText`. Text extraction (poppler, at least)
/// then takes the cluster to end where its last glyph ends, and a final
/// mark shifted left reads as a gap: "कृष्ण" was extracted as "कृ ष्ण".
/// Mapped one character per glyph, no `/ActualText` is needed.
///
/// Clusters with substituted glyphs (conjuncts such as ष्ण) or reordered
/// ones (ि, drawn before its consonant) keep the whole-cluster mapping,
/// the only correct one for them.
fn split_plain_clusters(glyphs: &mut [Glyph], text: &str, font: &FontData) {
    // Parsed on first need: most clusters are a single glyph.
    let mut font_ref = None;
    let mut start = 0;
    while start < glyphs.len() {
        let range = glyphs[start].text_range.clone();
        let len = glyphs[start..]
            .iter()
            .take_while(|g| g.text_range == range)
            .count();
        let cluster = &mut glyphs[start..start + len];
        start += len;

        let chars = &text[range.clone()];
        if len < 2 || chars.chars().count() != len {
            continue;
        }
        let font_ref = font_ref.get_or_insert_with(|| {
            FontRef::from_index(font.data.data(), font.index)
                .ok()
                .and_then(|font| font.cmap().ok())
        });
        let Some(cmap) = font_ref else {
            return;
        };
        let plain = chars.chars().zip(cluster.iter()).all(|(ch, glyph)| {
            cmap.map_codepoint(ch)
                .is_some_and(|id| id.to_u32() == glyph.id)
        });
        if plain {
            for ((offset, ch), glyph) in chars.char_indices().zip(cluster.iter_mut()) {
                let char_start = range.start + offset;
                glyph.text_range = char_start..char_start + ch.len_utf8();
            }
        }
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
