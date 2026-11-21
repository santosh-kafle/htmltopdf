//! Block, inline and table layout on one continuous canvas.
//!
//! Content is laid out top to bottom as if the page were infinitely tall,
//! producing a [`Canvas`]: [`Slice`]s, which are unbreakable horizontal
//! bands (a line of text, a table row), and [`Decoration`]s, the
//! backgrounds and borders of boxes that may span many slices.
//! [`crate::paginate`] then cuts the canvas into pages between slices.
//!
//! # Coordinates
//!
//! The vertical position on the canvas is an `f64`, because a long document
//! is hundreds of thousands of points tall and `f32` would lose sub-point
//! precision. Everything else is `f32` and small: horizontal positions, and
//! item positions relative to the top of their slice.
//!
//! # Modules
//!
//! - this module: the [`Flow`] state, block boxes and margin collapsing
//! - [`boxes`]: resolving the box model
//! - [`inline`]: gathering inline content, shaping and line breaking
//! - [`table`]: table layout
//! - [`replaced`]: sizing and placing images
//! - [`fonts`]: mapping CSS font families to fonts
//! - [`webfonts`]: `@font-face` fonts

mod boxes;
mod fonts;
mod inline;
mod inline_block;
mod replaced;
mod table;
mod webfonts;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parley::{FontContext, LayoutContext};
use presswork_dom::resource::{Fetcher, LoadError};
use presswork_dom::{Document, NodeId, NodeKind};
use presswork_style::{
    Break, BreakInside, Color, ComputedStyle, Dimension, Display, Side, Stylist,
};

use crate::fragment::{Fill, ImageData, Item};
use crate::geometry::Rect;
use boxes::BoxGeometry;
pub(crate) use fonts::GenericFonts;
pub(crate) use inline::Ink;
use inline::Inline;
pub(crate) use webfonts::WebFonts;
#[cfg(test)]
pub(crate) use webfonts::tests as webfont_tests;

/// An unbreakable band of content.
#[derive(Debug)]
pub(crate) struct Slice {
    /// Canvas position of the top of the band.
    pub top: f64,
    pub height: f32,
    /// Positioned relative to the canvas's left edge and the slice's top.
    pub items: Vec<Item>,
    /// The slice must start a new page.
    pub break_before: bool,
    /// The slice should stay on the page of the slice before it.
    pub keep: Keep,
    /// Table header to repeat above this slice when it starts a page.
    pub header: Option<Rc<RepeatedHeader>>,
}

/// Why a slice should not be separated from the slice before it. A page
/// break goes elsewhere if it can; see [`crate::paginate`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Keep {
    /// A small group: the first or last lines of a paragraph (`orphans`,
    /// `widows`), content after a box with `break-after: avoid` (headings)
    /// or before one with `break-before: avoid`, the first row after a
    /// table's header rows.
    pub tight: bool,
    /// Inside a box with `break-inside: avoid`, which may be large. Given
    /// up first when a group does not fit on a page.
    pub inside: bool,
}

/// Table header rows, repeated at the top of each page a table continues
/// on.
#[derive(Debug)]
pub(crate) struct RepeatedHeader {
    pub height: f32,
    /// Positioned relative to the canvas's left edge and the header's top.
    pub items: Vec<Item>,
}

/// The background and borders of a box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoxPaint {
    pub background: Color,
    /// Width and color of each border; a width of zero means no border.
    pub border: [(Side, f32, Color); 4],
}

impl BoxPaint {
    /// Paints nothing.
    pub const INVISIBLE: Self = Self {
        background: Color::TRANSPARENT,
        border: [
            (Side::Top, 0.0, Color::TRANSPARENT),
            (Side::Right, 0.0, Color::TRANSPARENT),
            (Side::Bottom, 0.0, Color::TRANSPARENT),
            (Side::Left, 0.0, Color::TRANSPARENT),
        ],
    };

