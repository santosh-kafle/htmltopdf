//! wkhtmltopdf's headers and footers: text ones (`--header-left` and
//! friends) with `[page]`-style variables, and HTML ones (`--header-html`).
//!
//! Matches wkhtmltopdf 0.12.6: the same defaults (12pt Arial, no line, no
//! spacing), the same variables, `--replace` for custom ones, and
//! `--default-header`. Text headers do not enlarge the margins; set
//! `--margin-top` and `--margin-bottom` to make room, as with wkhtmltopdf.
//!
//! HTML headers set the margin themselves when none is given: their body
//! height plus the spacing. wkhtmltopdf passes the variables to them in the
//! URL for a script to fill in. presswork runs no scripts, so when the
//! document has one, it fills in elements whose class is a variable name,
//! which is what the script in wkhtmltopdf's documentation does.

use std::collections::HashMap;

use anyhow::{Result, bail};
use chrono::NaiveDateTime;
use clap::{ArgAction, Args};
use presswork_dom::Document;
use presswork_dom::resource::display_url;
use presswork_layout::{
    Band, BandStyle, BandText, EdgeSizes, Engine, Fetcher, Item, LayoutOptions, LoadError,
    LoadFailure, Loader, MarginDocument, MarkKind, Page,
};
use presswork_style::{Side, Stylist};
use url::Url;

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

    /// HTML file (path or file: URL) drawn above the content of every page.
    /// Without --margin-top, the margin grows to fit it. Scripts do not run:
    /// if it has one, elements whose class is a variable name (page, topage,
    /// title, ...) get that variable's value, as the script in wkhtmltopdf's
    /// documentation does
    #[arg(long, value_name = "URL")]
    pub header_html: Option<String>,

    /// HTML file drawn below the content of every page, like --header-html
    #[arg(long, value_name = "URL")]
    pub footer_html: Option<String>,
}

/// A `--header-html` or `--footer-html` document.
pub(crate) struct HtmlBand<'a> {
    band: Band,
    doc: Document,
    styles: Stylist,
    /// Loads its images, stylesheets and fonts, relative to its own URL.
    pub fetcher: Fetcher<'a>,
    spacing: f32,
    /// Whether to fill in elements named after variables, page by page:
    /// only if the document has a script, which wkhtmltopdf would run.
    fill: bool,
    images: bool,
    /// Laid out once without variables: sizes the margin, and is what each
    /// page shows when nothing is filled in.
    laid_out: Option<MarginDocument>,
}

/// A user stylesheet's CSS and URL.
pub(crate) type UserStyleSheet = (String, Url);

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
    /// `[section]`, `[subsection]` and `[subsubsection]` of each page.
    pub sections: Vec<[String; 3]>,
}

