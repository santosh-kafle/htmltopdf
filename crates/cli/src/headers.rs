//! wkhtmltopdf's text headers and footers: `--header-left` and friends,
//! with `[page]`-style variables.
//!
//! Matches wkhtmltopdf 0.12.6: the same defaults (12pt Arial, no line, no
//! spacing), the same variables, `--replace` for custom ones, and
//! `--default-header`. Headers do not enlarge the margins; set
//! `--margin-top` and `--margin-bottom` to make room, as with wkhtmltopdf.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use clap::{ArgAction, Args};
use presswork_layout::{Band, BandStyle, BandText, EdgeSizes, Engine, Page};

use crate::page::parse_length;

/// The header and footer options of the command line.
#[derive(Debug, Args)]
pub(crate) struct HeaderArgs {
    /// Left-aligned header text. These variables are replaced: [page],
    /// [frompage], [topage], [sitepage], [sitepages], [title], [doctitle],
    /// [webpage], [date], [isodate], [time], and any set with --replace
    #[arg(long, value_name = "TEXT")]
    pub header_left: Option<String>,

    /// Centered header text
    #[arg(long, value_name = "TEXT")]
    pub header_center: Option<String>,

    /// Right-aligned header text
    #[arg(long, value_name = "TEXT")]
    pub header_right: Option<String>,

    /// Header font, with the default sans-serif font as fallback
    #[arg(long, value_name = "NAME", default_value = "Arial")]
    pub header_font_name: String,

    /// Header font size in points
    #[arg(long, value_name = "SIZE", default_value_t = 12.0, value_parser = parse_font_size)]
    pub header_font_size: f32,

    /// Draw a line below the header
    #[arg(long, overrides_with = "no_header_line")]
    pub header_line: bool,

    /// Do not draw a line below the header
    #[arg(long, overrides_with = "header_line")]
    pub no_header_line: bool,

    /// Space between the header and the content, in mm
    #[arg(
        long,
        value_name = "MM",
        default_value = "0",
        allow_hyphen_values = true,
        value_parser = parse_spacing
    )]
    pub header_spacing: f32,

    /// Left-aligned footer text, with the same variables as --header-left
    #[arg(long, value_name = "TEXT")]
    pub footer_left: Option<String>,

    /// Centered footer text
    #[arg(long, value_name = "TEXT")]
    pub footer_center: Option<String>,

    /// Right-aligned footer text
    #[arg(long, value_name = "TEXT")]
    pub footer_right: Option<String>,

    /// Footer font, with the default sans-serif font as fallback
    #[arg(long, value_name = "NAME", default_value = "Arial")]
    pub footer_font_name: String,

    /// Footer font size in points
    #[arg(long, value_name = "SIZE", default_value_t = 12.0, value_parser = parse_font_size)]
    pub footer_font_size: f32,

    /// Draw a line above the footer
    #[arg(long, overrides_with = "no_footer_line")]
    pub footer_line: bool,

    /// Do not draw a line above the footer
    #[arg(long, overrides_with = "footer_line")]
    pub no_footer_line: bool,

    /// Space between the footer and the content, in mm
    #[arg(
        long,
        value_name = "MM",
        default_value = "0",
        allow_hyphen_values = true,
        value_parser = parse_spacing
    )]
    pub footer_spacing: f32,

    /// Replace [NAME] with VALUE in headers and footers (repeatable)
    #[arg(
        long,
        num_args = 2,
        value_names = ["NAME", "VALUE"],
        action = ArgAction::Append
    )]
    pub replace: Vec<String>,

    /// Number of the first page, minus one: [page] counts from OFFSET + 1
    #[arg(
        long,
        value_name = "OFFSET",
        default_value_t = 0,
        allow_negative_numbers = true
    )]
    pub page_offset: i64,

    /// Header with the input's name on the left and [page]/[topage] on the
    /// right, a line below it, and a 2cm top margin unless --margin-top is
    /// given
    #[arg(long)]
    pub default_header: bool,
}

/// What headers and footers can refer to, apart from page numbers.
#[derive(Debug, Clone, Default)]
pub(crate) struct DocumentInfo {
    /// The input as given on the command line.
    pub webpage: String,
    /// The document's `<title>`.
    pub title: String,
    /// `--title`, or the document's `<title>`.
    pub doctitle: String,
    /// When the conversion runs, in local time.
    pub now: NaiveDateTime,
}

/// The texts of one band, before variables are replaced.
struct Template<'a> {
    left: &'a str,
    center: &'a str,
    right: &'a str,
}

