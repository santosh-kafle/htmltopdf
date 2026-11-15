//! CSS value types and their parsers.
//!
//! Specified values ([`Length`], [`Dim`]) are what a declaration says;
//! computed values ([`Dimension`]) have every font-relative unit resolved to
//! points. Parsers return `Err(Invalid)` for anything they do not understand,
//! which makes the cascade drop the declaration, as CSS requires.

use cssparser::{Parser, Token};

/// The error of every value parser: the input is not valid for the
/// property. It carries no detail, because CSS error handling only ever
/// drops the offending declaration or rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid;

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid CSS value")
    }
}

impl std::error::Error for Invalid {}

impl From<cssparser::BasicParseError> for Invalid {
    fn from(_: cssparser::BasicParseError) -> Self {
        Invalid
    }
}

impl<E> From<cssparser::ParseError<E>> for Invalid {
    fn from(_: cssparser::ParseError<E>) -> Self {
        Invalid
    }
}

/// Result of a value parser.
pub type Parsed<T> = Result<T, Invalid>;

/// Points per CSS pixel.
pub const PT_PER_PX: f32 = 0.75;

/// A specified CSS length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// An absolute length in points. All absolute units (`px`, `mm`, `in`,
    /// ...) are converted to points when parsed.
    Pt(f32),
    /// Relative to the element's font size (`em`; `ex` and `ch` are
    /// approximated as 0.5em).
    Em(f32),
    /// Relative to the root element's font size.
    Rem(f32),
}

impl Length {
    pub const ZERO: Length = Length::Pt(0.0);

    /// A length in CSS pixels.
    pub fn px(v: f32) -> Self {
        Length::Pt(v * PT_PER_PX)
    }

    /// Resolves the length to points, given the font sizes in points.
    pub fn to_pt(self, font_size: f32, root_font_size: f32) -> f32 {
        match self {
            Length::Pt(v) => v,
            Length::Em(v) => v * font_size,
            Length::Rem(v) => v * root_font_size,
        }
    }

    /// Builds a length from a number and a unit, e.g. `(10.0, "mm")`.
    pub fn from_unit(value: f32, unit: &str) -> Option<Self> {
        let pt = |per_unit: f32| Some(Length::Pt(value * per_unit));
        match unit.to_ascii_lowercase().as_str() {
            "px" => pt(PT_PER_PX),
            "pt" => pt(1.0),
            "pc" => pt(12.0),
            "in" => pt(72.0),
            "cm" => pt(72.0 / 2.54),
            "mm" => pt(72.0 / 25.4),
            "q" => pt(72.0 / 101.6),
            "em" => Some(Length::Em(value)),
            "ex" | "ch" => Some(Length::Em(value * 0.5)),
            "rem" => Some(Length::Rem(value)),
            _ => None,
        }
    }

    /// Parses a `<length>`. A unitless `0` is allowed.
    pub fn parse(input: &mut Parser<'_>) -> Parsed<Self> {
        match input.next().map_err(Invalid::from)? {
            Token::Dimension { value, unit, .. } => Length::from_unit(*value, unit).ok_or(Invalid),
            Token::Number { value, .. } if *value == 0.0 => Ok(Length::ZERO),
            _ => Err(Invalid),
        }
    }

    /// Parses a `<length>` that must not be negative.
    pub fn parse_non_negative(input: &mut Parser<'_>) -> Parsed<Self> {
        let length = Self::parse(input)?;
        let (Length::Pt(v) | Length::Em(v) | Length::Rem(v)) = length;
        if v < 0.0 { Err(Invalid) } else { Ok(length) }
    }
}

/// A specified `<length-percentage> | auto`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dim {
    Auto,
    Length(Length),
    /// A percentage of the containing block's width, as a fraction (50% is
    /// `0.5`).
    Percent(f32),
}

