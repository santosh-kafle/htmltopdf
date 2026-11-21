mod compat;
mod headers;
mod loader;
mod page;

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::builder::BoolishValueParser;
use clap::{ArgAction, Parser};
use presswork_layout::{Engine, Fetcher, LoadError, LoadFailure};
use presswork_paint::PdfOptions;
use presswork_style::Stylist;
use url::Url;

use headers::{DocumentInfo, HeaderArgs, UserStyleSheet};
use loader::ResourceArgs;
use page::PageArgs;

/// Convert HTML documents to PDF.
///
/// Accepts wkhtmltopdf's command line. wkhtmltopdf options that presswork
/// does not support yet are ignored with a warning.
#[derive(Debug, Parser)]
#[command(name = "presswork", about, disable_version_flag = true)]
struct Cli {
    /// Input HTML file followed by the output PDF file. Use `-` to read from
    /// standard input or write to standard output.
    #[arg(value_name = "INPUT OUTPUT", num_args = 2.., required = true)]
    paths: Vec<OsString>,

    #[command(flatten)]
    page: PageArgs,

    #[command(flatten)]
    resources: ResourceArgs,

    #[command(flatten)]
    headers: HeaderArgs,

    /// Document title written to the PDF metadata.
    #[arg(long)]
    title: Option<String>,

    #[command(flatten)]
    pdf: PdfArgs,

    /// A stylesheet (path or file: URL) applied to the document as a user
    /// stylesheet: the document's own styles win unless it says !important
    #[arg(long, value_name = "PATH")]
    user_style_sheet: Option<String>,

    /// Do not print warnings, except about ignored options and network
    /// resources, which change the output.
    #[arg(short, long)]
    quiet: bool,

    /// Fail instead of warning when an unsupported option (HTML headers,
    /// zoom, ...) or a network resource would change the output.
    #[arg(
        long,
        env = "PRESSWORK_STRICT",
        action = ArgAction::SetTrue,
        value_parser = BoolishValueParser::new()
    )]
    strict: bool,
}

/// Outline and link options.
#[derive(Debug, clap::Args)]
struct PdfArgs {
    /// Put an outline (bookmarks) of the document's headings in the PDF
    /// (default)
    #[arg(long, overrides_with = "no_outline")]
    outline: bool,

    /// Do not put an outline in the PDF
    #[arg(long, overrides_with = "outline")]
    no_outline: bool,

    /// How many heading levels the outline shows
    #[arg(long, value_name = "LEVEL", default_value_t = 4)]
    outline_depth: usize,

    /// Make links to other documents clickable (default)
    #[arg(long, overrides_with = "disable_external_links")]
    enable_external_links: bool,

    /// Do not make links to other documents clickable
    #[arg(long, overrides_with = "enable_external_links")]
    disable_external_links: bool,

    /// Make links within the document clickable (default)
    #[arg(long, overrides_with = "disable_internal_links")]
    enable_internal_links: bool,

    /// Do not make links within the document clickable
    #[arg(long, overrides_with = "enable_internal_links")]
    disable_internal_links: bool,
}

impl PdfArgs {
    fn options(&self, title: Option<String>) -> PdfOptions {
        PdfOptions {
            title,
            author: None,
            outline_depth: if self.no_outline {
                0
            } else {
                self.outline_depth
            },
            external_links: !self.disable_external_links,
            internal_links: !self.disable_internal_links,
        }
    }
}

/// The wkhtmltopdf release whose command line presswork accepts.
///
/// Wrapper libraries parse `-V` to pick an argument style: wicked_pdf, for
/// one, falls back to wkhtmltopdf 0.9 syntax unless it finds
/// `wkhtmltopdf X.Y.Z`.
const WKHTMLTOPDF_VERSION: &str = "0.12.6";

fn version_line() -> String {
    format!(
        "wkhtmltopdf {WKHTMLTOPDF_VERSION} (presswork {})",
        env!("CARGO_PKG_VERSION")
    )
}

/// Where the HTML comes from.
#[derive(Debug, PartialEq)]
enum Source {
    Stdin,
    File(PathBuf),
}

/// Where the PDF goes.
#[derive(Debug, PartialEq)]
enum Sink {
    Stdout,
    File(PathBuf),
}

impl Source {
    /// The URL that relative URLs in the document resolve against.
    fn base_url(&self) -> Result<Url> {
        let url = match self {
            Source::Stdin => {
                let cwd =
                    std::env::current_dir().context("failed to read the current directory")?;
                Url::from_directory_path(cwd)
            }
            Source::File(path) => Url::from_file_path(
                std::path::absolute(path)
                    .with_context(|| format!("failed to resolve {}", path.display()))?,
            ),
        };
        url.map_err(|()| anyhow::anyhow!("cannot make a URL for the input's location"))
    }