impl HeaderArgs {
    fn header(&self) -> (BandStyle, Template<'_>) {
        let default = self.default_header;
        let style = BandStyle {
            font_family: self.header_font_name.clone(),
            font_size: self.header_font_size,
            line: self.header_line || (default && !self.no_header_line),
            spacing: self.header_spacing,
        };
        let text = Template {
            left: (self.header_left.as_deref()).unwrap_or(if default { "[webpage]" } else { "" }),
            center: self.header_center.as_deref().unwrap_or(""),
            right: (self.header_right.as_deref()).unwrap_or(if default {
                "[page]/[topage]"
            } else {
                ""
            }),
        };
        (style, text)
    }

    fn footer(&self) -> (BandStyle, Template<'_>) {
        let style = BandStyle {
            font_family: self.footer_font_name.clone(),
            font_size: self.footer_font_size,
            line: self.footer_line,
            spacing: self.footer_spacing,
        };
        let text = Template {
            left: self.footer_left.as_deref().unwrap_or(""),
            center: self.footer_center.as_deref().unwrap_or(""),
            right: self.footer_right.as_deref().unwrap_or(""),
        };
        (style, text)
    }

    /// The `--margin-top` that `--default-header` implies: 2cm.
    pub fn default_margin_top(&self) -> Option<f32> {
        self.default_header.then(|| 20.0 * 72.0 / 25.4)
    }

    /// Draws the headers and footers on every page, if there are any.
    pub fn draw(
        &self,
        engine: &mut Engine,
        pages: &mut [Page],
        margins: EdgeSizes,
        info: &DocumentInfo,
    ) {
        let bands = [(Band::Header, self.header()), (Band::Footer, self.footer())];
        let bands: Vec<_> = bands
            .into_iter()
            .filter(|(_, (style, text))| {
                style.line
                    || !(text.left.is_empty() && text.center.is_empty() && text.right.is_empty())
            })
            .collect();
        if bands.is_empty() {
            return;
        }
        let count = pages.len();
        for (index, page) in pages.iter_mut().enumerate() {
            let vars = self.variables(index + 1, count, info);
            for (band, (style, text)) in &bands {
                let (left, center, right) = (
                    substitute(text.left, &vars),
                    substitute(text.center, &vars),
                    substitute(text.right, &vars),
                );
                let text = BandText {
                    left: &left,
                    center: &center,
                    right: &right,
                };
                engine.draw_band(page, *band, style, text, margins);
            }
        }
    }

    /// The variables for page `page` (counting from 1) of `count`, by
    /// lowercase name.
    fn variables(&self, page: usize, count: usize, info: &DocumentInfo) -> HashMap<String, String> {
        let mut vars: HashMap<String, String> = self
            .replace
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[name, value]| (name.to_lowercase(), value.clone()))
            .collect();
        let offset = self.page_offset;
        let number = |n: usize| (offset + n as i64).to_string();
        // Like wkhtmltopdf, the built-in variables win over --replace.
        let builtins = [
            ("page", number(page)),
            ("frompage", number(1)),
            ("topage", number(count)),
            ("sitepage", page.to_string()),
            ("sitepages", count.to_string()),
            ("webpage", info.webpage.clone()),
            ("title", info.title.clone()),
            ("doctitle", info.doctitle.clone()),
            // wkhtmltopdf's formats in the C locale, which servers and
            // containers usually run in.
            ("date", info.now.format("%-d %b %Y").to_string()),
            ("isodate", info.now.format("%Y-%m-%d").to_string()),
            ("time", info.now.format("%H:%M:%S").to_string()),
            // Headings are not tracked yet, so these are always empty, as
            // in a wkhtmltopdf document without headings.
            ("section", String::new()),
            ("subsection", String::new()),
            ("subsubsection", String::new()),
        ];
        vars.extend(builtins.map(|(name, value)| (name.to_owned(), value)));
        vars
    }
}

