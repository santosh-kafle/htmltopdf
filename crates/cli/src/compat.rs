//! wkhtmltopdf command-line compatibility.
//!
//! Wrapper libraries (pdfkit, wicked_pdf, Snappy, ...) pass many wkhtmltopdf
//! options that presswork does not implement yet. Rejecting them would break
//! those callers at argument parsing, before rendering even starts. Instead,
//! every documented wkhtmltopdf option is recognised here: options presswork
//! supports are passed through to clap, options whose behavior presswork
//! already matches are dropped silently, and the rest are dropped with a
//! warning. Options wkhtmltopdf itself does not know are left for clap to
//! reject, as wkhtmltopdf would.

use std::ffi::OsString;

/// How a recognised option is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Implemented by presswork; passed to clap with this many values.
    Pass(usize),
    /// `--extended-help`: shown as `--help`.
    Help,
    /// `--log-level <level>`: `none` maps to `--quiet`, other levels are accepted.
    LogLevel,
    /// presswork already behaves this way; dropped silently with its values.
    Accept(usize),
    /// Not implemented yet; dropped with a warning, along with its values.
    Ignore(usize),
}

use Kind::{Accept, Help, Ignore, LogLevel, Pass};

/// Every wkhtmltopdf 0.12.6 option: long name, short alias, handling.
const OPTIONS: &[(&str, Option<char>, Kind)] = &[
    // Implemented.
    ("page-size", Some('s'), Pass(1)),
    ("orientation", Some('O'), Pass(1)),
    ("title", None, Pass(1)),
    ("quiet", Some('q'), Pass(0)),
    ("help", Some('h'), Pass(0)),
    ("version", Some('V'), Pass(0)),
    ("extended-help", Some('H'), Help),
    ("log-level", None, LogLevel),
    // Already the default behavior: no JavaScript, no plugins, no local
    // file access, print media.
    ("disable-javascript", Some('n'), Accept(0)),
    ("disable-plugins", None, Accept(0)),
    ("disable-local-file-access", None, Accept(0)),
    ("print-media-type", None, Accept(0)),
    ("no-debug-javascript", None, Accept(0)),
    ("stop-slow-scripts", None, Accept(0)),
    ("no-stop-slow-scripts", None, Accept(0)),
    // Global options.
    ("collate", None, Ignore(0)),
    ("no-collate", None, Ignore(0)),
    ("cookie-jar", None, Ignore(1)),
    ("copies", None, Ignore(1)),
    ("dpi", Some('d'), Ignore(1)),
    ("grayscale", Some('g'), Ignore(0)),
    ("htmldoc", None, Ignore(0)),
    ("image-dpi", None, Ignore(1)),
    ("image-quality", None, Ignore(1)),
    ("license", None, Ignore(0)),
    ("lowquality", Some('l'), Ignore(0)),
    ("manpage", None, Ignore(0)),
    ("margin-bottom", Some('B'), Ignore(1)),
    ("margin-left", Some('L'), Ignore(1)),
    ("margin-right", Some('R'), Ignore(1)),
    ("margin-top", Some('T'), Ignore(1)),
    ("page-height", None, Ignore(1)),
    ("page-width", None, Ignore(1)),
    ("no-pdf-compression", None, Ignore(0)),
    ("read-args-from-stdin", None, Ignore(0)),
    ("readme", None, Ignore(0)),
    ("use-xserver", None, Ignore(0)),
    // Outline options.
    ("dump-default-toc-xsl", None, Ignore(0)),
    ("dump-outline", None, Ignore(1)),
    ("outline", None, Ignore(0)),
    ("no-outline", None, Ignore(0)),
    ("outline-depth", None, Ignore(1)),
    // Page options.
    ("allow", None, Ignore(1)),
    ("background", None, Ignore(0)),
    ("no-background", None, Ignore(0)),
    ("bypass-proxy-for", None, Ignore(1)),
    ("cache-dir", None, Ignore(1)),
    ("checkbox-checked-svg", None, Ignore(1)),
    ("checkbox-svg", None, Ignore(1)),
    ("cookie", None, Ignore(2)),
    ("custom-header", None, Ignore(2)),
    ("custom-header-propagation", None, Ignore(0)),
    ("no-custom-header-propagation", None, Ignore(0)),
    ("debug-javascript", None, Ignore(0)),
    ("default-header", None, Ignore(0)),
    ("encoding", None, Ignore(1)),
    ("disable-external-links", None, Ignore(0)),
    ("enable-external-links", None, Ignore(0)),
    ("disable-forms", None, Ignore(0)),
    ("enable-forms", None, Ignore(0)),
    ("images", None, Ignore(0)),
    ("no-images", None, Ignore(0)),
    ("disable-internal-links", None, Ignore(0)),
    ("enable-internal-links", None, Ignore(0)),
    ("enable-javascript", None, Ignore(0)),
    ("javascript-delay", None, Ignore(1)),
    ("keep-relative-links", None, Ignore(0)),
    ("load-error-handling", None, Ignore(1)),
    ("load-media-error-handling", None, Ignore(1)),
    ("enable-local-file-access", None, Ignore(0)),
    ("minimum-font-size", None, Ignore(1)),
    ("exclude-from-outline", None, Ignore(0)),
    ("include-in-outline", None, Ignore(0)),
    ("page-offset", None, Ignore(1)),
    ("password", None, Ignore(1)),
    ("enable-plugins", None, Ignore(0)),
    ("post", None, Ignore(2)),
    ("post-file", None, Ignore(2)),
    ("no-print-media-type", None, Ignore(0)),
    ("proxy", Some('p'), Ignore(1)),
    ("proxy-hostname-lookup", None, Ignore(0)),
    ("radiobutton-checked-svg", None, Ignore(1)),
    ("radiobutton-svg", None, Ignore(1)),
    ("resolve-relative-links", None, Ignore(0)),
    ("run-script", None, Ignore(1)),
    ("disable-smart-shrinking", None, Ignore(0)),
    ("enable-smart-shrinking", None, Ignore(0)),
    ("ssl-crt-path", None, Ignore(1)),
    ("ssl-key-password", None, Ignore(1)),
    ("ssl-key-path", None, Ignore(1)),
    ("disable-toc-back-links", None, Ignore(0)),
    ("enable-toc-back-links", None, Ignore(0)),
    ("user-style-sheet", None, Ignore(1)),
    ("username", None, Ignore(1)),
    ("viewport-size", None, Ignore(1)),
    ("window-status", None, Ignore(1)),
    ("zoom", None, Ignore(1)),
    // Header and footer options.
    ("footer-center", None, Ignore(1)),
    ("footer-font-name", None, Ignore(1)),
    ("footer-font-size", None, Ignore(1)),
    ("footer-html", None, Ignore(1)),
    ("footer-left", None, Ignore(1)),
    ("footer-line", None, Ignore(0)),
    ("no-footer-line", None, Ignore(0)),
    ("footer-right", None, Ignore(1)),
    ("footer-spacing", None, Ignore(1)),
    ("header-center", None, Ignore(1)),
    ("header-font-name", None, Ignore(1)),
    ("header-font-size", None, Ignore(1)),
    ("header-html", None, Ignore(1)),
    ("header-left", None, Ignore(1)),
    ("header-line", None, Ignore(0)),
    ("no-header-line", None, Ignore(0)),
    ("header-right", None, Ignore(1)),
    ("header-spacing", None, Ignore(1)),
    ("replace", None, Ignore(2)),
    // Table of contents options.
    ("disable-dotted-lines", None, Ignore(0)),
    ("toc-header-text", None, Ignore(1)),
    ("toc-level-indentation", None, Ignore(1)),
    ("disable-toc-links", None, Ignore(0)),
    ("toc-text-size-shrink", None, Ignore(1)),
    ("xsl-style-sheet", None, Ignore(1)),
];