impl Dim {
    /// Parses a `<length-percentage>`, plus `auto` if `allow_auto`.
    /// Negative values are rejected unless `allow_negative`.
    pub fn parse(input: &mut Parser<'_>, allow_auto: bool, allow_negative: bool) -> Parsed<Self> {
        if allow_auto && input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
            return Ok(Dim::Auto);
        }
        if let Ok(fraction) = input.try_parse(|i| i.expect_percentage()) {
            return if fraction < 0.0 && !allow_negative {
                Err(Invalid)
            } else {
                Ok(Dim::Percent(fraction))
            };
        }
        if allow_negative {
            Length::parse(input).map(Dim::Length)
        } else {
            Length::parse_non_negative(input).map(Dim::Length)
        }
    }

    /// Computes the value: resolves font-relative lengths to points.
    pub fn compute(self, font_size: f32, root_font_size: f32) -> Dimension {
        match self {
            Dim::Auto => Dimension::Auto,
            Dim::Length(l) => Dimension::Pt(l.to_pt(font_size, root_font_size)),
            Dim::Percent(p) => Dimension::Percent(p),
        }
    }
}

/// A computed `<length-percentage> | auto`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Dimension {
    #[default]
    Auto,
    /// Points.
    Pt(f32),
    /// A fraction of the containing block's width.
    Percent(f32),
}

impl Dimension {
    pub const ZERO: Dimension = Dimension::Pt(0.0);

    /// The used value in points, or `None` for `auto`. Percentages resolve
    /// against `basis`.
    pub fn resolve(self, basis: f32) -> Option<f32> {
        match self {
            Dimension::Auto => None,
            Dimension::Pt(v) => Some(v),
            Dimension::Percent(p) => Some(p * basis),
        }
    }

    pub fn is_auto(self) -> bool {
        self == Dimension::Auto
    }
}

/// The four sides of a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];
}

/// A value for each side of a box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sides<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T: Copy> Sides<T> {
    pub fn uniform(v: T) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub fn get(&self, side: Side) -> T {
        match side {
            Side::Top => self.top,
            Side::Right => self.right,
            Side::Bottom => self.bottom,
            Side::Left => self.left,
        }
    }

    pub fn set(&mut self, side: Side, v: T) {
        match side {
            Side::Top => self.top = v,
            Side::Right => self.right = v,
            Side::Bottom => self.bottom = v,
            Side::Left => self.left = v,
        }
    }

    pub fn map<U>(&self, mut f: impl FnMut(T) -> U) -> Sides<U> {
        Sides {
            top: f(self.top),
            right: f(self.right),
            bottom: f(self.bottom),
            left: f(self.left),
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

    pub fn is_transparent(self) -> bool {
        self.a == 0
    }

    /// Parses `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` without the `#`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        if !hex.is_ascii() {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
        let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        match hex.len() {
            3 | 4 => {
                let a = if hex.len() == 4 { digit(3)? * 17 } else { 255 };
                Some(Color::rgba(
                    digit(0)? * 17,
                    digit(1)? * 17,
                    digit(2)? * 17,
                    a,
                ))
            }
            6 | 8 => {
                let a = if hex.len() == 8 { pair(6)? } else { 255 };
                Some(Color::rgba(pair(0)?, pair(2)?, pair(4)?, a))
            }
            _ => None,
        }
    }

    /// Looks up a CSS named color, case-insensitively.
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        if name == "transparent" {
            return Some(Color::TRANSPARENT);
        }
        NAMED_COLORS
            .binary_search_by(|(n, _)| (*n).cmp(name.as_str()))
            .ok()
            .map(|i| {
                let rgb = NAMED_COLORS[i].1;
                Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
            })
    }

    /// Parses a color from an HTML attribute such as `bgcolor`: a named
    /// color or a hex color with or without `#`.
    pub fn from_attribute(value: &str) -> Option<Self> {
        let value = value.trim();
        Color::from_name(value).or_else(|| Color::from_hex(value.trim_start_matches('#')))
    }
}

/// A specified color: either a concrete color or `currentcolor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorValue {
    CurrentColor,
    Color(Color),
}

impl ColorValue {
    /// Parses a `<color>`: named, hex, `rgb()`/`rgba()`, `hsl()`/`hsla()`,
    /// `transparent` or `currentcolor`.
    pub fn parse(input: &mut Parser<'_>) -> Parsed<Self> {
        let token = input.next().map_err(Invalid::from)?.clone();
        match token {
            Token::Hash(hex) | Token::IDHash(hex) => {
                Color::from_hex(&hex).map(ColorValue::Color).ok_or(Invalid)
            }
            Token::Ident(name) if name.eq_ignore_ascii_case("currentcolor") => {
                Ok(ColorValue::CurrentColor)
            }
            Token::Ident(name) => Color::from_name(&name)
                .map(ColorValue::Color)
                .ok_or(Invalid),
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                input
                    .parse_nested_block(|args| {
                        parse_color_function(&name, args)
                            .map_err(|Invalid| cssparser::ParseError::<Invalid>::custom(Invalid))
                    })
                    .map(ColorValue::Color)
                    .map_err(Invalid::from)
            }
            _ => Err(Invalid),
        }
    }

    /// The concrete color, with `currentcolor` replaced by `current`.
    pub fn resolve(self, current: Color) -> Color {
        match self {
            ColorValue::CurrentColor => current,
            ColorValue::Color(c) => c,
        }
    }
}