    /// The paint for a box with this style, or `None` if nothing is
    /// visible.
    pub fn from_style(style: &ComputedStyle) -> Option<Self> {
        let widths = style.border_widths();
        let border = Side::ALL.map(|side| (side, widths.get(side), style.border_color(side)));
        let visible = !style.background_color.is_transparent()
            || border
                .iter()
                .any(|(_, width, color)| *width > 0.0 && !color.is_transparent());
        visible.then_some(Self {
            background: style.background_color,
            border,
        })
    }

    /// Removes the border on `side`.
    pub fn without_border(mut self, side: Side) -> Self {
        for edge in &mut self.border {
            if edge.0 == side {
                edge.1 = 0.0;
            }
        }
        self
    }

    /// Appends the fills for a box whose border box is `rect`.
    pub fn fills(&self, rect: Rect, out: &mut Vec<Item>) {
        let Rect { origin, size } = rect;
        let mut fill = |x: f32, y: f32, width: f32, height: f32, color: Color| {
            if width > 0.0 && height > 0.0 && !color.is_transparent() {
                out.push(Item::Fill(Fill {
                    rect: Rect::new(x, y, width, height),
                    color,
                }));
            }
        };
        fill(origin.x, origin.y, size.width, size.height, self.background);
        for (side, width, color) in self.border {
            let width = match side {
                Side::Top | Side::Bottom => width.min(size.height),
                Side::Left | Side::Right => width.min(size.width),
            };
            match side {
                Side::Top => fill(origin.x, origin.y, size.width, width, color),
                Side::Bottom => fill(
                    origin.x,
                    origin.y + size.height - width,
                    size.width,
                    width,
                    color,
                ),
                Side::Left => fill(origin.x, origin.y, width, size.height, color),
                Side::Right => fill(
                    origin.x + size.width - width,
                    origin.y,
                    width,
                    size.height,
                    color,
                ),
            }
        }
    }
}

/// A box's background and borders, spanning a vertical range of the
/// canvas.
#[derive(Debug)]
pub(crate) struct Decoration {
    pub top: f64,
    pub bottom: f64,
    /// Left edge of the border box.
    pub x: f32,
    pub width: f32,
    pub paint: BoxPaint,
}

/// Laid-out content in canvas coordinates.
#[derive(Debug, Default)]
pub(crate) struct Canvas {
    /// In top-to-bottom order.
    pub slices: Vec<Slice>,
    /// In paint order: a box comes before its descendants.
    pub decorations: Vec<Decoration>,
}

impl Canvas {
    /// Flattens a small canvas (the contents of a table cell) into items
    /// translated by `(dx, dy)`. Decorations paint first.
    pub fn into_items(self, dx: f32, dy: f32, out: &mut Vec<Item>) {
        for decoration in self.decorations {
            let rect = Rect::new(
                decoration.x + dx,
                decoration.top as f32 + dy,
                decoration.width,
                (decoration.bottom - decoration.top) as f32,
            );
            decoration.paint.fills(rect, out);
        }
        for slice in self.slices {
            let dy = slice.top as f32 + dy;
            out.extend(slice.items.into_iter().map(|item| item.translate(dx, dy)));
        }
    }
}

/// The widest and narrowest a piece of content can be laid out, measured
/// from the left edge of its container.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Extent {
    /// Width with every optional line break taken.
    pub min: f32,
    /// Width with no optional line break taken.
    pub max: f32,
}

/// Everything layout reads from or caches in, shared by nested flows.
pub(crate) struct Context<'a> {
    pub fonts: &'a mut FontContext,
    pub generics: &'a GenericFonts,
    pub texts: &'a mut LayoutContext<Ink>,
    pub doc: &'a Document,
    pub styles: &'a Stylist,
    pub fetcher: &'a Fetcher<'a>,
    pub web_fonts: WebFonts<'a>,
    /// Whether to load images at all (wkhtmltopdf's `--no-images`).
    pub load_images: bool,
    /// Images by URL, `None` for those that failed, so each loads once.
    pub images: HashMap<String, Option<Arc<ImageData>>>,
}