/// Command-line arguments rewritten for clap.
#[derive(Debug, Default)]
pub struct Normalized {
    /// Arguments to hand to clap, including the program name.
    pub args: Vec<OsString>,
    /// Options and objects that were dropped because presswork does not
    /// support them yet, as the user spelled them.
    pub ignored: Vec<String>,
}

fn lookup(arg: &str) -> Option<Kind> {
    if let Some(long) = arg.strip_prefix("--") {
        OPTIONS
            .iter()
            .find(|(name, _, _)| *name == long)
            .map(|&(_, _, kind)| kind)
    } else {
        let mut chars = arg.strip_prefix('-')?.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return None;
        };
        OPTIONS
            .iter()
            .find(|(_, short, _)| *short == Some(c))
            .map(|&(_, _, kind)| kind)
    }
}

/// Rewrites wkhtmltopdf-style arguments into ones clap understands.
pub fn normalize<I>(args: I) -> Normalized
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut out = Normalized::default();
    out.args.extend(args.next());

    let mut positional_only = false;
    while let Some(arg) = args.next() {
        let text = match arg.to_str() {
            Some(text) if !positional_only => text.to_owned(),
            _ => {
                out.args.push(arg);
                continue;
            }
        };

        if text == "--" {
            positional_only = true;
            out.args.push(arg);
            continue;
        }

        // Positional arguments, including `-` for stdin/stdout, and the
        // wkhtmltopdf object keywords.
        if text == "-" || !text.starts_with('-') {
            match text.as_str() {
                // `page <input>` is the explicit form of a plain input.
                "page" => {}
                "toc" => out.ignored.push(text),
                "cover" => {
                    out.ignored.push(text);
                    args.next();
                }
                _ => out.args.push(arg),
            }
            continue;
        }

        // wkhtmltopdf does not accept `--name=value`, but clap users expect
        // it, so treat it as one argument carrying its value.
        let (name, inline_value) = match text.split_once('=') {
            Some((name, value)) if text.starts_with("--") => (name, Some(value)),
            _ => (text.as_str(), None),
        };
        let values = |n: usize| if inline_value.is_some() { 0 } else { n };

        match lookup(name) {
            Some(Pass(n)) => {
                out.args.push(arg);
                out.args.extend(args.by_ref().take(values(n)));
            }
            Some(Help) => out.args.push("--help".into()),
            Some(LogLevel) => {
                let level = match inline_value {
                    Some(value) => Some(OsString::from(value)),
                    None => args.next(),
                };
                if level.is_some_and(|level| level == "none") {
                    out.args.push("--quiet".into());
                }
            }
            Some(Accept(n)) => {
                args.by_ref().take(values(n)).for_each(drop);
            }
            Some(Ignore(n)) => {
                out.ignored.push(name.to_owned());
                args.by_ref().take(values(n)).for_each(drop);
            }
            // Unknown to wkhtmltopdf as well: let clap report it.
            None => out.args.push(arg),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> (Vec<String>, Vec<String>) {
        let n = normalize(std::iter::once("presswork").chain(args.iter().copied()));
        let args = n
            .args
            .into_iter()
            .skip(1)
            .map(|a| a.into_string().unwrap())
            .collect();
        (args, n.ignored)
    }

    #[test]
    fn option_table_has_no_duplicates() {
        for (i, (name, short, _)) in OPTIONS.iter().enumerate() {
            for (other, other_short, _) in &OPTIONS[i + 1..] {
                assert_ne!(name, other, "duplicate --{name}");
                if short.is_some() {
                    assert_ne!(short, other_short, "duplicate short for --{name}");
                }
            }
        }
    }

    #[test]
    fn drops_unsupported_options_with_their_values() {
        let (args, ignored) = run(&[
            "--margin-top",
            "10mm",
            "-L",
            "5mm",
            "--cookie",
            "name",
            "value",
            "--grayscale",
            "in.html",
            "out.pdf",
        ]);
        assert_eq!(args, ["in.html", "out.pdf"]);
        assert_eq!(ignored, ["--margin-top", "-L", "--cookie", "--grayscale"]);
    }

    #[test]
    fn passes_supported_options_and_their_values() {
        // `toc` here is a title, not the TOC object keyword.
        let (args, ignored) = run(&["--title", "toc", "-s", "A4", "-q", "-", "-"]);
        assert_eq!(args, ["--title", "toc", "-s", "A4", "-q", "-", "-"]);
        assert!(ignored.is_empty());
    }

    #[test]
    fn accepts_options_that_match_default_behavior_silently() {
        let (args, ignored) = run(&["-n", "--disable-local-file-access", "a", "b"]);
        assert_eq!(args, ["a", "b"]);
        assert!(ignored.is_empty());
    }

    #[test]
    fn handles_inline_values() {
        let (args, ignored) = run(&["--margin-top=1in", "--page-size=A5", "a", "b"]);
        assert_eq!(args, ["--page-size=A5", "a", "b"]);
        assert_eq!(ignored, ["--margin-top"]);
    }

    #[test]
    fn log_level_none_means_quiet() {
        assert_eq!(
            run(&["--log-level", "none", "a", "b"]).0,
            ["--quiet", "a", "b"]
        );
        assert_eq!(run(&["--log-level", "warn", "a", "b"]).0, ["a", "b"]);
    }

    #[test]
    fn handles_object_keywords() {
        let (args, ignored) = run(&["cover", "c.html", "toc", "page", "in.html", "out.pdf"]);
        assert_eq!(args, ["in.html", "out.pdf"]);
        assert_eq!(ignored, ["cover", "toc"]);
    }

    #[test]
    fn leaves_unknown_options_for_clap() {
        assert_eq!(run(&["--bogus", "a", "b"]).0, ["--bogus", "a", "b"]);
    }

    #[test]
    fn stops_at_double_dash() {
        let (args, ignored) = run(&["--", "--grayscale", "toc"]);
        assert_eq!(args, ["--", "--grayscale", "toc"]);
        assert!(ignored.is_empty());
    }
}