    fn read(&self) -> Result<Vec<u8>> {
        match self {
            Source::Stdin => {
                let mut buf = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut buf)
                    .context("failed to read standard input")?;
                Ok(buf)
            }
            Source::File(path) => {
                std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))
            }
        }
    }
}

impl Sink {
    fn write(&self, pdf: &[u8]) -> Result<()> {
        match self {
            Sink::Stdout => {
                let mut out = std::io::stdout().lock();
                out.write_all(pdf)
                    .and_then(|()| out.flush())
                    .context("failed to write standard output")
            }
            Sink::File(path) => std::fs::write(path, pdf)
                .with_context(|| format!("failed to write {}", path.display())),
        }
    }
}

impl Cli {
    /// The input as given on the command line.
    fn input(&self) -> String {
        let inputs = &self.paths[..self.paths.len() - 1];
        inputs
            .iter()
            .map(|input| input.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn io(&self) -> Result<(Source, Sink)> {
        let (output, inputs) = self
            .paths
            .split_last()
            .expect("clap requires at least two paths");
        let [input] = inputs else {
            bail!("multiple input documents are not supported yet");
        };
        let source = match input.to_str() {
            Some("-") => Source::Stdin,
            // wicked_pdf passes its temporary HTML file as a `file://` URL.
            Some(url) if url.starts_with("file:") => Source::File(file_url_to_path(url)?),
            Some(url) if url.contains("://") => {
                bail!("loading documents from URLs is not supported: {url}")
            }
            _ => Source::File(input.into()),
        };
        let sink = if output == "-" {
            Sink::Stdout
        } else {
            Sink::File(output.into())
        };
        Ok((source, sink))
    }
}

/// Converts a `file:` URL to a local path, decoding percent escapes.
fn file_url_to_path(url: &str) -> Result<PathBuf> {
    let parsed = url::Url::parse(url).with_context(|| format!("invalid file URL: {url}"))?;
    match parsed.to_file_path() {
        Ok(path) => Ok(path),
        Err(()) => bail!("file URL does not name a local file: {url}"),
    }
}

impl Cli {
    /// Warnings about dropped options to print, or an error in strict mode.
    fn check_ignored(&self, args: &compat::Normalized) -> Result<Vec<String>> {
        let mut warnings = Vec::new();
        if !args.lossy.is_empty() {
            let list = args.lossy.join(", ");
            if self.strict {
                bail!("unsupported wkhtmltopdf options would change the output: {list}");
            }
            warnings.push(format!(
                "warning: ignoring unsupported wkhtmltopdf options; the output will not \
                 reflect them: {list}"
            ));
        }
        if !args.ignored.is_empty() && !self.quiet {
            warnings.push(format!(
                "warning: ignoring unsupported wkhtmltopdf options: {}",
                args.ignored.join(", ")
            ));
        }
        Ok(warnings)
    }
}

/// Reads a file named on the command line (a path or `file:` URL), such
/// as `--user-style-sheet`: whoever runs presswork named it, so the
/// document's file access rules do not apply. Returns its bytes and URL.
fn read_local(value: &str) -> Result<(Vec<u8>, Url)> {
    let path = if value.starts_with("file:") {
        file_url_to_path(value)?
    } else if value.contains("://") {
        bail!("loading files from URLs is not supported: {value}");
    } else {
        std::path::absolute(value).with_context(|| format!("failed to resolve {value}"))?
    };
    let bytes =
        std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let url = Url::from_file_path(&path)
        .map_err(|()| anyhow::anyhow!("cannot make a URL for {}", path.display()))?;
    Ok((bytes, url))
}

/// Reads `--user-style-sheet`. Returns the CSS and its URL.
fn read_user_style_sheet(value: &str) -> Result<UserStyleSheet> {
    let (bytes, url) = read_local(value)?;
    let css = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes));
    Ok((css.into_owned(), url))
}