impl Context<'_> {
    /// The image at `url`, loaded on first use. Failures are recorded by
    /// the fetcher.
    fn image(&mut self, url: &str) -> Option<Arc<ImageData>> {
        let url = url.trim();
        if url.is_empty() || !self.load_images {
            // Browsers do not load an empty `src` either.
            return None;
        }
        if let Some(image) = self.images.get(url) {
            return image.clone();
        }
        let image = self
            .fetcher
            .try_fetch(url, None)
            .and_then(|bytes| ImageData::new(bytes, url).map_err(LoadError::Failed));
        let image = match image {
            Ok(image) => Some(Arc::new(image)),
            Err(error) => {
                self.fetcher.record(url, error);
                None
            }
        };
        self.images.insert(url.to_owned(), image.clone());
        image
    }
}

/// Lays out `text` on its own as a paragraph in `style`, `width` wide, for
/// text that is not part of the document, such as a page header.
pub(crate) fn text_block(
    cx: &mut Context<'_>,
    text: &str,
    style: &ComputedStyle,
    width: f32,
) -> Canvas {
    let mut flow = Flow::new(cx);
    let mut inline = Inline::default();
    inline.push_text(text, style);
    flow.paragraph(inline, style, 0.0, width);
    flow.finish().0
}

/// What a [`Flow`] produces.
enum Mode {
    /// Positioned content.
    Layout,
    /// Only the min- and max-content widths, for table column sizing.
    Measure(Extent),
}

/// Lays out one block formatting context: the document, or a table cell.
pub(crate) struct Flow<'c, 'a> {
    cx: &'c mut Context<'a>,
    mode: Mode,
    canvas: Canvas,
    /// Bottom of the content so far, before any pending margin.
    y: f64,
    /// Collapsed margin waiting to be added before the next content.
    margin: f32,
    /// Decorations whose top edge is not known yet: boxes without top
    /// border or padding start where their first content lands, after
    /// margins collapse through them.
    unplaced: Vec<usize>,
    /// Horizontal margins, borders and padding of the enclosing boxes, for
    /// [`Mode::Measure`].
    edges: f32,
    /// A list marker waiting for the first line of its list item.
    marker: Option<String>,
    /// The next slice must start a new page.
    force_break: bool,
    /// The next slice should stay with the one before it.
    keep_next: bool,
    /// An element whose border box to report, and the decoration that
    /// records it once laid out. See [`Flow::track`].
    tracked: Option<(NodeId, Option<usize>)>,
    /// The baseline of the last line laid out, the baseline of an
    /// inline-block.
    baseline: Option<f64>,
}

/// A box entered with [`Flow::open_box`].
struct OpenBox {
    decoration: Option<usize>,
    /// The index of the box's first slice, if it has any.
    first_slice: usize,
}

impl<'c, 'a> Flow<'c, 'a> {
    pub fn new(cx: &'c mut Context<'a>) -> Self {
        Self::with_mode(cx, Mode::Layout)
    }

    fn measuring(cx: &'c mut Context<'a>) -> Self {
        Self::with_mode(cx, Mode::Measure(Extent::default()))
    }

    fn with_mode(cx: &'c mut Context<'a>, mode: Mode) -> Self {
        Self {
            cx,
            mode,
            canvas: Canvas::default(),
            y: 0.0,
            margin: 0.0,
            unplaced: Vec::new(),
            edges: 0.0,
            marker: None,
            force_break: false,
            keep_next: false,
            tracked: None,
            baseline: None,
        }
    }

    /// Asks for the canvas position of `id`'s border box, which
    /// [`Flow::tracked_box`] gives once it is laid out.
    pub fn track(&mut self, id: NodeId) {
        self.tracked = Some((id, None));
    }

    /// The top and bottom of the tracked element's border box, if it was
    /// laid out as a block.
    pub fn tracked_box(&self) -> Option<(f64, f64)> {
        let index = self.tracked?.1?;
        let decoration = &self.canvas.decorations[index];
        Some((decoration.top, decoration.bottom))
    }

