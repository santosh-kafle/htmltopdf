# presswork

A drop-in replacement for [wkhtmltopdf](https://wkhtmltopdf.org/): same
command-line interface, modern CSS, secure by default.

wkhtmltopdf is no longer maintained, yet many applications still call it
directly or through wrappers such as `pdfkit`, `wicked_pdf` and `snappy`.
presswork aims to let those codebases switch by replacing a single binary,
without rewriting their PDF generation.

Under the hood it is a fast, Chrome-free HTML/CSS to PDF engine written in
Rust, built for documents: invoices, reports, certificates and books.

> **Status:** early development. The pipeline is scaffolded; rendering is not
> implemented yet.

## Goals

- **wkhtmltopdf compatible.** Accept the same command-line flags and produce
  equivalent output, so existing scripts and wrapper libraries keep working.
- **No browser dependency.** A single static binary with its own layout engine.
- **Print first.** First-class support for CSS Paged Media: `@page` rules,
  margin boxes, page counters, running headers and generated tables of contents.
- **Modern CSS for documents.** Block, inline, tables, flexbox and grid.
- **Safe by default.** No JavaScript execution, and no network or local file
  access unless explicitly allowed.

Out of scope for now: JavaScript, interactive content, screen media.

## Architecture

```
HTML ─► dom ─► style ─► layout ─► paint ─► PDF
               (cascade) (boxes,   (fragment
                          pages)    tree → PDF)
```

| Crate                         | Responsibility                                          |
| ----------------------------- | ------------------------------------------------------- |
| [`dom`](crates/dom)           | HTML parsing into an arena-allocated document tree       |
| [`style`](crates/style)       | CSS parsing, selector matching, cascade, computed values |
| [`layout`](crates/layout)     | Box tree, block/inline/table/flex/grid layout, pagination |
| [`paint`](crates/paint)       | Fragment tree to PDF: text, graphics, links, outline     |
| [`cli`](crates/cli)           | The `presswork` command-line tool                        |

## Building

Requires Rust 1.85 or newer.

```sh
cargo build --release
cargo test --workspace
```

## Usage

```sh
presswork input.html output.pdf --page-size A4 --orientation portrait
```

## Migrating from wkhtmltopdf

presswork uses the same argument order and flag names as wkhtmltopdf, so in
most cases migrating means pointing your scripts or wrapper library at the
`presswork` binary instead.

Flags accepted today:

| wkhtmltopdf flag                         | Status   |
| ---------------------------------------- | -------- |
| `-s`, `--page-size <A3\|A4\|A5\|Letter\|Legal>` | Accepted |
| `-O`, `--orientation <Portrait\|Landscape>`  | Accepted |
| `--title <text>`                         | Accepted |

Known differences:

- **No JavaScript.** Pages are rendered as static HTML/CSS, so
  `--javascript-delay`, `--window-status` and `--run-script` have no effect.
  Render dynamic content on the server before converting.
- **Standards-based layout.** Output follows the CSS specifications rather
  than QtWebKit's rendering, so pages that relied on old WebKit quirks may
  look slightly different.
- **Restricted resource access by default.** Local files and network
  resources are not loaded unless explicitly allowed.
