# presswork

An HTML to PDF converter for documents such as invoices, reports and
certificates, written in Rust with no browser dependency. It aims to be a
replacement for [wkhtmltopdf](https://wkhtmltopdf.org/), which is no longer
maintained, and accepts the same command line.

> **Status: early development.** presswork renders text documents styled
> and images with a useful subset of CSS. Floats, positioning, flexbox and
> grid are not supported yet.

## What works today

- **HTML to PDF, end to end.** HTML5 parsing; paragraphs, headings, lists
  with hanging markers, preformatted text and tables.
- **CSS.** `<style>` elements, `style=""` attributes and a built-in default
  stylesheet, with the full cascade (specificity, source order,
  `!important`, inheritance) and Selectors Level 4 matching (`:nth-child`,
  `:not`, `:is`, attribute selectors, combinators). `@media print` rules
  apply; `@media screen` rules do not. Supported properties:
  - text: `color`, `font-family`, `font-size`, `font-weight`, `font-style`,
    `font`, `line-height`, `text-align`, `white-space`
  - boxes: `display`, `margin`, `padding`, `border` (drawn solid), `width`,
    `max-width`, `box-sizing`, `background-color`
  - images: `width`, `height`, `max-width`, `max-height`, `vertical-align`
  - tables: `border-collapse`, `border-spacing`, `vertical-align`
  - lists: `list-style-type`
  - pages: `break-before`, `break-after` and the `page-break-*` aliases
- **Legacy HTML attributes** that wkhtmltopdf-era templates rely on:
  `align`, `valign`, `width`, `height`, `bgcolor`, `border`, `cellpadding`,
  `cellspacing`, `nowrap` and `<font>`.
- **Images.** PNG, JPEG, GIF and WebP in `<img>`, inline or as blocks,
  sized by their attributes and CSS and scaled to fit with `max-width`.
  `data:` URLs always load. Local files load only with
  `--enable-local-file-access` or from a directory given with `--allow`,
  as in wkhtmltopdf 0.12.6, and relative paths resolve against the input
  file (or the current directory for standard input). An image that fails
  to load is reported and replaced by its `alt` text.
- **Tables.** Columns are sized to their content, with `colspan`, cell
  borders and backgrounds, and collapsed borders. Rows are never split
  across pages, and `<thead>` rows repeat at the top of every page.
- **Pagination.** Pages break between lines and table rows; backgrounds and
  borders continue across the break. `@page { size; margin }` is honored.
- **Text in any script the system has fonts for.** Text is shaped with
  HarfBuzz-compatible shaping, so Devanagari conjuncts and vowel signs render
  correctly, and fonts fall back by script. Fonts are embedded and subsetted,
  and the PDF text can be searched and copied.
- **Document metadata.** The `<title>` becomes the PDF title unless `--title`
  is given.
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
    ignored options that change the output, such as headers, footers and
    zoom. Those are always reported, because `pdfkit` passes `--quiet` by
    default.
  - `--strict`, or `PRESSWORK_STRICT=1` in the environment, makes those
    options an error instead of a warning. Use the environment variable when
    a wrapper library builds the command line for you.
- **Page options.** `-s` / `--page-size` accepts every size wkhtmltopdf does
  (A0–A9, B0–B10, C5E, Comm10E, DLE, Executive, Folio, Ledger, Legal, Letter,
  Tabloid). `-O` / `--orientation`, `--page-width`, `--page-height`,
  `-T` / `-R` / `-B` / `-L` (`--margin-*`) and `--title` are supported.
  Options given on the command line win over the document's `@page` rule.
- **Resource options.** `--enable-local-file-access`,
  `--disable-local-file-access`, `--allow`, `--images`, `--no-images` and
  `--load-media-error-handling` (`abort` stops without writing the PDF).

Not yet: SVG and CSS background images, linked stylesheets (`<link>`),
`@font-face`, floats and
positioning, flexbox and grid, generated content (`::before`), text
decoration, `height`, row-spanning cells (they occupy their first row only),
and the wkhtmltopdf header, footer and table-of-contents options.

## Usage

```sh
presswork [OPTIONS] <INPUT> <OUTPUT>
presswork --page-size Letter --margin-top 20mm invoice.html invoice.pdf
cat invoice.html | presswork --quiet - - > invoice.pdf
```

## Differences from wkhtmltopdf

- **No JavaScript.** `--javascript-delay`, `--window-status` and
  `--run-script` have no effect. Render dynamic content on the server first.
- **No network access.** Documents and images are never fetched from
  `http(s)://` URLs. An image that would need the network is reported even
  under `--quiet`, since wkhtmltopdf would have loaded it, and `--strict`
  makes it an error. Embed such images as `data:` URLs or local files.
- **Standards-based layout.** Output follows the CSS specifications, not
  QtWebKit, so templates tuned for wkhtmltopdf's rendering (for example its
  smart shrinking) may lay out differently.
- **Print media.** Stylesheets are evaluated for `print` media, so
  `@media screen` rules do not apply. wkhtmltopdf uses `screen` unless
  `--print-media-type` is given.
- **System fonts.** Fonts come from the system's font configuration, so
  output can differ between machines with different fonts installed. On
  Linux, fontconfig is loaded at runtime: building needs no fontconfig
  development package, but rendering text needs fontconfig and fonts
  installed (presswork warns if it finds none).
- **One input document.** Multiple inputs, cover pages and `toc` objects are
  not supported yet.

## Building

Requires Rust 1.88 or newer.

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