/// Replaces each `[name]` in `text` whose name (in any case) is in `vars`.
/// Other brackets are left as they are. Values are not themselves
/// searched for variables.
fn substitute(text: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after
            .find(']')
            .and_then(|close| Some((close, vars.get(&after[..close].to_lowercase())?)))
        {
            Some((close, value)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_font_size(value: &str) -> Result<f32, String> {
    match value.trim().parse::<f32>() {
        Ok(size) if size.is_finite() && size > 0.0 => Ok(size),
        _ => Err(format!("`{value}` is not a font size")),
    }
}

/// Parses a spacing in millimetres (or with a unit, as for margins), which
/// may be negative.
fn parse_spacing(value: &str) -> Result<f32, String> {
    let value = value.trim();
    match value.strip_prefix('-') {
        Some(magnitude) => parse_length(magnitude).map(|length| -length),
        None => parse_length(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(flatten)]
        headers: HeaderArgs,
    }

    fn parse(args: &[&str]) -> HeaderArgs {
        Cli::try_parse_from(std::iter::once("presswork").chain(args.iter().copied()))
            .unwrap()
            .headers
    }

    fn info() -> DocumentInfo {
        DocumentInfo {
            webpage: "invoice.html".into(),
            title: "Invoice 42".into(),
            doctitle: "Invoices".into(),
            now: NaiveDateTime::parse_from_str("2026-03-07 09:05:01", "%Y-%m-%d %H:%M:%S").unwrap(),
        }
    }

    fn expand(args: &HeaderArgs, text: &str, page: usize, count: usize) -> String {
        substitute(text, &args.variables(page, count, &info()))
    }

    #[test]
    fn replaces_page_numbers_and_document_variables() {
        let args = parse(&[]);
        assert_eq!(expand(&args, "[page]/[toPage]", 2, 5), "2/5");
        assert_eq!(expand(&args, "[frompage]-[topage]", 2, 5), "1-5");
        assert_eq!(
            expand(&args, "[title] in [doctitle]", 1, 1),
            "Invoice 42 in Invoices"
        );
        assert_eq!(expand(&args, "[webpage]", 1, 1), "invoice.html");
        assert_eq!(
            expand(&args, "[date] [isodate] [time]", 1, 1),
            "7 Mar 2026 2026-03-07 09:05:01"
        );
        assert_eq!(expand(&args, "[section][subsection]", 1, 1), "");
    }

    #[test]
    fn page_offset_shifts_page_numbers_but_not_site_pages() {
        let args = parse(&["--page-offset", "10"]);
        assert_eq!(
            expand(&args, "[page] [frompage] [topage]", 2, 3),
            "12 11 13"
        );
        assert_eq!(expand(&args, "[sitepage]/[sitepages]", 2, 3), "2/3");
        let args = parse(&["--page-offset", "-1"]);
        assert_eq!(expand(&args, "[page]", 1, 3), "0");
    }

    #[test]
    fn replace_adds_variables_but_built_ins_win() {
        let args = parse(&[
            "--replace",
            "Client",
            "ACME [page]",
            "--replace",
            "page",
            "x",
        ]);
        // Values are not expanded again.
        assert_eq!(expand(&args, "[client]: [page]", 3, 4), "ACME [page]: 3");
    }

    #[test]
    fn leaves_unknown_and_unbalanced_brackets_alone() {
        let args = parse(&[]);
        assert_eq!(expand(&args, "[x] [ [page] ]", 1, 1), "[x] [ 1 ]");
        assert_eq!(expand(&args, "a[page", 1, 1), "a[page");
        assert_eq!(expand(&args, "पृष्ठ [page]", 4, 4), "पृष्ठ 4");
    }

    #[test]
    fn defaults_match_wkhtmltopdf() {
        let args = parse(&[]);
        let (style, text) = args.header();
        assert_eq!(
            style,
            BandStyle {
                font_family: "Arial".into(),
                font_size: 12.0,
                line: false,
                spacing: 0.0
            }
        );
        assert_eq!((text.left, text.center, text.right), ("", "", ""));
        assert_eq!(args.default_margin_top(), None);
    }

    #[test]
    fn default_header_shows_the_input_and_page_count() {
        let args = parse(&["--default-header"]);
        let (style, text) = args.header();
        assert!(style.line);
        assert_eq!((text.left, text.right), ("[webpage]", "[page]/[topage]"));
        assert!((args.default_margin_top().unwrap() - 56.69).abs() < 0.01);

        let args = parse(&["--default-header", "--no-header-line", "--header-left", "x"]);
        let (style, text) = args.header();
        assert!(!style.line);
        assert_eq!(text.left, "x");
    }

    #[test]
    fn the_last_line_option_wins() {
        assert!(
            !parse(&["--footer-line", "--no-footer-line"])
                .footer()
                .0
                .line
        );
        assert!(
            parse(&["--no-footer-line", "--footer-line"])
                .footer()
                .0
                .line
        );
    }

    #[test]
    fn parses_sizes_and_spacing() {
        let args = parse(&[
            "--footer-font-size",
            "8",
            "--footer-spacing",
            "-2.5",
            "--header-spacing",
            "5",
        ]);
        assert_eq!(args.footer().0.font_size, 8.0);
        assert!((args.footer().0.spacing + 2.5 * 72.0 / 25.4).abs() < 1e-4);
        assert!((args.header().0.spacing - 5.0 * 72.0 / 25.4).abs() < 1e-4);
        let bad = Cli::try_parse_from(["presswork", "--header-font-size", "0"]);
        assert!(bad.is_err());
    }
}
