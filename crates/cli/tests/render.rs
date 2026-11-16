//! End-to-end tests of the `presswork` binary.

use std::io::Write;
use std::process::{Command, Stdio};

fn presswork(args: &[&str], stdin: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_presswork"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("presswork runs");
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

/// pdfkit's invocation: HTML on stdin, PDF on stdout, unsupported options.
#[test]
fn renders_stdin_to_stdout_like_pdfkit() {
    let out = presswork(
        &[
            "--quiet",
            "--page-size",
            "Letter",
            "--margin-top",
            "0.75in",
            "--footer-right",
            "[page]",
            "-",
            "-",
        ],
        "<h1>Invoice</h1><p>नमस्ते</p>".as_bytes(),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(out.stdout.starts_with(b"%PDF-"));
    // Options that change the output are reported even under --quiet;
    // supported ones are not.
    assert!(stderr.contains("--footer-right"), "{stderr}");
    assert!(!stderr.contains("--margin-top"), "{stderr}");
}

#[test]
fn renders_file_to_file() {
    let dir = std::env::temp_dir().join(format!("presswork-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("invoice.pdf");
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/invoice-ne.html"
    );

    let out = presswork(&[input, output.to_str().unwrap()], b"");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pdf = std::fs::read(&output).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A 2x1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x7b, 0x40, 0xe8,
    0xdd, 0x00, 0x00, 0x00, 0x0f, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xc0,
    0xc0, 0xf0, 0x1f, 0x00, 0x07, 0x00, 0x01, 0xff, 0x7e, 0x08, 0xb1, 0xd0, 0x00, 0x00, 0x00, 0x00,
    0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn has_image(pdf: &[u8]) -> bool {
    pdf.windows(14).any(|w| w == b"/Subtype/Image")
}

/// An invoice next to its logo, as wicked_pdf and Snappy users set it up.
#[test]
fn loads_local_images_only_when_allowed() {
    let dir = std::env::temp_dir().join(format!("presswork-images-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("img")).unwrap();
    std::fs::write(dir.join("img/logo.png"), PNG).unwrap();
    let input = dir.join("invoice.html");
    std::fs::write(&input, "<img src='img/logo.png' alt='ACME'><p>Invoice</p>").unwrap();
    let input = input.to_str().unwrap();

    // Blocked by default, as in wkhtmltopdf 0.12.6, with a hint.
    let out = presswork(&[input, "-"], b"");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(!has_image(&out.stdout));
    assert!(stderr.contains("--enable-local-file-access"), "{stderr}");
    // --quiet silences it, as wkhtmltopdf's does.
    let out = presswork(&["-q", input, "-"], b"");
    assert!(out.stderr.is_empty());

    for allow in [
        vec!["--enable-local-file-access"],
        vec!["--allow", dir.to_str().unwrap()],
    ] {
        let mut args = allow.clone();
        args.extend([input, "-"]);
        let out = presswork(&args, b"");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stderr.is_empty(),
            "{allow:?}: {stderr}"
        );
        assert!(has_image(&out.stdout), "{allow:?}");
    }

    // abort turns the failure into an error.
    let out = presswork(&["--load-media-error-handling", "abort", input, "-"], b"");
    assert!(!out.status.success());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reports_network_images_even_when_quiet() {
    let html = b"<img src='https://example.com/logo.png'><img src='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DAwPAfAAcAAf9+CLHQAAAAAElFTkSuQmCC'>";
    let out = presswork(&["--quiet", "-", "-"], html);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("https://example.com/logo.png"), "{stderr}");
    assert!(has_image(&out.stdout), "the data: image still loads");

    let out = presswork(&["--quiet", "--strict", "-", "-"], html);
    assert!(!out.status.success());
}

/// Whether the PDF's pages are A5 (420 by 595 points) rather than A4.
fn is_a5(pdf: &[u8]) -> bool {
    String::from_utf8_lossy(pdf).contains("/MediaBox[0 0 419.5")
}

/// A document linking `css/print.css`, which makes the pages A5.
struct Site(std::path::PathBuf);

impl Site {
    fn new(name: &str, html: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("presswork-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("css")).unwrap();
        std::fs::write(
            dir.join("css/print.css"),
            "@import 'page.css'; p { color: #333 }",
        )
        .unwrap();
        std::fs::write(dir.join("css/page.css"), "@page { size: A5 }").unwrap();
        std::fs::write(dir.join("invoice.html"), html).unwrap();
        Self(dir)
    }

    fn path(&self, file: &str) -> String {
        self.0.join(file).to_str().unwrap().to_owned()
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn linked_stylesheets_load_under_the_file_rules() {
    let site = Site::new(
        "link",
        "<link rel=stylesheet href='css/print.css'><p>Invoice</p>",
    );
    let input = site.path("invoice.html");

    let out = presswork(&[&input, "-"], b"");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(!is_a5(&out.stdout));
    assert!(stderr.contains("css/print.css"), "{stderr}");

    // Allowing the directory loads the stylesheet and what it imports.
    let out = presswork(&["--allow", &site.path("css"), &input, "-"], b"");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && stderr.is_empty(), "{stderr}");
    assert!(is_a5(&out.stdout));
}

#[test]
fn user_style_sheet_applies_below_the_document_styles() {
    let site = Site::new("user", "<p>Invoice</p>");
    let input = site.path("invoice.html");
    let sheet = site.path("css/page.css");

    // The operator's stylesheet is read even with local file access off.
    let out = presswork(&["--user-style-sheet", &sheet, &input, "-"], b"");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(is_a5(&out.stdout));

    // The document's own @page wins.
    std::fs::write(
        site.0.join("invoice.html"),
        "<style>@page { size: A4 }</style><p>Invoice</p>",
    )
    .unwrap();
    let out = presswork(&["--user-style-sheet", &sheet, &input, "-"], b"");
    assert!(!is_a5(&out.stdout));

    let out = presswork(&["--user-style-sheet", "missing.css", &input, "-"], b"");
    assert!(!out.status.success());
}

#[test]
fn no_images_skips_images_without_warnings() {
    let html = b"<img src='missing.png' alt='Logo'><p>Invoice</p>";
    let out = presswork(&["--no-images", "-", "-"], html);
    assert!(out.status.success() && out.stderr.is_empty());
    assert!(!has_image(&out.stdout));
}
