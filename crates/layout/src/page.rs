//! Page sizes and orientation.

use std::fmt;
use std::str::FromStr;

use crate::geometry::Size;

/// Points per millimetre.
const PT_PER_MM: f32 = 72.0 / 25.4;
/// Points per inch.
const PT_PER_IN: f32 = 72.0;

/// Page orientation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

impl FromStr for Orientation {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "portrait" => Ok(Orientation::Portrait),
            "landscape" => Ok(Orientation::Landscape),
            other => Err(format!(
                "unknown orientation `{other}` (expected Portrait or Landscape)"
            )),
        }
    }
}

/// Paper dimensions in portrait orientation, in their native unit.
#[derive(Clone, Copy)]
enum Dim {
    Mm(f32, f32),
    In(f32, f32),
}

macro_rules! page_sizes {
    ($($(#[$attr:meta])* $variant:ident => $dim:expr),* $(,)?) => {
        /// A named page size, matching the names wkhtmltopdf accepts.
        ///
        /// ISO sizes use ISO 216 dimensions; the remaining names are the
        /// North American and envelope sizes wkhtmltopdf inherits from Qt.
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        pub enum PageSize {
            $(
                #[doc = concat!("`", stringify!($variant), "`")]
                $(#[$attr])*
                $variant,
            )*
        }

        impl PageSize {
            /// Every supported page size, in the order wkhtmltopdf lists them.
            pub const ALL: &[PageSize] = &[$(PageSize::$variant),*];

            /// The canonical name, as accepted on the command line.
            pub const fn name(self) -> &'static str {
                match self {
                    $(PageSize::$variant => stringify!($variant),)*
                }
            }

            const fn dim(self) -> Dim {
                match self {
                    $(PageSize::$variant => $dim,)*
                }
            }
        }
    };
}

page_sizes! {
    A0 => Dim::Mm(841.0, 1189.0),
    A1 => Dim::Mm(594.0, 841.0),
    A2 => Dim::Mm(420.0, 594.0),
    A3 => Dim::Mm(297.0, 420.0),
    #[default]
    A4 => Dim::Mm(210.0, 297.0),
    A5 => Dim::Mm(148.0, 210.0),
    A6 => Dim::Mm(105.0, 148.0),
    A7 => Dim::Mm(74.0, 105.0),
    A8 => Dim::Mm(52.0, 74.0),
    A9 => Dim::Mm(37.0, 52.0),
    B0 => Dim::Mm(1000.0, 1414.0),
    B1 => Dim::Mm(707.0, 1000.0),
    B2 => Dim::Mm(500.0, 707.0),
    B3 => Dim::Mm(353.0, 500.0),
    B4 => Dim::Mm(250.0, 353.0),
    B5 => Dim::Mm(176.0, 250.0),
    B6 => Dim::Mm(125.0, 176.0),
    B7 => Dim::Mm(88.0, 125.0),
    B8 => Dim::Mm(62.0, 88.0),
    B9 => Dim::Mm(44.0, 62.0),
    B10 => Dim::Mm(31.0, 44.0),
    C5E => Dim::Mm(163.0, 229.0),
    Comm10E => Dim::In(4.125, 9.5),
    DLE => Dim::Mm(110.0, 220.0),
    Executive => Dim::In(7.25, 10.5),
    Folio => Dim::Mm(210.0, 330.0),
    Ledger => Dim::In(17.0, 11.0),
    Legal => Dim::In(8.5, 14.0),
    Letter => Dim::In(8.5, 11.0),
    Tabloid => Dim::In(11.0, 17.0),
}

impl PageSize {
    /// Page dimensions in points for the given orientation.
    pub fn size(self, orientation: Orientation) -> Size {
        let (width, height) = match self.dim() {
            Dim::Mm(w, h) => (w * PT_PER_MM, h * PT_PER_MM),
            Dim::In(w, h) => (w * PT_PER_IN, h * PT_PER_IN),
        };
        match orientation {
            Orientation::Portrait => Size { width, height },
            Orientation::Landscape => Size {
                width: height,
                height: width,
            },
        }
    }
}

impl fmt::Display for PageSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for PageSize {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PageSize::ALL
            .iter()
            .copied()
            .find(|size| size.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown page size `{s}`"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn landscape_swaps_dimensions() {
        let p = PageSize::Letter.size(Orientation::Portrait);
        let l = PageSize::Letter.size(Orientation::Landscape);
        assert_eq!((p.width, p.height), (l.height, l.width));
    }

    #[test]
    fn known_dimensions_in_points() {
        let a4 = PageSize::A4.size(Orientation::Portrait);
        assert!(approx(a4.width, 595.28) && approx(a4.height, 841.89));
        let letter = PageSize::Letter.size(Orientation::Portrait);
        assert_eq!((letter.width, letter.height), (612.0, 792.0));
        let legal = PageSize::Legal.size(Orientation::Portrait);
        assert_eq!((legal.width, legal.height), (612.0, 1008.0));
    }

    #[test]
    fn parses_case_insensitively() {
        assert_eq!("A4".parse::<PageSize>(), Ok(PageSize::A4));
        assert_eq!("letter".parse::<PageSize>(), Ok(PageSize::Letter));
        assert_eq!("b9".parse::<PageSize>(), Ok(PageSize::B9));
        assert_eq!("comm10e".parse::<PageSize>(), Ok(PageSize::Comm10E));
        assert!("A10".parse::<PageSize>().is_err());
    }

    #[test]
    fn names_round_trip() {
        for &size in PageSize::ALL {
            assert_eq!(size.name().parse::<PageSize>(), Ok(size));
        }
        assert_eq!(PageSize::ALL.len(), 30);
    }

    #[test]
    fn parses_orientation() {
        assert_eq!("Landscape".parse(), Ok(Orientation::Landscape));
        assert_eq!("portrait".parse(), Ok(Orientation::Portrait));
        assert!("sideways".parse::<Orientation>().is_err());
    }
}
