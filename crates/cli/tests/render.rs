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
            "-",
            "-",
        ],
        "<h1>Invoice</h1><p>नमस्ते</p>".as_bytes(),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(out.stdout.starts_with(b"%PDF-"));
    // Dropped margins are reported even under --quiet.
    assert!(stderr.contains("--margin-top"), "{stderr}");
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