    /// Finishes the flow, returning its content and total height.
    pub fn finish(mut self) -> (Canvas, f64) {
        self.settle_margin();
        (self.canvas, self.y)
    }

    fn is_measuring(&self) -> bool {
        matches!(self.mode, Mode::Measure(_))
    }

    /// Records content that is `min` to `max` wide inside the current box.
    fn record_extent(&mut self, min: f32, max: f32) {
        let edges = self.edges;
        if let Mode::Measure(extent) = &mut self.mode {
            extent.min = extent.min.max(edges + min);
            extent.max = extent.max.max(edges + max);
        }
    }

    fn extent(&self) -> Extent {
        match self.mode {
            Mode::Measure(extent) => extent,
            Mode::Layout => Extent::default(),
        }
    }

    fn add_margin(&mut self, margin: f32) {
        // Adjacent vertical margins collapse to the largest one.
        self.margin = self.margin.max(margin);
    }

    /// Commits the pending margin: content is about to be placed at `y`.
    fn settle_margin(&mut self) {
        self.y += f64::from(self.margin);
        self.margin = 0.0;
        for index in self.unplaced.drain(..) {
            self.canvas.decorations[index].top = self.y;
        }
    }

    /// Adds a slice `height` tall at `top`, with items relative to it.
    fn push_slice(&mut self, top: f64, height: f32, items: Vec<Item>) {
        self.canvas.slices.push(Slice {
            top,
            height,
            items,
            break_before: std::mem::take(&mut self.force_break),
            keep: Keep {
                tight: std::mem::take(&mut self.keep_next),
                inside: false,
            },
            header: None,
        });
    }

    /// Keeps the last slice added with the one before it.
    fn keep_last_slice(&mut self) {
        if let Some(slice) = self.canvas.slices.last_mut() {
            slice.keep.tight = true;
        }
    }

    /// Lays out the block-level element `id` in a containing block whose
    /// content box starts at `x` and is `width` wide.
    pub fn block(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        if replaced::is_image(self.cx.doc, id) {
            return self.block_image(id, style, x, width);
        }
        if style.display == Display::Table {
            return self.table(id, style, x, width);
        }
        let geometry = BoxGeometry::resolve(style, width, None);
        if self.is_measuring()
            && let Dimension::Pt(_) = style.width
        {
            // A fixed-width box is as wide as it says, whatever is inside.
            let outer = geometry.horizontal_edges() + geometry.content_width;
            self.record_extent(outer, outer);
            return;
        }
        let tracked = self.tracked.is_some_and(|(tracked, _)| tracked == id);
        let min_height = geometry.min_content_height(style);
        let sized = min_height > 0.0 && !self.is_measuring();
        // A box with a height needs a decoration to know where its content
        // starts.
        let open = self.open_box(style, &geometry, x, tracked || sized);
        if tracked && let Some((_, index)) = &mut self.tracked {
            *index = open.decoration;
        }
        self.block_contents(id, style, x + geometry.content_x(), geometry.content_width);
        if sized && let Some(index) = open.decoration {
            self.extend_to(index, geometry.top_edge(), min_height);
        }
        self.close_box(style, &geometry, open);
    }

