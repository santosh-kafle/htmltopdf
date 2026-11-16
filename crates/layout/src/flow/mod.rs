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

mod boxes;
mod fonts;
mod inline;
mod replaced;
mod table;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parley::{FontContext, LayoutContext};
use presswork_dom::resource::{LoadError, Loader, display_url};
use presswork_dom::{Document, NodeId, NodeKind};
use presswork_style::{Break, Color, ComputedStyle, Dimension, Display, Side, Stylist};

use crate::LoadFailure;
use crate::fragment::{Fill, ImageData, Item};
use crate::geometry::Rect;
use boxes::BoxGeometry;
pub(crate) use fonts::GenericFonts;
pub(crate) use inline::Ink;
use inline::Inline;

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
    /// Table header to repeat above this slice when it starts a page.
    pub header: Option<Rc<RepeatedHeader>>,
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

    /// Whether the border on `side` is drawn.
    pub fn has_border(&self, side: Side) -> bool {
        self.border
            .iter()
            .any(|(s, width, color)| *s == side && *width > 0.0 && !color.is_transparent())
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
    pub loader: &'a dyn Loader,
    /// Images by URL, `None` for those that failed, so each loads once.
    pub images: HashMap<String, Option<Arc<ImageData>>>,
    pub failures: Vec<LoadFailure>,
}

impl Context<'_> {
    /// The image at `url`, loaded on first use. Failures are recorded in
    /// [`Context::failures`].
    fn image(&mut self, url: &str) -> Option<Arc<ImageData>> {
        let url = url.trim();
        if url.is_empty() {
            // Browsers do not load an empty `src` either.
            return None;
        }
        if let Some(image) = self.images.get(url) {
            return image.clone();
        }
        let image = self
            .loader
            .load(url)
            .and_then(|bytes| ImageData::new(bytes, url).map_err(LoadError::Failed));
        let image = match image {
            Ok(image) => Some(Arc::new(image)),
            Err(error) => {
                self.failures.push(LoadFailure {
                    url: display_url(url).into_owned(),
                    error,
                });
                None
            }
        };
        self.images.insert(url.to_owned(), image.clone());
        image
    }
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
}

/// A box entered with [`Flow::open_box`].
struct OpenBox {
    decoration: Option<usize>,
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
        }
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
            header: None,
        });
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
        let open = self.open_box(style, &geometry, x);
        self.block_contents(id, style, x + geometry.content_x(), geometry.content_width);
        self.close_box(style, &geometry, open);
    }

    /// Starts a box: top margin, border and padding, and its decoration.
    fn open_box(&mut self, style: &ComputedStyle, geometry: &BoxGeometry, x: f32) -> OpenBox {
        if style.break_before == Break::Page {
            self.force_break = true;
        }
        self.add_margin(geometry.margin.top);
        self.edges += geometry.horizontal_edges();

        let paint = if self.is_measuring() {
            None
        } else {
            BoxPaint::from_style(style)
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
        OpenBox { decoration }
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
        self.edges -= geometry.horizontal_edges();
        self.add_margin(geometry.margin.bottom);
        if style.break_after == Break::Page {
            self.force_break = true;
        }
    }

    /// Lays out the children of the block container `id`, whose content
    /// box starts at `x` and is `width` wide.
    fn block_contents(&mut self, id: NodeId, style: &ComputedStyle, x: f32, width: f32) {
        if style.display == Display::ListItem {
            self.marker = inline::list_marker(self.cx.doc, id, style);
        }
        let mut inline = Inline::default();
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
                        Display::Inline if replaced::is_image(doc, child) => {
                            self.inline_image(child, &child_style, width, inline);
                        }
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
}
