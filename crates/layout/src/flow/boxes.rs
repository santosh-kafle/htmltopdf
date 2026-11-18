//! Box model: resolving margins, borders, padding and width.

use presswork_style::{BoxSizing, ComputedStyle, Dimension, Sides};

/// The used horizontal geometry and vertical edge sizes of a block box,
/// in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoxGeometry {
    pub margin: Sides<f32>,
    pub border: Sides<f32>,
    pub padding: Sides<f32>,
    pub content_width: f32,
}

impl BoxGeometry {
    /// Resolves the box of `style` inside a containing block that is
    /// `containing_width` wide (CSS 2.1 section 10.3.3).
    ///
    /// `shrink_to_fit` is the content width of a box that sizes itself to
    /// its contents (a table); otherwise the `width` property decides, and
    /// `auto` fills the containing block.
    pub fn resolve(
        style: &ComputedStyle,
        containing_width: f32,
        shrink_to_fit: Option<f32>,
    ) -> Self {
        let cb = containing_width;
        let border = style.border_widths();
        let padding = style.padding.map(|p| p.resolve(cb).unwrap_or(0.0).max(0.0));
        let edges = border.left + border.right + padding.left + padding.right;

        let margin_left = style.margin.left.resolve(cb);
        let margin_right = style.margin.right.resolve(cb);
        let mut width = shrink_to_fit.or_else(|| Self::specified_content_width(style, cb, edges));
        if let Some(max) = Self::content_size(style.max_width, style, cb, edges) {
            // An auto width fills what the margins leave.
            let fill = cb - edges - margin_left.unwrap_or(0.0) - margin_right.unwrap_or(0.0);
            if width.unwrap_or(fill) > max {
                width = Some(max);
            }
        }

        let (content_width, left, right) = match width {
            None => {
                let (left, right) = (margin_left.unwrap_or(0.0), margin_right.unwrap_or(0.0));
                ((cb - edges - left - right).max(0.0), left, right)
            }
            Some(width) => {
                let fixed = margin_left.unwrap_or(0.0) + margin_right.unwrap_or(0.0);
                let free = (cb - edges - width - fixed).max(0.0);
                let (left, right) = match (margin_left, margin_right) {
                    // Both auto: centered.
                    (None, None) => (free / 2.0, free / 2.0),
                    (None, Some(right)) => (free, right),
                    (Some(left), right) => (left, right.unwrap_or(0.0)),
                };
                (width, left, right)
            }
        };

        Self {
            margin: Sides {
                top: style.margin.top.resolve(cb).unwrap_or(0.0),
                right,
                bottom: style.margin.bottom.resolve(cb).unwrap_or(0.0),
                left,
            },
            border,
            padding,
            content_width,
        }
    }

    /// The content width the `width` property asks for, if not `auto`.
    /// `edges` is the horizontal border plus padding.
    pub fn specified_content_width(style: &ComputedStyle, cb: f32, edges: f32) -> Option<f32> {
        Self::content_size(style.width, style, cb, edges)
    }

    /// A `width`-like property's value as a content size, if not `auto`:
    /// percentages resolve against `basis`, and under `box-sizing:
    /// border-box` the `edges` (border plus padding) are taken off.
    pub fn content_size(
        value: Dimension,
        style: &ComputedStyle,
        basis: f32,
        edges: f32,
    ) -> Option<f32> {
        let size = value.resolve(basis)?;
        Some(match style.box_sizing {
            BoxSizing::ContentBox => size,
            BoxSizing::BorderBox => (size - edges).max(0.0),
        })
    }

    /// Horizontal offset from the containing block's left edge to the
    /// border box.
    pub fn border_box_x(&self) -> f32 {
        self.margin.left
    }

    /// Horizontal offset from the containing block's left edge to the
    /// content box.
    pub fn content_x(&self) -> f32 {
        self.margin.left + self.border.left + self.padding.left
    }

    pub fn border_box_width(&self) -> f32 {
        self.border.left
            + self.padding.left
            + self.content_width
            + self.padding.right
            + self.border.right
    }

    /// Everything beside the content, horizontally: margins, borders and
    /// padding on both sides.
    pub fn horizontal_edges(&self) -> f32 {
        self.margin.left + self.margin.right + self.border_box_width() - self.content_width
    }

