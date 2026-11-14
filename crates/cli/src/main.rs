mod compat;

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use presswork_layout::{Orientation, PageSize};
use presswork_paint::PdfOptions;

/// Convert HTML documents to PDF.
///
/// Accepts wkhtmltopdf's command line. wkhtmltopdf options that presswork
/// does not support yet are ignored with a warning.
#[derive(Debug, Parser)]
#[command(name = "presswork", version, about)]
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

    /// Do not print warnings.
    #[arg(short, long)]
    quiet: bool,
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
            Some(url) if url.starts_with("file://") => Source::File(url["file://".len()..].into()),
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

fn render(_html: &[u8], _options: &PdfOptions) -> Result<Vec<u8>> {
    bail!("rendering is not implemented yet");
}

fn main() -> Result<()> {
    let args = compat::normalize(std::env::args_os());
    let cli = Cli::parse_from(args.args);

    if !cli.quiet && !args.ignored.is_empty() {
        eprintln!(
            "warning: ignoring unsupported wkhtmltopdf options: {}",
            args.ignored.join(", ")
        );
    }

    let (source, sink) = cli.io()?;
    let html = source.read()?;
    let options = PdfOptions {
        page_size: cli.page_size,
        orientation: cli.orientation,
        title: cli.title,
        author: None,
    };
    let pdf = render(&html, &options)?;
    sink.write(&pdf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<(Cli, Vec<String>), clap::Error> {
        let n = compat::normalize(std::iter::once("presswork").chain(args.iter().copied()));
        Ok((Cli::try_parse_from(n.args)?, n.ignored))
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
        assert_eq!(ignored.len(), 6);
    }

    /// The shape of a wicked_pdf invocation: short quiet flag, file paths.
    #[test]
    fn accepts_wicked_pdf_invocation() {
        let (cli, _) = parse(&[
            "-q",
            "--encoding",
            "UTF-8",
            "--orientation",
            "Portrait",
            "--margin-top",
            "10",
            "--print-media-type",
            "file:///tmp/wicked_pdf.html",
            "/tmp/wicked_pdf_generated_file.pdf",
        ])
        .expect("wicked_pdf arguments should parse");

        assert!(cli.quiet);
        assert_eq!(cli.orientation, Orientation::Portrait);
        assert_eq!(
            cli.io().unwrap().0,
            Source::File("/tmp/wicked_pdf.html".into())
        );
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
