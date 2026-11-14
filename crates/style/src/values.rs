//! Basic CSS value types.

/// A CSS length before it has been resolved to absolute units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// CSS pixels (1px = 0.75pt).
    Px(f32),
    /// Typographic points.
    Pt(f32),
    /// Relative to the element's font size.
    Em(f32),
    /// Relative to the root element's font size.
    Rem(f32),
    /// Percentage of the containing block (meaning depends on the property).
    Percent(f32),
}

impl Length {
    /// Points per CSS pixel.
    pub const PT_PER_PX: f32 = 0.75;

    /// Resolves the length to points, given the font sizes and the
    /// percentage basis (all in points).
    pub fn to_pt(self, font_size: f32, root_font_size: f32, percent_basis: f32) -> f32 {
        match self {
            Length::Px(v) => v * Self::PT_PER_PX,
            Length::Pt(v) => v,
            Length::Em(v) => v * font_size,
            Length::Rem(v) => v * root_font_size,
            Length::Percent(v) => v / 100.0 * percent_basis,
        }
    }
}

/// An sRGB color with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn px_converts_to_points() {
        assert_eq!(Length::Px(16.0).to_pt(12.0, 12.0, 0.0), 12.0);
    }

    #[test]
    fn relative_lengths_resolve() {
        assert_eq!(Length::Em(2.0).to_pt(10.0, 12.0, 0.0), 20.0);
        assert_eq!(Length::Rem(2.0).to_pt(10.0, 12.0, 0.0), 24.0);
        assert_eq!(Length::Percent(50.0).to_pt(10.0, 12.0, 400.0), 200.0);
    }
}