/// Runs the whole pipeline: parse, style, lay out, write the PDF.
fn render(html: &[u8], cli: &Cli, base: Url) -> Result<Vec<u8>> {
    let doc = presswork_dom::parse_html(html);
    let loader = cli.resources.loader();
    let fetcher = Fetcher::new(&loader, Some(base));
    let user_css = (cli.user_style_sheet.as_deref())
        .map(read_user_style_sheet)
        .transpose()?;
    let mut styles = Stylist::new();
    if let Some((css, url)) = &user_css {
        styles.add_user_stylesheet(css, Some(url.clone()), &fetcher);
    }
    styles.add_document(&doc, &fetcher);
    let images = !cli.resources.no_images;
    let mut failures = Vec::new();
    let (mut html_bands, warnings) =
        cli.headers
            .load_html(&loader, user_css.as_ref(), images, &mut failures)?;
    if !cli.quiet {
        for warning in warnings {
            eprintln!("{warning}");
        }
    }

    let mut layout = cli.page.layout_options(styles.page());
    // --default-header implies a 2cm top margin, as if given with -T.
    if cli.page.margin_top.is_none()
        && let Some(top) = cli.headers.default_margin_top()
    {
        layout.margins.top = top;
    }
    layout.images = images;
    let mut engine = Engine::new();
    if !engine.has_system_fonts() {
        // Always shown: the PDF would have no visible text.
        eprintln!(
            "warning: no system fonts found, so text will not be drawn; \
             install fontconfig and at least one font package"
        );
    }
    let (top, bottom) =
        (cli.headers).explicit_margins(cli.page.margin_top, cli.page.margin_bottom, styles.page());
    HeaderArgs::fit_margins(&mut html_bands, &mut engine, &mut layout, top, bottom);
    let mut pages = engine.layout(&doc, &styles, &layout, &fetcher);
    // wkhtmltopdf uses the document's <title> unless --title is given.
    let title = cli.title.clone().or_else(|| doc.title());
    let info = DocumentInfo {
        webpage: cli.input(),
        title: doc.title().unwrap_or_default(),
        doctitle: title.clone().unwrap_or_default(),
        now: chrono::Local::now().naive_local(),
        sections: headers::sections(&pages),
    };
    cli.headers
        .draw(&mut engine, &mut pages, layout.margins, &info, &html_bands);
    let options = cli.pdf.options(title);
    let pdf = presswork_paint::render_pdf(&pages, &options)?;

    failures.extend(fetcher.failures());
    for band in &html_bands {
        failures.extend(band.fetcher.failures());
    }
    failures.extend(pdf.broken_images.into_iter().map(|url| LoadFailure {
        url,
        error: LoadError::Failed("the image data is corrupt".to_owned()),
    }));
    for warning in cli.resources.report(&failures, cli.quiet, cli.strict)? {
        eprintln!("{warning}");
    }
    Ok(pdf.bytes)
}

