use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use presswork_layout::{Orientation, PageSize};
use presswork_paint::PdfOptions;

/// Convert HTML documents to PDF.
#[derive(Debug, Parser)]
#[command(name = "presswork", version, about)]
struct Cli {
    /// Input HTML file.
    input: PathBuf,

    /// Output PDF file.
    output: PathBuf,

    /// Page size: A3, A4, A5, Letter or Legal.
    #[arg(short = 's', long, default_value = "A4")]
    page_size: PageSize,

    /// Page orientation: Portrait or Landscape.
    #[arg(
        short = 'O',
        long,
        value_enum,
        ignore_case = true,
        default_value_t = OrientationArg::Portrait
    )]
    orientation: OrientationArg,

    /// Document title written to the PDF metadata.
    #[arg(long)]
    title: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OrientationArg {
    Portrait,
    Landscape,
}

impl From<OrientationArg> for Orientation {
    fn from(value: OrientationArg) -> Self {
        match value {
            OrientationArg::Portrait => Orientation::Portrait,
            OrientationArg::Landscape => Orientation::Landscape,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let _html = std::fs::read_to_string(&cli.input)
        .with_context(|| format!("failed to read {}", cli.input.display()))?;

    let _options = PdfOptions {
        page_size: cli.page_size,
        orientation: cli.orientation.into(),
        title: cli.title,
        author: None,
    };

    bail!("rendering is not implemented yet");
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn accepts_wkhtmltopdf_style_arguments() {
        let cli = Cli::try_parse_from([
            "presswork",
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
        assert!(matches!(cli.orientation, OrientationArg::Landscape));
        assert_eq!(cli.title.as_deref(), Some("Invoice"));
    }
}