/// The section, subsection and subsubsection of each page: the titles of
/// the last heading on the page or before it, and of the last headings
/// under it, by outline nesting. Like wkhtmltopdf, which takes them from
/// its outline.
pub(crate) fn sections(pages: &[Page]) -> Vec<[String; 3]> {
    let mut open: Vec<(u8, &str)> = Vec::new();
    pages
        .iter()
        .map(|page| {
            for item in &page.items {
                if let Item::Mark(mark) = item
                    && let MarkKind::Heading { level, title } = &mark.kind
                {
                    while open.last().is_some_and(|(open, _)| open >= level) {
                        open.pop();
                    }
                    open.push((*level, title));
                }
            }
            std::array::from_fn(|depth| {
                open.get(depth)
                    .map_or_else(String::new, |(_, title)| (*title).to_owned())
            })
        })
        .collect()
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

    /// Loads the `--header-html` and `--footer-html` documents, which are
    /// read whatever the file access options say, since whoever runs
    /// presswork named them; what they link to follows those options.
    ///
    /// A document that would need the network is recorded in `failures`
    /// and left out. Returns the documents and any warnings.
    pub fn load_html<'a>(
        &self,
        loader: &'a dyn Loader,
        user_css: Option<&UserStyleSheet>,
        images: bool,
        failures: &mut Vec<LoadFailure>,
    ) -> Result<(Vec<HtmlBand<'a>>, Vec<String>)> {
        let mut bands = Vec::new();
        let mut warnings = Vec::new();
        let sources = [
            (
                Band::Header,
                "--header-html",
                &self.header_html,
                self.header_spacing,
            ),
            (
                Band::Footer,
                "--footer-html",
                &self.footer_html,
                self.footer_spacing,
            ),
        ];
        for (band, option, value, spacing) in sources {
            let Some(value) = value else {
                continue;
            };
            if value.trim_start().starts_with('<') {
                bail!("{option} should be a path or URL, not HTML");
            }
            if value.contains("://") && !value.starts_with("file:") {
                failures.push(LoadFailure {
                    url: display_url(value).into_owned(),
                    error: LoadError::Network,
                });
                continue;
            }
            let (bytes, url) = crate::read_local(value)?;
            let doc = presswork_dom::parse_html(&bytes);
            let fetcher = Fetcher::new(loader, Some(url));
            let mut styles = Stylist::new();
            if let Some((css, url)) = user_css {
                styles.add_user_stylesheet(css, Some(url.clone()), &fetcher);
            }
            styles.add_document(&doc, &fetcher);
            let fill = doc.find_element("script").is_some();
            if fill
                && fill_targets(&doc, &self.variables(1, 1, &DocumentInfo::default())).is_empty()
            {
                warnings.push(format!(
                    "warning: {option} has a script, which presswork does not run; \
                     to show page numbers, give elements a class such as page or topage"
                ));
            }
            bands.push(HtmlBand {
                band,
                doc,
                styles,
                fetcher,
                spacing,
                fill,
                images,
                laid_out: None,
            });
        }
        Ok((bands, warnings))
    }

    /// Lays out the HTML headers and footers once and makes room for them,
    /// as wkhtmltopdf does: a margin given explicitly (`top` and `bottom`,
    /// from the command line or `@page`) grows by the spacing; a missing
    /// one becomes the document's body height plus the spacing.
    pub fn fit_margins(
        bands: &mut [HtmlBand<'_>],
        engine: &mut Engine,
        layout: &mut LayoutOptions,
        top: Option<f32>,
        bottom: Option<f32>,
    ) {
        let margins = &mut layout.margins;
        let width = layout.page_size.width - margins.left - margins.right;
        for band in bands {
            let laid_out = engine.layout_margin_document(
                &band.doc,
                &band.styles,
                width,
                band.images,
                &band.fetcher,
            );
            let (explicit, margin) = match band.band {
                Band::Header => (top, &mut margins.top),
                Band::Footer => (bottom, &mut margins.bottom),
            };
            *margin = (explicit.unwrap_or(laid_out.body_height) + band.spacing).max(0.0);
            band.laid_out = Some(laid_out);
        }
    }

    /// The margins given for the page, if any: on the command line, by
    /// `--default-header`, or by the document's `@page` rule.
    pub fn explicit_margins(
        &self,
        top: Option<f32>,
        bottom: Option<f32>,
        page: &presswork_style::PageRule,
    ) -> (Option<f32>, Option<f32>) {
        (
            top.or(self.default_margin_top())
                .or(page.margin.get(Side::Top)),
            bottom.or(page.margin.get(Side::Bottom)),
        )
    }

    /// The `--margin-top` that `--default-header` implies: 2cm.
    pub fn default_margin_top(&self) -> Option<f32> {
        self.default_header.then(|| 20.0 * 72.0 / 25.4)
    }

    /// Draws the headers and footers on every page, if there are any:
    /// text first, then HTML, as wkhtmltopdf does.
    pub fn draw(
        &self,
        engine: &mut Engine,
        pages: &mut [Page],
        margins: EdgeSizes,
        info: &DocumentInfo,
        html: &[HtmlBand<'_>],
    ) {
        let bands = [(Band::Header, self.header()), (Band::Footer, self.footer())];
        let bands: Vec<_> = bands
            .into_iter()
            .filter(|(_, (style, text))| {
                style.line
                    || !(text.left.is_empty() && text.center.is_empty() && text.right.is_empty())
            })
            .collect();
        if bands.is_empty() && html.is_empty() {
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
            for band in html {
                if band.fill {
                    let mut doc = band.doc.clone();
                    fill_variables(&mut doc, &vars);
                    let width = page.size.width - margins.left - margins.right;
                    engine
                        .layout_margin_document(
                            &doc,
                            &band.styles,
                            width,
                            band.images,
                            &band.fetcher,
                        )
                        .draw(page, band.band, band.spacing, margins);
                } else if let Some(laid_out) = &band.laid_out {
                    laid_out.draw(page, band.band, band.spacing, margins);
                }
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
        let section = |depth: usize| {
            info.sections
                .get(page - 1)
                .map_or_else(String::new, |sections| sections[depth].clone())
        };
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
            ("section", section(0)),
            ("subsection", section(1)),
            ("subsubsection", section(2)),
        ];
        vars.extend(builtins.map(|(name, value)| (name.to_owned(), value)));
        vars
    }
}

/// The elements of `doc` to fill in: those with a class named after a
/// variable, with that variable's value.
fn fill_targets<'v>(
    doc: &Document,
    vars: &'v HashMap<String, String>,
) -> Vec<(presswork_dom::NodeId, &'v str)> {
    doc.descendants(doc.root())
        .filter_map(|id| {
            let class = doc.element(id)?.attr("class")?;
            let value = class
                .split_ascii_whitespace()
                .find_map(|name| vars.get(name))?;
            Some((id, value.as_str()))
        })
        .collect()
}

/// Sets the text of each element whose class names a variable to that
/// variable's value, as the script in wkhtmltopdf's documentation does:
///
/// ```js
/// var x = ['page', 'topage', ...];
/// for (var i in x) {
///     var y = document.getElementsByClassName(x[i]);
///     for (var j = 0; j < y.length; ++j) y[j].textContent = vars[x[i]];
/// }
/// ```
fn fill_variables(doc: &mut Document, vars: &HashMap<String, String>) {
    for (id, value) in fill_targets(doc, vars) {
        doc.set_text_content(id, value);
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

    #[test]
    fn sections_follow_the_headings_page_by_page() {
        use presswork_layout::{Mark, Point, Size};
        let heading = |level, title: &str| {
            Item::Mark(Mark {
                origin: Point { x: 0.0, y: 0.0 },
                kind: MarkKind::Heading {
                    level,
                    title: title.into(),
                },
            })
        };
        let page = |items| Page {
            size: Size {
                width: 100.0,
                height: 100.0,
            },
            items,
        };
        let pages = [
            page(vec![]),
            page(vec![heading(1, "Seller"), heading(2, "Address")]),
            page(vec![heading(3, "Phone")]),
            page(vec![heading(1, "Terms")]),
            page(vec![]),
        ];
        let names = |s: &[String; 3]| s.clone();
        let sections: Vec<[String; 3]> = sections(&pages).iter().map(names).collect();
        let row = |a: &str, b: &str, c: &str| [a.to_owned(), b.to_owned(), c.to_owned()];
        assert_eq!(
            sections,
            [
                row("", "", ""),
                row("Seller", "Address", ""),
                row("Seller", "Address", "Phone"),
                row("Terms", "", ""),
                row("Terms", "", ""),
            ]
        );
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
            sections: vec![[String::new(), String::new(), String::new()]],
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

    /// Writes `html` to a fresh temporary file and returns its path.
    fn temp_html(name: &str, html: &str) -> String {
        let dir = std::env::temp_dir().join(format!("presswork-headers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, html).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn load(args: &HeaderArgs) -> (Vec<HtmlBand<'static>>, Vec<String>, Vec<LoadFailure>) {
        let mut failures = Vec::new();
        let (bands, warnings) = args
            .load_html(&presswork_layout::DataUrls, None, true, &mut failures)
            .unwrap();
        (bands, warnings, failures)
    }

    #[test]
    fn html_headers_size_missing_margins_and_widen_given_ones() {
        let header = temp_html(
            "sized.html",
            "<body style=\"margin: 0\"><div style=\"padding-top: 40pt\"></div>",
        );
        let spacing_mm = "5";
        let spacing = 5.0 * 72.0 / 25.4;
        let args = parse(&[
            "--header-html",
            &header,
            "--header-spacing",
            spacing_mm,
            "--footer-html",
            &header,
        ]);
        let (mut bands, warnings, failures) = load(&args);
        assert!(warnings.is_empty() && failures.is_empty());
        let mut engine = Engine::new();

        let mut layout = LayoutOptions::default();
        HeaderArgs::fit_margins(&mut bands, &mut engine, &mut layout, None, None);
        assert!((layout.margins.top - (40.0 + spacing)).abs() < 1e-3);
        assert_eq!(layout.margins.bottom, 40.0);

        // Given margins grow by the spacing instead.
        let mut layout = LayoutOptions::default();
        HeaderArgs::fit_margins(&mut bands, &mut engine, &mut layout, Some(10.0), Some(20.0));
        assert!((layout.margins.top - (10.0 + spacing)).abs() < 1e-3);
        assert_eq!(layout.margins.bottom, 20.0);
    }

    #[test]
    fn html_headers_must_be_files() {
        let args = parse(&["--header-html", "<p>Invoice</p>"]);
        let mut failures = Vec::new();
        let result = args.load_html(&presswork_layout::DataUrls, None, true, &mut failures);
        assert!(result.is_err());

        // Network URLs are reported like other network resources.
        let args = parse(&["--footer-html", "https://example.com/footer.html"]);
        let (bands, _, failures) = load(&args);
        assert!(bands.is_empty());
        assert_eq!(failures[0].error, LoadError::Network);
    }

    #[test]
    fn scripts_get_their_page_number_classes_filled_in() {
        let html = "<script>subst()</script><p>Page <span class=\"page\">?</span> of \
                    <span class=\"x topage\"></span></p>";
        let args = parse(&["--footer-html", &temp_html("subst.html", html)]);
        let (bands, warnings, _) = load(&args);
        assert!(bands[0].fill && warnings.is_empty());
        let mut doc = bands[0].doc.clone();
        fill_variables(&mut doc, &args.variables(2, 3, &info()));
        let p = doc.find_element("p").unwrap();
        assert_eq!(doc.text_content(p), "Page 2 of 3");

        // Without a script, wkhtmltopdf would show the document as it is.
        let args = parse(&[
            "--footer-html",
            &temp_html("static.html", "<p class=page>x"),
        ]);
        assert!(!load(&args).0[0].fill);

        // A script presswork cannot stand in for is pointed out.
        let html = "<script>document.getElementById('n').textContent = 1</script><b id=n>";
        let args = parse(&["--header-html", &temp_html("id.html", html)]);
        let warnings = load(&args).1;
        assert!(warnings[0].contains("--header-html"), "{warnings:?}");
    }
}