fn main() -> Result<()> {
    let args = compat::normalize(std::env::args_os());
    if args.version {
        println!("{}", version_line());
        return Ok(());
    }
    let cli = Cli::parse_from(&args.args);
    for warning in cli.check_ignored(&args)? {
        eprintln!("{warning}");
    }

    let (source, sink) = cli.io()?;
    let html = source.read()?;
    let pdf = render(&html, &cli, source.base_url()?)?;
    sink.write(&pdf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use presswork_layout::{Orientation, PageSize};

    fn parse(args: &[&str]) -> Result<(Cli, compat::Normalized), clap::Error> {
        let n = compat::normalize(std::iter::once("presswork").chain(args.iter().copied()));
        Ok((Cli::try_parse_from(&n.args)?, n))
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn accepts_wkhtmltopdf_style_arguments() {
        let (cli, _) = parse(&[
            "-s",
            "Letter",
            "-O",
            "Landscape",
            "--title",
            "Invoice",
            "in.html",
            "out.pdf",
        ])
        .expect("wkhtmltopdf-style arguments should parse");

        assert_eq!(cli.page.page_size, Some(PageSize::Letter));
        assert_eq!(cli.page.orientation, Some(Orientation::Landscape));
        assert_eq!(cli.title.as_deref(), Some("Invoice"));
        assert_eq!(
            cli.io().unwrap(),
            (Source::File("in.html".into()), Sink::File("out.pdf".into()))
        );
    }

    /// The invocation pdfkit (Ruby and Python) builds: quiet, options, then
    /// HTML on stdin and the PDF on stdout.
    #[test]
    fn accepts_pdfkit_invocation() {
        let (cli, ignored) = parse(&[
            "--quiet",
            "--page-size",
            "Letter",
            "--margin-top",
            "0.75in",
            "--margin-right",
            "0.75in",
            "--margin-bottom",
            "0.75in",
            "--margin-left",
            "0.75in",
            "--encoding",
            "UTF-8",
            "--disable-smart-shrinking",
            "-",
            "-",
        ])
        .expect("pdfkit arguments should parse");

        assert!(cli.quiet);
        assert_eq!(cli.page.page_size, Some(PageSize::Letter));
        assert_eq!(cli.page.margin_top, Some(54.0));
        assert_eq!(cli.io().unwrap(), (Source::Stdin, Sink::Stdout));
        assert_eq!(ignored.lossy, ["--encoding"]);

        // pdfkit always passes --quiet, but options that change the output
        // still warn.
        let warnings = cli.check_ignored(&ignored).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("--encoding"), "{warnings:?}");
    }

    /// The shape of a wicked_pdf invocation: options, then its temporary
    /// HTML file as `"file:///" + path`, which gives four slashes.
    #[test]
    fn accepts_wicked_pdf_invocation() {
        let (cli, _) = parse(&[
            "--encoding",
            "UTF-8",
            "--orientation",
            "Portrait",
            "--margin-top",
            "10",
            "--print-media-type",
            "file:////tmp/wicked_pdf.html",
            "/tmp/wicked_pdf_generated_file.pdf",
        ])
        .expect("wicked_pdf arguments should parse");

        assert_eq!(cli.page.orientation, Some(Orientation::Portrait));
        assert_eq!(
            cli.io().unwrap().0,
            Source::File("/tmp/wicked_pdf.html".into())
        );
    }

    /// wicked_pdf's version detection (`WickedPdf::BinaryVersion`). If this
    /// regex does not match, wicked_pdf assumes 0.9.9 and switches to the old
    /// `--toc` / `--cover` argument style.
    #[test]
    fn version_satisfies_wicked_pdf_detection() {
        let re = regex::Regex::new(r"wkhtmltopdf\s*(\d*\.\d*\.\d*\w*)").unwrap();
        let line = version_line();
        let caps = re
            .captures(&line)
            .expect("wicked_pdf should find a version");
        assert_eq!(&caps[1], WKHTMLTOPDF_VERSION);
    }

    #[test]
    fn decodes_file_urls() {
        let (cli, _) = parse(&["file:///tmp/my%20file.html", "out.pdf"]).unwrap();
        assert_eq!(
            cli.io().unwrap().0,
            Source::File("/tmp/my file.html".into())
        );
        let (cli, _) = parse(&["file://otherhost/x.html", "out.pdf"]).unwrap();
        assert!(cli.io().is_err());
    }

    #[test]
    fn outline_and_link_options_reach_the_pdf() {
        let options = |args: &[&str]| {
            let mut all = args.to_vec();
            all.extend(["a", "b"]);
            let (cli, n) = parse(&all).unwrap();
            assert_eq!(cli.check_ignored(&n).unwrap(), Vec::<String>::new());
            cli.pdf.options(None)
        };
        let default = options(&[]);
        assert_eq!(default.outline_depth, 4);
        assert!(default.external_links && default.internal_links);
        assert_eq!(options(&["--outline-depth", "2"]).outline_depth, 2);
        assert_eq!(options(&["--no-outline"]).outline_depth, 0);
        assert_eq!(options(&["--no-outline", "--outline"]).outline_depth, 4);
        let links = options(&["--disable-external-links", "--disable-internal-links"]);
        assert!(!links.external_links && !links.internal_links);
        assert!(options(&["--disable-external-links", "--enable-external-links"]).external_links);
    }

    #[test]
    fn quiet_silences_only_harmless_warnings() {
        let (cli, n) = parse(&["-q", "--disable-forms", "--zoom", "2", "a", "b"]).unwrap();
        let warnings = cli.check_ignored(&n).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("--zoom") && !warnings[0].contains("--disable-forms"));

        let (cli, n) = parse(&["--disable-forms", "--zoom", "2", "a", "b"]).unwrap();
        assert_eq!(cli.check_ignored(&n).unwrap().len(), 2);
    }

    #[test]
    fn strict_fails_on_options_that_change_the_output() {
        let (cli, n) = parse(&["--strict", "--disable-forms", "a", "b"]).unwrap();
        assert!(cli.check_ignored(&n).is_ok());

        let (cli, n) = parse(&["--strict", "--viewport-size", "800x600", "a", "b"]).unwrap();
        let err = cli.check_ignored(&n).unwrap_err().to_string();
        assert!(err.contains("--viewport-size"), "{err}");
    }

    #[test]
    fn refuses_network_urls() {
        let (cli, _) = parse(&["https://example.com/", "out.pdf"]).unwrap();
        assert!(cli.io().is_err());
    }

    #[test]
    fn rejects_options_unknown_to_wkhtmltopdf() {
        assert!(parse(&["--bogus", "in.html", "out.pdf"]).is_err());
    }

    #[test]
    fn rejects_multiple_inputs_clearly() {
        let (cli, _) = parse(&["a.html", "b.html", "out.pdf"]).unwrap();
        let err = cli.io().unwrap_err().to_string();
        assert!(err.contains("multiple input"), "{err}");
    }

    #[test]
    fn requires_input_and_output() {
        assert!(parse(&["in.html"]).is_err());
    }

    #[test]
    fn accepts_every_wkhtmltopdf_page_size() {
        for size in PageSize::ALL {
            let (cli, _) = parse(&["-s", size.name(), "a", "b"]).unwrap();
            assert_eq!(cli.page.page_size, Some(*size));
        }
    }
}
