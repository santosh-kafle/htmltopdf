# presswork

An HTML to PDF converter for documents such as invoices, reports and
certificates, written in Rust with no browser dependency. It aims to be a
replacement for [wkhtmltopdf](https://wkhtmltopdf.org/), which is no longer
maintained, and accepts the same command line.

> **Status: early development. presswork does not produce PDFs yet.**
> The command line works; the rendering engine does not exist yet.

## What works today

- **wkhtmltopdf's command line.** Every documented wkhtmltopdf 0.12.6 option
  is recognised, so invocations from wrapper libraries such as `pdfkit`,
  `wicked_pdf` and `snappy` parse:
  - `-` reads HTML from standard input or writes the PDF to standard output.
  - `file://` input URLs are accepted.
  - `-V` reports `wkhtmltopdf 0.12.6 (presswork <version>)`, so wrappers that
    check the version (such as `wicked_pdf`) use the current argument style.
  - Options that are not implemented yet are ignored with a warning.
    Options unknown to wkhtmltopdf are rejected, as wkhtmltopdf does.
  - `-q` / `--quiet` and `--log-level none` silence warnings, except for
    ignored options that change the output, such as margins, headers and
    footers. Those are always reported, because `pdfkit` passes `--quiet` by
    default.
  - `--strict`, or `PRESSWORK_STRICT=1` in the environment, makes those
    options an error instead of a warning. Use the environment variable when
    a wrapper library builds the command line for you.
- **Page options.** `-s` / `--page-size` accepts every size wkhtmltopdf does
  (A0–A9, B0–B10, C5E, Comm10E, DLE, Executive, Folio, Ledger, Legal, Letter,
  Tabloid). `-O` / `--orientation` and `--title` are also supported.

Running a conversion currently ends with
`Error: rendering is not implemented yet`.

## Usage

```sh
presswork [OPTIONS] <INPUT> <OUTPUT>
presswork --page-size Letter invoice.html invoice.pdf
cat invoice.html | presswork --quiet - - > invoice.pdf
```

## Differences from wkhtmltopdf

- **No JavaScript.** `--javascript-delay`, `--window-status` and
  `--run-script` have no effect. Render dynamic content on the server first.
- **No network access.** Documents are not fetched from `http(s)://` URLs.
  Local and network resources referenced by a document will only load when
  explicitly allowed.
- **Standards-based layout.** Output follows the CSS specifications, not
  QtWebKit, so templates tuned for wkhtmltopdf's rendering (for example its
  smart shrinking) may lay out differently.
- **One input document.** Multiple inputs, cover pages and `toc` objects are
  not supported yet.

## Building

Requires Rust 1.85 or newer.

```sh
cargo build --release
cargo test --workspace
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