    /// The least height of the content box that `height`, `min-height`
    /// and `max-height` ask for, in points; zero if none of them does.
    ///
    /// A block is never cut shorter than its content: the result is a
    /// minimum, and taller content makes the box grow rather than
    /// overflow. Percentages count as `auto`, as no containing block has a
    /// definite height.
    pub fn min_content_height(&self, style: &ComputedStyle) -> f32 {
        let edges = self.top_edge() + self.bottom_edge();
        let content = |value: Dimension| match value {
            Dimension::Pt(size) => Some(match style.box_sizing {
                BoxSizing::ContentBox => size,
                BoxSizing::BorderBox => (size - edges).max(0.0),
            }),
            Dimension::Auto | Dimension::Percent(_) => None,
        };
        let max = content(style.max_height).unwrap_or(f32::INFINITY);
        let height = content(style.height).map_or(0.0, |height| height.min(max));
        // `min-height` wins over `max-height`.
        height.max(content(style.min_height).unwrap_or(0.0))
    }

    pub fn top_edge(&self) -> f32 {
        self.border.top + self.padding.top
    }

    pub fn bottom_edge(&self) -> f32 {
        self.border.bottom + self.padding.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presswork_style::{BorderStyle, Dimension};

    fn style() -> ComputedStyle {
        ComputedStyle::initial()
    }

    #[test]
    fn auto_width_fills_the_containing_block() {
        let mut s = style();
        s.margin.left = Dimension::Pt(10.0);
        s.padding.right = Dimension::Percent(0.1);
        s.border.left.style = BorderStyle::Solid;
        s.border.left.width = 2.0;
        let g = BoxGeometry::resolve(&s, 200.0, None);
        assert_eq!(g.content_width, 200.0 - 10.0 - 20.0 - 2.0);
        assert_eq!(g.content_x(), 12.0);
        assert_eq!(g.border_box_width(), 190.0);
        assert_eq!(g.horizontal_edges(), 32.0);
    }

    #[test]
    fn auto_margins_center_a_box_with_a_width() {
        let mut s = style();
        s.width = Dimension::Pt(100.0);
        s.margin.left = Dimension::Auto;
        s.margin.right = Dimension::Auto;
        let g = BoxGeometry::resolve(&s, 300.0, None);
        assert_eq!(
            (g.margin.left, g.margin.right, g.content_width),
            (100.0, 100.0, 100.0)
        );

        s.margin.right = Dimension::Pt(0.0);
        let g = BoxGeometry::resolve(&s, 300.0, None);
        assert_eq!(g.margin.left, 200.0);
    }

    #[test]
    fn border_box_sizing_subtracts_edges() {
        let mut s = style();
        s.width = Dimension::Percent(0.5);
        s.padding.left = Dimension::Pt(10.0);
        s.padding.right = Dimension::Pt(10.0);
        assert_eq!(BoxGeometry::resolve(&s, 200.0, None).content_width, 100.0);
        s.box_sizing = BoxSizing::BorderBox;
        assert_eq!(BoxGeometry::resolve(&s, 200.0, None).content_width, 80.0);
    }

    #[test]
    fn max_width_caps_the_width() {
        let mut s = style();
        s.max_width = Dimension::Percent(0.5);
        assert_eq!(BoxGeometry::resolve(&s, 300.0, None).content_width, 150.0);
        // Centered by auto margins once capped.
        s.margin.left = Dimension::Auto;
        s.margin.right = Dimension::Auto;
        let g = BoxGeometry::resolve(&s, 300.0, None);
        assert_eq!((g.content_width, g.margin.left), (150.0, 75.0));
        // A narrower width is left alone.
        s.width = Dimension::Pt(100.0);
        assert_eq!(BoxGeometry::resolve(&s, 300.0, None).content_width, 100.0);
    }

    #[test]
    fn heights_are_minimums_capped_by_max_height() {
        let mut s = style();
        s.padding.top = Dimension::Pt(5.0);
        s.padding.bottom = Dimension::Pt(5.0);
        let g = |s: &ComputedStyle| BoxGeometry::resolve(s, 300.0, None).min_content_height(s);
        assert_eq!(g(&s), 0.0);
        s.height = Dimension::Pt(50.0);
        assert_eq!(g(&s), 50.0);
        s.box_sizing = BoxSizing::BorderBox;
        assert_eq!(g(&s), 40.0);
        s.max_height = Dimension::Pt(30.0);
        assert_eq!(g(&s), 20.0);
        s.min_height = Dimension::Pt(60.0);
        assert_eq!(g(&s), 50.0);
        s.height = Dimension::Percent(1.0);
        s.min_height = Dimension::Auto;
        assert_eq!(g(&s), 0.0);
    }

    #[test]
    fn shrink_to_fit_width_wins() {
        let mut s = style();
        s.margin.left = Dimension::Auto;
        s.margin.right = Dimension::Auto;
        let g = BoxGeometry::resolve(&s, 300.0, Some(60.0));
        assert_eq!((g.content_width, g.margin.left), (60.0, 120.0));
    }
}
