//! Page geometry: wkhtmltopdf's page options combined with `@page` rules.

use clap::Args;
use presswork_layout::{DEFAULT_MARGIN, EdgeSizes, LayoutOptions, Orientation, PageSize, Size};
use presswork_style::{Length, PageRule, PageSizeValue, Side};

/// The page options of the command line.
#[derive(Debug, Args)]
pub(crate) struct PageArgs {
    /// Page size: A0-A9, B0-B10, C5E, Comm10E, DLE, Executive, Folio,
    /// Ledger, Legal, Letter or Tabloid [default: A4]
    #[arg(short = 's', long)]
    pub page_size: Option<PageSize>,

    /// Page orientation: Portrait or Landscape [default: Portrait]
    #[arg(short = 'O', long)]
    pub orientation: Option<Orientation>,

    /// Page width, e.g. 210mm. Overrides the page size
    #[arg(long, value_name = "LENGTH", value_parser = parse_length)]
    pub page_width: Option<f32>,

    /// Page height, e.g. 297mm. Overrides the page size
    #[arg(long, value_name = "LENGTH", value_parser = parse_length)]
    pub page_height: Option<f32>,

    /// Top page margin, e.g. 10mm, 0.75in [default: 10mm]
    #[arg(short = 'T', long, value_name = "LENGTH", value_parser = parse_length)]
    pub margin_top: Option<f32>,

    /// Right page margin [default: 10mm]
    #[arg(short = 'R', long, value_name = "LENGTH", value_parser = parse_length)]
    pub margin_right: Option<f32>,

    /// Bottom page margin [default: 10mm]
    #[arg(short = 'B', long, value_name = "LENGTH", value_parser = parse_length)]
    pub margin_bottom: Option<f32>,

    /// Left page margin [default: 10mm]
    #[arg(short = 'L', long, value_name = "LENGTH", value_parser = parse_length)]
    pub margin_left: Option<f32>,
}

/// Parses a wkhtmltopdf length such as `10mm`, `0.75in`, `2cm` or `12pt`
/// into points. A bare number is in millimetres.
pub(crate) fn parse_length(value: &str) -> Result<f32, String> {
    let value = value.trim();
    let split = value
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: f32 = number
        .trim()
        .parse()
        .map_err(|_| format!("`{value}` is not a length"))?;
    if !number.is_finite() || number < 0.0 {
        return Err(format!("`{value}` is not a non-negative length"));
    }
    let unit = if unit.is_empty() { "mm" } else { unit };
    match Length::from_unit(number, unit) {
        Some(Length::Pt(points)) => Ok(points),
        _ => Err(format!("unknown unit `{unit}` (use mm, cm, in, pt or px)")),
    }
}

impl PageArgs {
    /// The page geometry to lay out with.
    ///
    /// Command-line options win over the document's `@page` rule, which
    /// wins over the defaults (A4 portrait, 10mm margins), so existing
    /// wkhtmltopdf invocations keep their meaning.
    pub fn layout_options(&self, page: &PageRule) -> LayoutOptions {
        let css = page.size.as_ref();
        let orientation = self
            .orientation
            .or(css.and_then(|size| size.landscape).map(|landscape| {
                if landscape {
                    Orientation::Landscape
                } else {
                    Orientation::Portrait
                }
            }))
            .unwrap_or_default();

        let mut page_size = match (self.page_size, css) {
            (Some(named), _) => named.size(orientation),
            (None, Some(size)) => css_page_size(size, orientation),
            (None, None) => PageSize::default().size(orientation),
        };
        if let Some(width) = self.page_width {
            page_size.width = width;
        }
        if let Some(height) = self.page_height {
            page_size.height = height;
        }

        let margin =
            |option: Option<f32>, side| option.or(page.margin.get(side)).unwrap_or(DEFAULT_MARGIN);
        LayoutOptions {
            page_size,
            margins: EdgeSizes {
                top: margin(self.margin_top, Side::Top),
                right: margin(self.margin_right, Side::Right),
                bottom: margin(self.margin_bottom, Side::Bottom),
                left: margin(self.margin_left, Side::Left),
            },
        }
    }
}

/// The page size an `@page { size }` descriptor asks for.
fn css_page_size(size: &PageSizeValue, orientation: Orientation) -> Size {
    if let Some((width, height)) = size.dimensions {
        return Size { width, height };
    }
    size.name
        .as_deref()
        .and_then(|name| name.parse::<PageSize>().ok())
        .unwrap_or_default()
        .size(orientation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use presswork_style::Stylist;

    fn args() -> PageArgs {
        PageArgs {
            page_size: None,
            orientation: None,
            page_width: None,
            page_height: None,
            margin_top: None,
            margin_right: None,
            margin_bottom: None,
            margin_left: None,
        }
    }

    fn page_rule(css: &str) -> PageRule {
        let mut styles = Stylist::new();
        styles.add_stylesheet(css);
        styles.page().clone()
    }

    #[test]
    fn parses_wkhtmltopdf_lengths() {
        assert_eq!(parse_length("72pt"), Ok(72.0));
        assert_eq!(parse_length("1in"), Ok(72.0));
        assert_eq!(parse_length("25.4mm"), Ok(72.0));
        assert_eq!(
            parse_length("25.4"),
            Ok(72.0),
            "bare numbers are millimetres"
        );
        assert_eq!(parse_length("2.54cm"), Ok(72.0));
        assert_eq!(parse_length("0"), Ok(0.0));
        assert!(parse_length("-5mm").is_err());
        assert!(parse_length("2em").is_err());
        assert!(parse_length("wide").is_err());
    }

    #[test]
    fn defaults_match_wkhtmltopdf() {
        let options = args().layout_options(&PageRule::default());
        assert_eq!(options, LayoutOptions::default());
    }

    #[test]
    fn page_rule_sets_size_and_margins() {
        let rule = page_rule("@page { size: letter landscape; margin: 1in 0.5in }");
        let options = args().layout_options(&rule);
        assert_eq!(
            options.page_size,
            Size {
                width: 792.0,
                height: 612.0
            }
        );
        assert_eq!((options.margins.top, options.margins.left), (72.0, 36.0));

        let rule = page_rule("@page { size: 100pt 200pt }");
        assert_eq!(
            args().layout_options(&rule).page_size,
            Size {
                width: 100.0,
                height: 200.0
            }
        );
    }

    #[test]
    fn command_line_wins_over_page_rule() {
        let rule = page_rule("@page { size: letter landscape; margin: 1in }");
        let cli = PageArgs {
            page_size: Some(PageSize::A5),
            orientation: Some(Orientation::Portrait),
            margin_top: Some(5.0),
            ..args()
        };
        let options = cli.layout_options(&rule);
        assert_eq!(options.page_size, PageSize::A5.size(Orientation::Portrait));
        assert_eq!(options.margins.top, 5.0);
        // Sides not given on the command line still come from @page.
        assert_eq!(options.margins.bottom, 72.0);
    }

    #[test]
    fn explicit_width_and_height_override_the_named_size() {
        let cli = PageArgs {
            page_width: Some(300.0),
            page_height: Some(400.0),
            ..args()
        };
        let options = cli.layout_options(&PageRule::default());
        assert_eq!(
            options.page_size,
            Size {
                width: 300.0,
                height: 400.0
            }
        );
    }
}
