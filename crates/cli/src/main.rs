mod compat;

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::builder::BoolishValueParser;
use clap::{ArgAction, Parser};
use presswork_layout::{Engine, LayoutOptions};
use presswork_layout::{Orientation, PageSize};
use presswork_paint::PdfOptions;

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

    /// Page size: A0-A9, B0-B10, C5E, Comm10E, DLE, Executive, Folio,
    /// Ledger, Legal, Letter or Tabloid.
    #[arg(short = 's', long, default_value_t = PageSize::A4)]
    page_size: PageSize,

    /// Page orientation: Portrait or Landscape.
    #[arg(short = 'O', long, default_value = "Portrait")]
    orientation: Orientation,

    /// Document title written to the PDF metadata.
    #[arg(long)]
    title: Option<String>,

    /// Do not print warnings, except for ignored options that change the
    /// output.
    #[arg(short, long)]
    quiet: bool,

    /// Fail instead of warning when an unsupported option would change the
    /// output (margins, headers, footers, ...).
    #[arg(
        long,
        env = "PRESSWORK_STRICT",
        action = ArgAction::SetTrue,
        value_parser = BoolishValueParser::new()
    )]
    strict: bool,
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

/// Runs the whole pipeline: parse, lay out, write the PDF.
fn render(html: &[u8], cli: &Cli) -> Result<Vec<u8>> {
    let doc = presswork_dom::parse_html(html);
    let layout = LayoutOptions {
        page_size: cli.page_size.size(cli.orientation),
        ..LayoutOptions::default()
    };
    let pages = Engine::new().layout(&doc, &layout);
    let pdf = PdfOptions {
        // wkhtmltopdf uses the document's <title> unless --title is given.
        title: cli.title.clone().or_else(|| doc.title()),
        author: None,
    };
    Ok(presswork_paint::render_pdf(&pages, &pdf)?)
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
    let pdf = render(&html, &cli)?;
    sink.write(&pdf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

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

        assert_eq!(cli.page_size, PageSize::Letter);
        assert_eq!(cli.orientation, Orientation::Landscape);
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
        assert_eq!(cli.page_size, PageSize::Letter);
        assert_eq!(cli.io().unwrap(), (Source::Stdin, Sink::Stdout));
        assert_eq!(ignored.lossy.len(), 5);

        // pdfkit always passes --quiet, but dropped margins still warn.
        let warnings = cli.check_ignored(&ignored).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("--margin-top"), "{warnings:?}");
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

        assert_eq!(cli.orientation, Orientation::Portrait);
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
    fn quiet_silences_only_harmless_warnings() {
        let (cli, n) = parse(&["-q", "--outline", "--margin-top", "1in", "a", "b"]).unwrap();
        let warnings = cli.check_ignored(&n).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("--margin-top") && !warnings[0].contains("--outline"));

        let (cli, n) = parse(&["--outline", "--margin-top", "1in", "a", "b"]).unwrap();
        assert_eq!(cli.check_ignored(&n).unwrap().len(), 2);
    }

    #[test]
    fn strict_fails_on_options_that_change_the_output() {
        let (cli, n) = parse(&["--strict", "--outline", "a", "b"]).unwrap();
        assert!(cli.check_ignored(&n).is_ok());

        let (cli, n) = parse(&["--strict", "--footer-right", "[page]", "a", "b"]).unwrap();
        let err = cli.check_ignored(&n).unwrap_err().to_string();
        assert!(err.contains("--footer-right"), "{err}");
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
            assert_eq!(cli.page_size, *size);
        }
    }
}
