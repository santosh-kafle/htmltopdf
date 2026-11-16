//! wkhtmltopdf command-line compatibility.
//!
//! Wrapper libraries (pdfkit, wicked_pdf, Snappy, ...) pass many wkhtmltopdf
//! options that presswork does not implement yet. Rejecting them would break
//! those callers at argument parsing, before rendering even starts. Instead,
//! every documented wkhtmltopdf option is recognised here: options presswork
//! supports are passed through to clap, options whose behavior presswork
//! already matches are dropped silently, and the rest are dropped with a
//! warning. Unsupported options that change the output are reported even
//! under `--quiet`, since pdfkit passes `--quiet` by default. Options wkhtmltopdf itself does not know are left for clap to
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
    /// `--version`: prints a wkhtmltopdf-style version line.
    Version,
    /// Not implemented yet, but only affects things presswork does not do
    /// anyway (fetching, links, outline); dropped with a warning that
    /// `--quiet` silences, along with its values.
    Ignore(usize),
    /// Not implemented yet and changes what ends up on the page (margins,
    /// headers, footers, zoom, ...); dropped with a warning that
    /// `--quiet` does not silence, or an error in strict mode.
    Lossy(usize),
}

use Kind::{Accept, Help, Ignore, LogLevel, Lossy, Pass, Version};