/// Parses the arguments of `rgb()`, `rgba()`, `hsl()` or `hsla()`, in both
/// the comma-separated and the space-separated (`/ alpha`) syntax.
fn parse_color_function(name: &str, input: &mut Parser<'_>) -> Parsed<Color> {
    // A number or percentage; percentages are returned as fractions.
    #[derive(Clone, Copy)]
    enum Component {
        Number(f32),
        Percent(f32),
    }
    fn component(input: &mut Parser<'_>) -> Parsed<Component> {
        match input.next().map_err(Invalid::from)? {
            Token::Number { value, .. } => Ok(Component::Number(*value)),
            Token::Percentage { unit_value, .. } => Ok(Component::Percent(*unit_value)),
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("deg") => {
                Ok(Component::Number(*value))
            }
            _ => Err(Invalid),
        }
    }

    let first = component(input)?;
    let commas = input.try_parse(|i| i.expect_comma()).is_ok();
    let second = component(input)?;
    if commas {
        input.expect_comma().map_err(Invalid::from)?;
    }
    let third = component(input)?;
    let alpha = if input.is_exhausted() {
        1.0
    } else {
        if commas {
            input.expect_comma().map_err(Invalid::from)?;
        } else {
            input.expect_delim('/').map_err(Invalid::from)?;
        }
        match component(input)? {
            Component::Number(v) | Component::Percent(v) => v.clamp(0.0, 1.0),
        }
    };
    input.expect_exhausted().map_err(Invalid::from)?;

    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = match name {
        "rgb" | "rgba" => {
            let channel = |c| match c {
                Component::Number(v) => v / 255.0,
                Component::Percent(v) => v,
            };
            (channel(first), channel(second), channel(third))
        }
        "hsl" | "hsla" => {
            let (Component::Number(hue), Component::Percent(s), Component::Percent(l)) =
                (first, second, third)
            else {
                return Err(Invalid);
            };
            hsl_to_rgb(hue, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
        }
        _ => return Err(Invalid),
    };
    Ok(Color::rgba(byte(r), byte(g), byte(b), byte(alpha)))
}

fn hsl_to_rgb(hue: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = hue.rem_euclid(360.0) / 30.0;
    let a = s * l.min(1.0 - l);
    let f = |n: f32| {
        let k = (n + h) % 12.0;
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    (f(0.0), f(8.0), f(4.0))
}

/// The CSS named colors, sorted by name for binary search.
#[rustfmt::skip]
const NAMED_COLORS: &[(&str, u32)] = &[
    ("aliceblue", 0xF0F8FF), ("antiquewhite", 0xFAEBD7), ("aqua", 0x00FFFF),
    ("aquamarine", 0x7FFFD4), ("azure", 0xF0FFFF), ("beige", 0xF5F5DC),
    ("bisque", 0xFFE4C4), ("black", 0x000000), ("blanchedalmond", 0xFFEBCD),
    ("blue", 0x0000FF), ("blueviolet", 0x8A2BE2), ("brown", 0xA52A2A),
    ("burlywood", 0xDEB887), ("cadetblue", 0x5F9EA0), ("chartreuse", 0x7FFF00),
    ("chocolate", 0xD2691E), ("coral", 0xFF7F50), ("cornflowerblue", 0x6495ED),
    ("cornsilk", 0xFFF8DC), ("crimson", 0xDC143C), ("cyan", 0x00FFFF),
    ("darkblue", 0x00008B), ("darkcyan", 0x008B8B), ("darkgoldenrod", 0xB8860B),
    ("darkgray", 0xA9A9A9), ("darkgreen", 0x006400), ("darkgrey", 0xA9A9A9),
    ("darkkhaki", 0xBDB76B), ("darkmagenta", 0x8B008B), ("darkolivegreen", 0x556B2F),
    ("darkorange", 0xFF8C00), ("darkorchid", 0x9932CC), ("darkred", 0x8B0000),
    ("darksalmon", 0xE9967A), ("darkseagreen", 0x8FBC8F), ("darkslateblue", 0x483D8B),
    ("darkslategray", 0x2F4F4F), ("darkslategrey", 0x2F4F4F), ("darkturquoise", 0x00CED1),
    ("darkviolet", 0x9400D3), ("deeppink", 0xFF1493), ("deepskyblue", 0x00BFFF),
    ("dimgray", 0x696969), ("dimgrey", 0x696969), ("dodgerblue", 0x1E90FF),
    ("firebrick", 0xB22222), ("floralwhite", 0xFFFAF0), ("forestgreen", 0x228B22),
    ("fuchsia", 0xFF00FF), ("gainsboro", 0xDCDCDC), ("ghostwhite", 0xF8F8FF),
    ("gold", 0xFFD700), ("goldenrod", 0xDAA520), ("gray", 0x808080),
    ("green", 0x008000), ("greenyellow", 0xADFF2F), ("grey", 0x808080),
    ("honeydew", 0xF0FFF0), ("hotpink", 0xFF69B4), ("indianred", 0xCD5C5C),
    ("indigo", 0x4B0082), ("ivory", 0xFFFFF0), ("khaki", 0xF0E68C),
    ("lavender", 0xE6E6FA), ("lavenderblush", 0xFFF0F5), ("lawngreen", 0x7CFC00),
    ("lemonchiffon", 0xFFFACD), ("lightblue", 0xADD8E6), ("lightcoral", 0xF08080),
    ("lightcyan", 0xE0FFFF), ("lightgoldenrodyellow", 0xFAFAD2), ("lightgray", 0xD3D3D3),
    ("lightgreen", 0x90EE90), ("lightgrey", 0xD3D3D3), ("lightpink", 0xFFB6C1),
    ("lightsalmon", 0xFFA07A), ("lightseagreen", 0x20B2AA), ("lightskyblue", 0x87CEFA),
    ("lightslategray", 0x778899), ("lightslategrey", 0x778899), ("lightsteelblue", 0xB0C4DE),
    ("lightyellow", 0xFFFFE0), ("lime", 0x00FF00), ("limegreen", 0x32CD32),
    ("linen", 0xFAF0E6), ("magenta", 0xFF00FF), ("maroon", 0x800000),
    ("mediumaquamarine", 0x66CDAA), ("mediumblue", 0x0000CD), ("mediumorchid", 0xBA55D3),
    ("mediumpurple", 0x9370DB), ("mediumseagreen", 0x3CB371), ("mediumslateblue", 0x7B68EE),
    ("mediumspringgreen", 0x00FA9A), ("mediumturquoise", 0x48D1CC), ("mediumvioletred", 0xC71585),
    ("midnightblue", 0x191970), ("mintcream", 0xF5FFFA), ("mistyrose", 0xFFE4E1),
    ("moccasin", 0xFFE4B5), ("navajowhite", 0xFFDEAD), ("navy", 0x000080),
    ("oldlace", 0xFDF5E6), ("olive", 0x808000), ("olivedrab", 0x6B8E23),
    ("orange", 0xFFA500), ("orangered", 0xFF4500), ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA), ("palegreen", 0x98FB98), ("paleturquoise", 0xAFEEEE),
    ("palevioletred", 0xDB7093), ("papayawhip", 0xFFEFD5), ("peachpuff", 0xFFDAB9),
    ("peru", 0xCD853F), ("pink", 0xFFC0CB), ("plum", 0xDDA0DD),
    ("powderblue", 0xB0E0E6), ("purple", 0x800080), ("rebeccapurple", 0x663399),
    ("red", 0xFF0000), ("rosybrown", 0xBC8F8F), ("royalblue", 0x4169E1),
    ("saddlebrown", 0x8B4513), ("salmon", 0xFA8072), ("sandybrown", 0xF4A460),
    ("seagreen", 0x2E8B57), ("seashell", 0xFFF5EE), ("sienna", 0xA0522D),
    ("silver", 0xC0C0C0), ("skyblue", 0x87CEEB), ("slateblue", 0x6A5ACD),
    ("slategray", 0x708090), ("slategrey", 0x708090), ("snow", 0xFFFAFA),
    ("springgreen", 0x00FF7F), ("steelblue", 0x4682B4), ("tan", 0xD2B48C),
    ("teal", 0x008080), ("thistle", 0xD8BFD8), ("tomato", 0xFF6347),
    ("turquoise", 0x40E0D0), ("violet", 0xEE82EE), ("wheat", 0xF5DEB3),
    ("white", 0xFFFFFF), ("whitesmoke", 0xF5F5F5), ("yellow", 0xFFFF00),
    ("yellowgreen", 0x9ACD32),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<T>(css: &str, f: impl FnOnce(&mut Parser<'_>) -> Parsed<T>) -> Parsed<T> {
        let mut parser = Parser::new(css);
        let value = f(&mut parser)?;
        parser.expect_exhausted().map_err(Invalid::from)?;
        Ok(value)
    }

    fn color(css: &str) -> Parsed<Color> {
        parse(css, ColorValue::parse).map(|c| c.resolve(Color::BLACK))
    }

    #[test]
    fn absolute_units_convert_to_points() {
        assert_eq!(parse("16px", Length::parse), Ok(Length::Pt(12.0)));
        assert_eq!(parse("1in", Length::parse), Ok(Length::Pt(72.0)));
        assert_eq!(parse("25.4mm", Length::parse), Ok(Length::Pt(72.0)));
        assert_eq!(parse("2pc", Length::parse), Ok(Length::Pt(24.0)));
        assert_eq!(parse("0", Length::parse), Ok(Length::ZERO));
        assert!(parse("5", Length::parse).is_err());
        assert!(parse("5vw", Length::parse).is_err());
    }

    #[test]
    fn relative_lengths_resolve() {
        assert_eq!(Length::Em(2.0).to_pt(10.0, 12.0), 20.0);
        assert_eq!(Length::Rem(2.0).to_pt(10.0, 12.0), 24.0);
        assert_eq!(
            Dim::Percent(0.5).compute(10.0, 12.0).resolve(400.0),
            Some(200.0)
        );
        assert_eq!(Dim::Auto.compute(10.0, 12.0).resolve(400.0), None);
    }

    #[test]
    fn dim_respects_auto_and_sign_rules() {
        assert_eq!(parse("auto", |i| Dim::parse(i, true, false)), Ok(Dim::Auto));
        assert!(parse("auto", |i| Dim::parse(i, false, false)).is_err());
        assert_eq!(
            parse("50%", |i| Dim::parse(i, false, false)),
            Ok(Dim::Percent(0.5))
        );
        assert!(parse("-1px", |i| Dim::parse(i, false, false)).is_err());
        assert_eq!(
            parse("-1pt", |i| Dim::parse(i, true, true)),
            Ok(Dim::Length(Length::Pt(-1.0)))
        );
    }

    #[test]
    fn parses_colors() {
        assert_eq!(color("#f00"), Ok(Color::rgb(255, 0, 0)));
        assert_eq!(color("#00ff0080"), Ok(Color::rgba(0, 255, 0, 128)));
        assert_eq!(color("RebeccaPurple"), Ok(Color::rgb(0x66, 0x33, 0x99)));
        assert_eq!(color("rgb(255, 128, 0)"), Ok(Color::rgb(255, 128, 0)));
        assert_eq!(color("rgba(0,0,0,0.5)"), Ok(Color::rgba(0, 0, 0, 128)));
        assert_eq!(
            color("rgb(100% 0% 0% / 50%)"),
            Ok(Color::rgba(255, 0, 0, 128))
        );
        assert_eq!(color("hsl(120, 100%, 50%)"), Ok(Color::rgb(0, 255, 0)));
        assert_eq!(color("transparent"), Ok(Color::TRANSPARENT));
        assert!(color("#12").is_err());
        assert!(color("notacolor").is_err());
    }

    #[test]
    fn named_colors_are_sorted() {
        assert!(NAMED_COLORS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn attribute_colors_allow_missing_hash() {
        assert_eq!(Color::from_attribute("ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(
            Color::from_attribute(" #EEE "),
            Some(Color::rgb(238, 238, 238))
        );
        assert_eq!(
            Color::from_attribute("Silver"),
            Some(Color::rgb(192, 192, 192))
        );
    }
}