    /// Starts a box: top margin, border and padding, and its decoration.
    /// A `tracked` box gets a decoration even if it paints nothing, so
    /// that its position is worked out like any other's.
    fn open_box(
        &mut self,
        style: &ComputedStyle,
        geometry: &BoxGeometry,
        x: f32,
        tracked: bool,
    ) -> OpenBox {
        match style.break_before {
            Break::Page => self.force_break = true,
            Break::Avoid => self.keep_next = true,
            Break::Auto => {}
        }
        self.add_margin(geometry.margin.top);
        self.edges += geometry.horizontal_edges();

        let paint = if self.is_measuring() {
            None
        } else {
            BoxPaint::from_style(style).or(tracked.then_some(BoxPaint::INVISIBLE))
        };
        let decoration = paint.map(|paint| {
            self.canvas.decorations.push(Decoration {
                top: self.y,
                bottom: self.y,
                x: x + geometry.border_box_x(),
                width: geometry.border_box_width(),
                paint,
            });
            self.canvas.decorations.len() - 1
        });

        if geometry.top_edge() > 0.0 {
            // A border or padding keeps the box's margin from collapsing
            // with its children's.
            self.settle_margin();
            if let Some(index) = decoration {
                self.canvas.decorations[index].top = self.y;
            }
            self.y += f64::from(geometry.top_edge());
        } else if let Some(index) = decoration {
            self.unplaced.push(index);
        }
        OpenBox {
            decoration,
            first_slice: self.canvas.slices.len(),
        }
    }

    /// Makes the content of the box whose decoration is `index` at least
    /// `height` tall, with an empty slice below the content if it falls
    /// short. The slice keeps the space together on one page, and moves an
    /// empty box (a signature field) to the next page whole if it does
    /// not fit.
    fn extend_to(&mut self, index: usize, top_edge: f32, height: f32) {
        // The last child's bottom margin stays inside a box with a height.
        // Settling also places a box that is still empty.
        self.settle_margin();
        let content_top = self.canvas.decorations[index].top + f64::from(top_edge);
        let missing = content_top + f64::from(height) - self.y;
        if missing > 0.0 {
            self.push_slice(self.y, missing as f32, Vec::new());
            self.y += missing;
        }
    }

    /// Ends a box: bottom padding, border and margin.
    fn close_box(&mut self, style: &ComputedStyle, geometry: &BoxGeometry, open: OpenBox) {
        if geometry.bottom_edge() > 0.0 {
            self.settle_margin();
            self.y += f64::from(geometry.bottom_edge());
        }
        if let Some(index) = open.decoration {
            // Still unplaced: the box is empty and has no height.
            if let Some(position) = self.unplaced.iter().position(|&i| i == index) {
                self.unplaced.remove(position);
                self.canvas.decorations[index].top = self.y;
            }
            self.canvas.decorations[index].bottom = self.y;
        }
        if style.break_inside == BreakInside::Avoid {
            let inner = (open.first_slice + 1).min(self.canvas.slices.len());
            for slice in &mut self.canvas.slices[inner..] {
                slice.keep.inside = true;
            }
        }
        self.edges -= geometry.horizontal_edges();
        self.add_margin(geometry.margin.bottom);
        match style.break_after {
            Break::Page => self.force_break = true,
            Break::Avoid => self.keep_next = true,
            Break::Auto => {}
        }
    }

    /// Lays out the children of the block container `id`, whose content
    /// box starts at `x` and is `width` wide.
    fn block_contents(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        if style.display == Display::ListItem {
            self.marker = inline::list_marker(self.cx.doc, id, style);
        }
        let mut inline = Inline::first();
        self.children(id, style, x, width, &mut inline);
        self.paragraph(inline, style, x, width);
        // An item without text has nowhere to put its marker.
        self.marker = None;
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
        let doc = self.cx.doc;
        for child in doc.children(id) {
            match &doc.node(child).kind {
                NodeKind::Text(text) => inline.push_text(text, style),
                NodeKind::Element(element) => {
                    let child_style = self.cx.styles.style(doc, child, Some(style));
                    match child_style.display {
                        Display::None => {}
                        Display::Inline if element.name == "br" => inline.push_break(&child_style),
                        Display::Inline | Display::InlineBlock
                            if replaced::is_image(doc, child) =>
                        {
                            self.inline_image(child, &child_style, width, inline);
                        }
                        Display::Inline => {
                            inline.open_element(child, &child_style, width);
                            self.children(child, &child_style, x, width, inline);
                            inline.close_element(child);
                        }
                        Display::InlineBlock => {
                            self.inline_block(child, &child_style, width, inline);
                        }
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
}