/// Every wkhtmltopdf 0.12.6 option: long name, short alias, handling.
const OPTIONS: &[(&str, Option<char>, Kind)] = &[
    // Implemented.
    ("page-size", Some('s'), Pass(1)),
    ("orientation", Some('O'), Pass(1)),
    ("page-width", None, Pass(1)),
    ("page-height", None, Pass(1)),
    ("margin-top", Some('T'), Pass(1)),
    ("margin-right", Some('R'), Pass(1)),
    ("margin-bottom", Some('B'), Pass(1)),
    ("margin-left", Some('L'), Pass(1)),
    ("title", None, Pass(1)),
    ("quiet", Some('q'), Pass(0)),
    ("help", Some('h'), Pass(0)),
    ("version", Some('V'), Version),
    ("extended-help", Some('H'), Help),
    ("log-level", None, LogLevel),
    ("enable-local-file-access", None, Pass(0)),
    ("disable-local-file-access", None, Pass(0)),
    ("allow", None, Pass(1)),
    ("images", None, Pass(0)),
    ("no-images", None, Pass(0)),
    ("load-media-error-handling", None, Pass(1)),
    // Already the default behavior: no JavaScript, no plugins, print media.
    ("disable-javascript", Some('n'), Accept(0)),
    ("disable-plugins", None, Accept(0)),
    ("print-media-type", None, Accept(0)),
    ("no-debug-javascript", None, Accept(0)),
    ("stop-slow-scripts", None, Accept(0)),
    // presswork never shrinks content to fit, which is what this asks for.
    ("disable-smart-shrinking", None, Accept(0)),
    ("no-stop-slow-scripts", None, Accept(0)),
    // Global options.
    ("collate", None, Ignore(0)),
    ("no-collate", None, Ignore(0)),
    ("cookie-jar", None, Ignore(1)),
    ("copies", None, Lossy(1)),
    ("dpi", Some('d'), Lossy(1)),
    ("grayscale", Some('g'), Lossy(0)),
    ("htmldoc", None, Ignore(0)),
    ("image-dpi", None, Ignore(1)),
    ("image-quality", None, Ignore(1)),
    ("license", None, Ignore(0)),
    ("lowquality", Some('l'), Ignore(0)),
    ("manpage", None, Ignore(0)),
    ("no-pdf-compression", None, Ignore(0)),
    ("read-args-from-stdin", None, Lossy(0)),
    ("readme", None, Ignore(0)),
    ("use-xserver", None, Ignore(0)),
    // Outline options.
    ("dump-default-toc-xsl", None, Ignore(0)),
    ("dump-outline", None, Ignore(1)),
    ("outline", None, Ignore(0)),
    ("no-outline", None, Ignore(0)),
    ("outline-depth", None, Ignore(1)),
    // Page options.
    ("background", None, Ignore(0)),
    ("no-background", None, Lossy(0)),
    ("bypass-proxy-for", None, Ignore(1)),
    ("cache-dir", None, Ignore(1)),
    ("checkbox-checked-svg", None, Ignore(1)),
    ("checkbox-svg", None, Ignore(1)),
    ("cookie", None, Ignore(2)),
    ("custom-header", None, Ignore(2)),
    ("custom-header-propagation", None, Ignore(0)),
    ("no-custom-header-propagation", None, Ignore(0)),
    ("debug-javascript", None, Ignore(0)),
    ("default-header", None, Lossy(0)),
    ("encoding", None, Lossy(1)),
    ("disable-external-links", None, Ignore(0)),
    ("enable-external-links", None, Ignore(0)),
    ("disable-forms", None, Ignore(0)),
    ("enable-forms", None, Ignore(0)),
    ("disable-internal-links", None, Ignore(0)),
    ("enable-internal-links", None, Ignore(0)),
    ("enable-javascript", None, Lossy(0)),
    ("javascript-delay", None, Lossy(1)),
    ("keep-relative-links", None, Ignore(0)),
    ("load-error-handling", None, Ignore(1)),
    ("minimum-font-size", None, Lossy(1)),
    ("exclude-from-outline", None, Ignore(0)),
    ("include-in-outline", None, Ignore(0)),
    ("page-offset", None, Lossy(1)),
    ("password", None, Ignore(1)),
    ("enable-plugins", None, Ignore(0)),
    ("post", None, Ignore(2)),
    ("post-file", None, Ignore(2)),
    ("no-print-media-type", None, Lossy(0)),
    ("proxy", Some('p'), Ignore(1)),
    ("proxy-hostname-lookup", None, Ignore(0)),
    ("radiobutton-checked-svg", None, Ignore(1)),
    ("radiobutton-svg", None, Ignore(1)),
    ("resolve-relative-links", None, Ignore(0)),
    ("run-script", None, Lossy(1)),
    ("enable-smart-shrinking", None, Lossy(0)),
    ("ssl-crt-path", None, Ignore(1)),
    ("ssl-key-password", None, Ignore(1)),
    ("ssl-key-path", None, Ignore(1)),
    ("disable-toc-back-links", None, Ignore(0)),
    ("enable-toc-back-links", None, Ignore(0)),
    ("user-style-sheet", None, Lossy(1)),
    ("username", None, Ignore(1)),
    ("viewport-size", None, Lossy(1)),
    ("window-status", None, Lossy(1)),
    ("zoom", None, Lossy(1)),
    // Header and footer options.
    ("footer-center", None, Lossy(1)),
    ("footer-font-name", None, Lossy(1)),
    ("footer-font-size", None, Lossy(1)),
    ("footer-html", None, Lossy(1)),
    ("footer-left", None, Lossy(1)),
    ("footer-line", None, Lossy(0)),
    ("no-footer-line", None, Lossy(0)),
    ("footer-right", None, Lossy(1)),
    ("footer-spacing", None, Lossy(1)),
    ("header-center", None, Lossy(1)),
    ("header-font-name", None, Lossy(1)),
    ("header-font-size", None, Lossy(1)),
    ("header-html", None, Lossy(1)),
    ("header-left", None, Lossy(1)),
    ("header-line", None, Lossy(0)),
    ("no-header-line", None, Lossy(0)),
    ("header-right", None, Lossy(1)),
    ("header-spacing", None, Lossy(1)),
    ("replace", None, Lossy(2)),
    // wkhtmltopdf 0.9 spelled the TOC and cover objects as options. wicked_pdf
    // still sends these when it cannot detect a 0.12 binary.
    ("toc", None, Lossy(0)),
    ("cover", None, Lossy(1)),
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
    /// Dropped options that do not change the output, as spelled.
    pub ignored: Vec<String>,
    /// Dropped options and objects that change the output, as spelled.
    pub lossy: Vec<String>,
    /// `-V` / `--version` was given.
    pub version: bool,
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
                "toc" => out.lossy.push(text),
                "cover" => {
                    out.lossy.push(text);
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
            Some(Version) => out.version = true,
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
            Some(Lossy(n)) => {
                out.lossy.push(name.to_owned());
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

    struct Run {
        args: Vec<String>,
        ignored: Vec<String>,
        lossy: Vec<String>,
        version: bool,
    }

    fn run(args: &[&str]) -> Run {
        let n = normalize(std::iter::once("presswork").chain(args.iter().copied()));
        Run {
            args: n
                .args
                .into_iter()
                .skip(1)
                .map(|a| a.into_string().unwrap())
                .collect(),
            ignored: n.ignored,
            lossy: n.lossy,
            version: n.version,
        }
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
        let r = run(&[
            "--zoom",
            "1.5",
            "-d",
            "300",
            "--cookie",
            "name",
            "value",
            "--outline",
            "--footer-right",
            "[page]",
            "in.html",
            "out.pdf",
        ]);
        assert_eq!(r.args, ["in.html", "out.pdf"]);
        assert_eq!(r.ignored, ["--cookie", "--outline"]);
        assert_eq!(r.lossy, ["--zoom", "-d", "--footer-right"]);
    }

    #[test]
    fn passes_supported_options_and_their_values() {
        // `toc` here is a title, not the TOC object keyword.
        let r = run(&["--title", "toc", "-s", "A4", "-T", "10mm", "-q", "-", "-"]);
        assert_eq!(
            r.args,
            ["--title", "toc", "-s", "A4", "-T", "10mm", "-q", "-", "-"]
        );
        assert!(r.ignored.is_empty() && r.lossy.is_empty());
    }

    #[test]
    fn accepts_options_that_match_default_behavior_silently() {
        let r = run(&[
            "-n",
            "--print-media-type",
            "--disable-smart-shrinking",
            "a",
            "b",
        ]);
        assert_eq!(r.args, ["a", "b"]);
        assert!(r.ignored.is_empty() && r.lossy.is_empty());
    }

    #[test]
    fn handles_inline_values() {
        let r = run(&["--zoom=2", "--page-size=A5", "a", "b"]);
        assert_eq!(r.args, ["--page-size=A5", "a", "b"]);
        assert_eq!(r.lossy, ["--zoom"]);
    }

    #[test]
    fn log_level_none_means_quiet() {
        assert_eq!(
            run(&["--log-level", "none", "a", "b"]).args,
            ["--quiet", "a", "b"]
        );
        assert_eq!(run(&["--log-level", "warn", "a", "b"]).args, ["a", "b"]);
    }

    #[test]
    fn handles_object_keywords() {
        let r = run(&["cover", "c.html", "toc", "page", "in.html", "out.pdf"]);
        assert_eq!(r.args, ["in.html", "out.pdf"]);
        assert_eq!(r.lossy, ["cover", "toc"]);
    }

    /// wicked_pdf falls back to the 0.9 spelling when it misdetects the
    /// binary's version.
    #[test]
    fn handles_old_style_object_options() {
        let r = run(&["-q", "--cover", "c.html", "--toc", "in.html", "out.pdf"]);
        assert_eq!(r.args, ["-q", "in.html", "out.pdf"]);
        assert_eq!(r.lossy, ["--cover", "--toc"]);
    }

    #[test]
    fn detects_version_flag() {
        assert!(run(&["-V"]).version);
        assert!(run(&["--version"]).version);
        assert!(!run(&["a", "b"]).version);
    }

    #[test]
    fn leaves_unknown_options_for_clap() {
        assert_eq!(run(&["--bogus", "a", "b"]).args, ["--bogus", "a", "b"]);
    }

    #[test]
    fn stops_at_double_dash() {
        let r = run(&["--", "--grayscale", "toc"]);
        assert_eq!(r.args, ["--", "--grayscale", "toc"]);
        assert!(r.ignored.is_empty() && r.lossy.is_empty());
    }
}
