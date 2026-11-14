//! Page sizes and orientation.

use std::str::FromStr;

use crate::geometry::Size;

/// Page orientation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// A named page size. Dimensions are given in portrait orientation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PageSize {
    A3,
    #[default]
    A4,
    A5,
    Letter,
    Legal,
}

impl PageSize {
    /// Page dimensions in points for the given orientation.
    pub fn size(self, orientation: Orientation) -> Size {
        let (width, height) = match self {
            PageSize::A3 => (841.89, 1190.55),
            PageSize::A4 => (595.28, 841.89),
            PageSize::A5 => (419.53, 595.28),
            PageSize::Letter => (612.0, 792.0),
            PageSize::Legal => (612.0, 1008.0),
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

impl FromStr for PageSize {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "a3" => Ok(PageSize::A3),
            "a4" => Ok(PageSize::A4),
            "a5" => Ok(PageSize::A5),
            "letter" => Ok(PageSize::Letter),
            "legal" => Ok(PageSize::Legal),
            other => Err(format!("unknown page size `{other}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_swaps_dimensions() {
        let p = PageSize::Letter.size(Orientation::Portrait);
        let l = PageSize::Letter.size(Orientation::Landscape);
        assert_eq!((p.width, p.height), (l.height, l.width));
    }

    #[test]
    fn parses_case_insensitively() {
        assert_eq!("A4".parse::<PageSize>(), Ok(PageSize::A4));
        assert_eq!("letter".parse::<PageSize>(), Ok(PageSize::Letter));
        assert!("b9".parse::<PageSize>().is_err());
    }
}
