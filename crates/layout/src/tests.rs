//! End-to-end layout tests: HTML in, positioned items out.

use presswork_dom::parse_html;
use presswork_style::Stylist;

use super::*;

fn layout_with(html: &str, options: &LayoutOptions) -> Vec<Page> {
    let (pages, failures) = with_failures(html, options);
    assert_eq!(failures, [], "every resource loads");
    pages
}

/// The pages, and the resources that failed to load.
fn with_failures(html: &str, options: &LayoutOptions) -> (Vec<Page>, Vec<LoadFailure>) {
    let fetcher = Fetcher::data_only();
    let pages = layout_fetching(&mut Engine::new(), html, options, &fetcher);
    (pages, fetcher.failures())
}

fn layout_fetching(
    engine: &mut Engine,
    html: &str,
    options: &LayoutOptions,
    fetcher: &Fetcher<'_>,
) -> Vec<Page> {
    let doc = parse_html(html.as_bytes());
    let styles = Stylist::for_document(&doc, fetcher);
    engine.layout(&doc, &styles, options, fetcher)
}

fn layout(html: &str) -> Vec<Page> {
    layout_with(html, &LayoutOptions::default())
}

fn runs(page: &Page) -> Vec<&TextRun> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Text(run) => Some(run),
            Item::Fill(_) | Item::Image(_) => None,
        })
        .collect()
}

fn fills(page: &Page) -> Vec<Fill> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Fill(fill) => Some(*fill),
            Item::Text(_) | Item::Image(_) => None,
        })
        .collect()
}

/// The words on a page, in paint order.
fn words(page: &Page) -> Vec<&str> {
    runs(page)
        .into_iter()
        .map(|run| run.covered_text().trim())
        .collect()
}

/// The run containing `word`.
fn run_with<'a>(page: &'a Page, word: &str) -> &'a TextRun {
    runs(page)
        .into_iter()
        .find(|run| run.covered_text().contains(word))
        .unwrap_or_else(|| panic!("no run containing {word:?}"))
}

/// The right edge of a run.
fn right_edge(run: &TextRun) -> f32 {
    run.origin.x + run.glyphs.iter().map(|g| g.advance).sum::<f32>()
}

const RED: Color = Color::rgb(255, 0, 0);
const M: f32 = DEFAULT_MARGIN;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

#[test]
fn lays_out_paragraphs_top_to_bottom() {
    let pages = layout("<h1>Invoice</h1><p>Hello <b>world</b></p>");
    assert_eq!(pages.len(), 1);
    assert_eq!(words(&pages[0]), ["Invoice", "Hello", "world"]);
    let runs = runs(&pages[0]);
    assert!(
        runs[0].origin.y < runs[1].origin.y,
        "heading above paragraph"
    );
    assert_eq!(
        runs[1].origin.y, runs[2].origin.y,
        "same line, same baseline"
    );
    assert_eq!(runs[0].font_size, 24.0);
}

#[test]
fn hides_head_and_scripts() {
    let pages = layout("<title>T</title><script>x()</script><p>Body</p>");
    assert_eq!(words(&pages[0]), ["Body"]);
}

#[test]
fn author_styles_apply() {
    let pages = layout(
        "<style>.total { color: red; font-size: 20pt } .gone { display: none }</style>
         <p class=total>Total</p><p class=gone>Hidden</p>",
    );
    let runs = runs(&pages[0]);
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].color, runs[0].font_size), (RED, 20.0));
}

#[test]
fn color_changes_inside_a_line_keep_per_glyph_text() {
    let pages = layout("<p>ab<span style='color:red'>cd</span>ef</p>");
    let runs = runs(&pages[0]);
    let texts: Vec<&str> = runs.iter().map(|r| r.covered_text()).collect();
    assert_eq!(texts, ["ab", "cd", "ef"]);
    assert_eq!(runs[1].color, RED);
    let letters: Vec<&str> = runs[1]
        .glyphs
        .iter()
        .map(|g| &runs[1].text[g.text_range.clone()])
        .collect();
    assert_eq!(letters, ["c", "d"]);
}

#[test]
fn glyphs_map_back_to_their_own_characters() {
    let pages = layout("<p>Hello</p>");
    let run = runs(&pages[0])[0];
    let chars: Vec<&str> = run
        .glyphs
        .iter()
        .map(|g| &run.text[g.text_range.clone()])
        .collect();
    assert_eq!(chars, ["H", "e", "l", "l", "o"]);
}

#[test]
fn margins_padding_and_width_position_content() {
    let pages = layout(
        "<body style='margin:0'>
         <div style='margin-left:20pt; padding-left:10pt; border-left:5pt solid red; width:100pt'>
         <p style='margin:0; text-align:right'>x</p></div>",
    );
    let page = &pages[0];
    let x = run_with(page, "x");
    // Content box: 20 + 5 + 10 from the page margin, 100 wide.
    assert!(approx(right_edge(x), M + 35.0 + 100.0), "{x:?}");
    // The left border: 5pt wide, starting after the margin.
    let border = fills(page)[0];
    assert_eq!(border.color, RED);
    assert!(approx(border.rect.origin.x, M + 20.0) && approx(border.rect.size.width, 5.0));
}

#[test]
fn auto_margins_center_a_block() {
    let pages = layout(
        "<body style='margin:0'><div style='width:100pt; margin:0 auto; background:red'>x</div>",
    );
    let page = &pages[0];
    let background = fills(page)[0];
    let content_width = page.size.width - 2.0 * M;
    assert!(approx(
        background.rect.origin.x,
        M + (content_width - 100.0) / 2.0
    ));
    assert!(approx(background.rect.size.width, 100.0));
}

#[test]
fn vertical_margins_collapse() {
    let spaced = layout(
        "<body style='margin:0'><p style='margin:10pt 0'>a</p><p style='margin:30pt 0'>b</p>",
    );
    let tight =
        layout("<body style='margin:0'><p style='margin:10pt 0'>a</p><p style='margin:0'>b</p>");
    let gap = |pages: &[Page]| {
        let page = &pages[0];
        run_with(page, "b").origin.y - run_with(page, "a").origin.y
    };
    // max(10, 30) = 30 between the paragraphs, not 40.
    assert!(approx(gap(&spaced) - gap(&tight), 20.0));
}

#[test]
fn background_covers_the_box_and_paints_before_text() {
    let pages = layout(
        "<body style='margin:0'><div style='background:red; padding:10pt'><p style='margin:0'>x</p></div>",
    );
    let page = &pages[0];
    assert!(matches!(page.items[0], Item::Fill(_)), "background first");
    let background = fills(page)[0];
    let x = run_with(page, "x");
    assert!(approx(background.rect.origin.y, M));
    assert!(approx(
        background.rect.size.width,
        page.size.width - 2.0 * M
    ));
    assert!(background.rect.size.height > 20.0 + x.font_size * 0.9);
    assert!(approx(x.origin.x, M + 10.0));
}

#[test]
fn child_margins_collapse_through_an_undecorated_edge() {
    // No padding or border: the paragraph's margin collapses through the
    // div, and the background only covers the text.
    let pages = layout(
        "<body style='margin:0'><div style='background:red'><p style='margin:40pt 0'>x</p></div>",
    );
    let background = fills(&pages[0])[0];
    assert!(approx(background.rect.origin.y, M + 40.0), "{background:?}");
    assert!(background.rect.size.height < 30.0);
}

#[test]
fn long_documents_paginate() {
    let pages = layout(&"<p>line</p>".repeat(200));
    assert!(pages.len() > 1, "{} pages", pages.len());
    for page in &pages {
        for run in runs(page) {
            assert!(run.origin.y > M && run.origin.y < page.size.height - M);
        }
    }
}

#[test]
fn forced_page_breaks() {
    let pages = layout(
        "<p>one</p><p style='page-break-before: always'>two</p>
         <div style='break-after: page'>three</div><p>four</p>",
    );
    let per_page: Vec<Vec<&str>> = pages.iter().map(words).collect();
    assert_eq!(per_page, [vec!["one"], vec!["two", "three"], vec!["four"]]);
}

#[test]
fn backgrounds_continue_across_pages() {
    let html = format!(
        "<div style='background:red'>{}</div>",
        "<p>line</p>".repeat(80)
    );
    let pages = layout(&html);
    assert!(pages.len() > 1);
    for page in &pages {
        assert_eq!(fills(page).len(), 1, "one background piece per page");
    }
}

#[test]
fn table_cells_share_a_row_and_align() {
    let pages = layout(
        "<body style='margin:0'><table width='100%' cellspacing=0 cellpadding=0>
         <tr><td>Item</td><td align=right>Qty</td></tr></table>",
    );
    let page = &pages[0];
    let (item, qty) = (run_with(page, "Item"), run_with(page, "Qty"));
    assert_eq!(item.origin.y, qty.origin.y);
    assert!(approx(item.origin.x, M));
    assert!(approx(right_edge(qty), page.size.width - M), "{qty:?}");
}

#[test]
fn table_columns_fit_their_content() {
    let pages = layout(
        "<body style='margin:0'><table cellspacing=0 cellpadding=0>
         <tr><td>a</td><td>a much longer cell of text</td></tr>
         <tr><td>bb</td><td>c</td></tr></table>",
    );
    let page = &pages[0];
    // An auto-width table is only as wide as its content, and the second
    // column starts right after the widest cell of the first.
    let second = run_with(page, "c");
    let widest_first = right_edge(run_with(page, "bb"));
    assert!(approx(second.origin.x, widest_first), "{second:?}");
    assert!(right_edge(run_with(page, "longer")) < page.size.width / 2.0);
}

#[test]
fn wide_tables_shrink_columns_to_fit_the_page() {
    let cell = "word ".repeat(40);
    let pages = layout(&format!(
        "<table><tr><td>{cell}</td><td>{cell}</td></tr></table>"
    ));
    let page = &pages[0];
    for run in runs(page) {
        assert!(right_edge(run) <= page.size.width - M + 0.5, "{run:?}");
    }
}

#[test]
fn colspan_spans_columns() {
    let pages = layout(
        "<body style='margin:0'><table width=300 cellspacing=0 cellpadding=0>
         <tr><td>aaaa</td><td>bbbb</td><td>cccc</td></tr>
         <tr><td colspan=2 align=right>sum</td><td>t</td></tr></table>",
    );
    let page = &pages[0];
    assert!(approx(
        right_edge(run_with(page, "sum")),
        run_with(page, "cccc").origin.x
    ));
    assert!(approx(
        run_with(page, "t").origin.x,
        run_with(page, "cccc").origin.x
    ));
}

#[test]
fn rowspan_shifts_cells_in_later_rows() {
    let pages = layout("<table><tr><td rowspan=2>a</td><td>b</td></tr><tr><td>c</td></tr></table>");
    let page = &pages[0];
    assert!(approx(
        run_with(page, "c").origin.x,
        run_with(page, "b").origin.x
    ));
}

#[test]
fn cell_borders_and_backgrounds_fill_the_row() {
    let pages = layout(
        "<style>td { border: 1pt solid red; padding: 0 }</style>
         <body style='margin:0'><table cellspacing=0><tr><td>a<br>b<br>c</td><td>x</td></tr></table>",
    );
    let page = &pages[0];
    let borders = fills(page);
    // Four edges per cell, and both cells are as tall as the row.
    assert_eq!(borders.len(), 8);
    let heights: Vec<f32> = borders
        .iter()
        .filter(|f| f.rect.size.width <= 1.0)
        .map(|f| f.rect.size.height)
        .collect();
    assert_eq!(heights.len(), 4);
    assert!(
        heights.iter().all(|h| approx(*h, heights[0])),
        "{heights:?}"
    );
}

#[test]
fn collapsed_borders_are_drawn_once() {
    let html = |collapse: &str| {
        format!(
            "<style>table {{ border-collapse: {collapse}; border-spacing: 0 }}
             td {{ border: 1pt solid red }}</style>
             <table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>"
        )
    };
    let separate = fills(&layout(&html("separate"))[0]).len();
    let collapsed = fills(&layout(&html("collapse"))[0]).len();
    assert_eq!(separate, 16);
    // Shared edges are drawn by one cell only: 16 - 2 vertical - 2 horizontal.
    assert_eq!(collapsed, 12);
}

#[test]
fn vertical_align_moves_cell_content() {
    let html = |valign: &str| {
        format!("<table><tr><td>a<br>b<br>c</td><td valign={valign}>x</td></tr></table>")
    };
    let y = |valign: &str| run_with(&layout(&html(valign))[0], "x").origin.y;
    let (top, middle, bottom) = (y("top"), y("middle"), y("bottom"));
    assert!(top < middle && middle < bottom, "{top} {middle} {bottom}");
    assert!(approx(middle - top, bottom - middle));
}

#[test]
fn table_header_repeats_on_every_page() {
    let rows = "<tr><td>row</td></tr>".repeat(120);
    let pages = layout(&format!(
        "<table><thead><tr><th>Head</th></tr></thead><tbody>{rows}</tbody></table>"
    ));
    assert!(pages.len() >= 2);
    for page in &pages {
        assert_eq!(words(page)[0], "Head");
    }
    let total: usize = pages
        .iter()
        .map(|p| words(p).iter().filter(|w| **w == "row").count())
        .sum();
    assert_eq!(total, 120, "no row lost or duplicated");
}

#[test]
fn list_markers_hang_left_of_the_text() {
    let pages = layout("<ol start=3><li>first</li><li>second</li></ol><ul><li>bullet</li></ul>");
    let page = &pages[0];
    assert_eq!(words(page), ["first", "3.", "second", "4.", "bullet", "•"]);
    let (text, marker) = (run_with(page, "first"), run_with(page, "3."));
    assert_eq!(text.origin.y, marker.origin.y);
    assert!(right_edge(marker) < text.origin.x);
}

#[test]
fn pre_keeps_line_breaks_and_does_not_wrap() {
    let long = "x".repeat(400);
    let pages = layout(&format!("<pre>one\ntwo\n{long}</pre>"));
    let runs = runs(&pages[0]);
    assert_eq!(runs.len(), 3, "three lines, the long one unwrapped");
    assert!(runs[0].origin.y < runs[1].origin.y);
}

#[test]
fn page_size_and_margins_come_from_options() {
    let options = LayoutOptions {
        page_size: Size {
            width: 300.0,
            height: 200.0,
        },
        margins: EdgeSizes {
            top: 50.0,
            right: 0.0,
            bottom: 0.0,
            left: 40.0,
        },
        images: true,
    };
    let pages = layout_with("<body style='margin:0'><p style='margin:0'>x</p>", &options);
    let page = &pages[0];
    assert_eq!(page.size, options.page_size);
    let x = run_with(page, "x");
    assert!(approx(x.origin.x, 40.0) && x.origin.y > 50.0);
}

/// Golden test: a bilingual Nepali/English invoice. Every glyph must come
/// from a font that has it: glyph 0 is `.notdef`, the "tofu" box drawn
/// when no font covers a character.
///
/// Needs a Devanagari font on the system (e.g. `fonts-noto-core`).
#[test]
fn nepali_invoice_has_no_missing_glyphs() {
    let html = include_str!("../../../tests/fixtures/invoice-ne.html");
    let pages = layout(html);
    assert_eq!(pages.len(), 1);
    let mut glyphs = 0;
    for run in runs(&pages[0]) {
        for glyph in &run.glyphs {
            glyphs += 1;
            assert_ne!(
                glyph.id,
                0,
                "missing glyph for {:?}",
                &run.text[glyph.text_range.clone()]
            );
        }
    }
    assert!(glyphs > 500, "only {glyphs} glyphs");
}

/// Golden test: an invoice styled the way real templates are (classes,
/// collapsed borders, shaded rows, `page-break-before`, a long table).
#[test]
fn styled_invoice_paginates_with_repeated_headers() {
    let html = include_str!("../../../tests/fixtures/invoice-styled.html");
    let pages = layout(html);
    assert!(pages.len() >= 3, "{} pages", pages.len());

    // The item table runs over several pages; each starts with its header.
    let with_rows: Vec<&Page> = pages
        .iter()
        .filter(|page| words(page).iter().any(|w| w.contains("Dhaka topi")))
        .collect();
    assert!(with_rows.len() >= 2);
    for page in &with_rows[1..] {
        // The header's first cell; the dots come from another font, so it
        // is more than one run.
        assert!(words(page)[0].starts_with("क्र"), "header repeats");
    }

    // The terms start their own page.
    let last = pages.last().unwrap();
    assert!(words(last)[..3].contains(&"Terms"), "{:?}", words(last));

    // Header cells are white text on a filled background.
    let header = run_with(&pages[0], "Description");
    assert_eq!(header.color, Color::WHITE);
    assert!(
        fills(&pages[0])
            .iter()
            .any(|f| f.color == Color::rgb(0x1a, 0x4e, 0x8a))
    );

    for page in &pages {
        for run in runs(page) {
            assert!(
                run.glyphs.iter().all(|g| g.id != 0),
                "missing glyph in {run:?}"
            );
            assert!(
                right_edge(run) <= page.size.width,
                "{run:?} overflows the page"
            );
        }
    }
}

/// A 2x1 PNG (one red pixel, one blue): 1.5pt by 0.75pt at its natural
/// size.
const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DAwPAfAAcAAf9+CLHQAAAAAElFTkSuQmCC";

fn images(page: &Page) -> Vec<Rect> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Image(image) => Some(image.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn inline_image_sits_on_the_baseline_after_the_text() {
    let pages = layout(&format!("<p>Logo <img src='{PNG}' width=200> text</p>"));
    let [image] = images(&pages[0])[..] else {
        panic!("one image");
    };
    // 200px wide, half as tall: the 2:1 shape is kept.
    assert_eq!((image.size.width, image.size.height), (150.0, 75.0));
    let logo = run_with(&pages[0], "Logo");
    // Right after "Logo " (the run includes the space).
    assert!(approx(image.origin.x, right_edge(logo)));
    let bottom = image.origin.y + image.size.height;
    assert!(
        approx(bottom, logo.origin.y),
        "{bottom} vs {}",
        logo.origin.y
    );
    // The line grew to fit it: the text shares the image's line.
    let text = run_with(&pages[0], "text");
    assert_eq!(text.origin.y, logo.origin.y);
    assert!(approx(text.origin.x, image.origin.x + image.size.width));
}

#[test]
fn vertical_align_middle_centers_an_image_on_the_text() {
    let pages = layout(&format!(
        "<p>x <img src='{PNG}' width=80 style='vertical-align: middle'></p>"
    ));
    let image = images(&pages[0])[0];
    let baseline = run_with(&pages[0], "x").origin.y;
    // Centered half an x-height above the baseline: a few points for
    // 12pt text.
    let center = image.origin.y + image.size.height / 2.0;
    assert!(
        center < baseline && center > baseline - 5.0,
        "{center} vs {baseline}"
    );
}

#[test]
fn block_image_is_centered_by_auto_margins() {
    let pages = layout(&format!(
        "<img src='{PNG}' style='display: block; width: 100pt; margin: 0 auto'>"
    ));
    let [image] = images(&pages[0])[..] else {
        panic!("one image");
    };
    // Inside the body's default 8px (6pt) margin.
    let content = PageSize::A4.size(Orientation::Portrait).width - 2.0 * M - 12.0;
    assert!(approx(image.origin.x, M + 6.0 + (content - 100.0) / 2.0));
    assert_eq!((image.origin.y, image.size.height), (M + 6.0, 50.0));
}

#[test]
fn max_width_fits_a_large_image_into_its_table_cell() {
    let pages = layout(&format!(
        "<table><tr><td><img src='{PNG}' style='width: 4000px; max-width: 100%'></td>\
         <td>Invoice number 42</td></tr></table>"
    ));
    let [image] = images(&pages[0])[..] else {
        panic!("one image");
    };
    let page_width = pages[0].size.width;
    assert!(image.origin.x + image.size.width <= page_width - M + 0.01);
    assert!(image.size.width > 100.0, "uses the room there is");
    assert!(approx(image.size.height, image.size.width / 2.0));
    let text = run_with(&pages[0], "Invoice");
    assert!(right_edge(text) <= page_width - M + 0.01);
}

#[test]
fn missing_image_shows_its_alt_text_and_is_reported_once() {
    let (pages, failures) = with_failures(
        "<p><img src='logo.png' alt='ACME Ltd'> and again <img src=' logo.png '></p>",
        &LayoutOptions::default(),
    );
    assert_eq!(words(&pages[0]), ["ACME Ltd and again"]);
    assert!(images(&pages[0]).is_empty());
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].url, "logo.png");
    // Relative, and the document has no location.
    assert!(matches!(failures[0].error, LoadError::Failed(_)));
}

#[test]
fn missing_image_with_a_size_keeps_its_box() {
    let (pages, _) = with_failures(
        "<p><img src='logo.png' width=100 height=40 style='border: 2px solid red'>x</p>",
        &LayoutOptions::default(),
    );
    let page = &pages[0];
    let border: Vec<Fill> = fills(page).into_iter().filter(|f| f.color == RED).collect();
    assert_eq!(border.len(), 4);
    let left = border
        .iter()
        .map(|f| f.rect.origin.x)
        .fold(f32::MAX, f32::min);
    let x = run_with(page, "x");
    // 100px of content plus 2px of border each side.
    assert!(approx(x.origin.x - left, 75.0 + 3.0));
}

#[test]
fn unsupported_image_formats_are_reported() {
    let (_, failures) = with_failures(
        "<img src='data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22/%3E'>",
        &LayoutOptions::default(),
    );
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0].error.to_string().contains("SVG"),
        "{failures:?}"
    );
}

#[test]
fn an_image_that_does_not_fit_moves_to_the_next_page() {
    let image = format!("<img src='{PNG}' style='display: block; height: 500pt'>");
    let pages = layout(&format!("{image}{image}"));
    assert_eq!(pages.len(), 2);
    assert_eq!(images(&pages[1])[0].origin.y, M);
}

/// The file and glyphs of the system font that sans-serif text uses, or
/// `None` if it is a font collection, which these tests cannot reuse.
fn system_sans() -> Option<(Vec<u8>, Vec<u32>, u64)> {
    let pages = layout("<p style='font-family: sans-serif'>Invoice 42</p>");
    let run = run_with(&pages[0], "Invoice");
    let bytes = run.font.data.data().to_vec();
    if bytes.starts_with(b"ttcf") || run.font.index != 0 {
        eprintln!("skipped: the sans-serif font is in a collection");
        return None;
    }
    let glyphs = run.glyphs.iter().map(|g| g.id).collect();
    Some((bytes, glyphs, run.font.data.id()))
}

/// A `data:` URL for `bytes`, percent-encoded.
fn data_url(bytes: &[u8]) -> String {
    let mut url = String::from("data:font/ttf,");
    for byte in bytes {
        url.push_str(&format!("%{byte:02X}"));
    }
    url
}

/// The run containing "Invoice" when `html` declares the family Brand.
fn brand_run(engine: &mut Engine, css: &str, fetcher: &Fetcher<'_>) -> (Vec<u32>, u64) {
    let html = format!("<style>{css}</style><p style='font-family: Brand, serif'>Invoice 42</p>");
    let pages = layout_fetching(engine, &html, &LayoutOptions::default(), fetcher);
    let run = run_with(&pages[0], "Invoice");
    (
        run.glyphs.iter().map(|g| g.id).collect(),
        run.font.data.id(),
    )
}

#[test]
fn font_face_fonts_are_used_and_do_not_leak_into_the_next_document() {
    let Some((bytes, glyphs, system_id)) = system_sans() else {
        return;
    };
    let css = format!(
        "@font-face {{ font-family: Brand; src: url(missing.woff2) format('woff2'), url({}) }}",
        data_url(&bytes)
    );
    let mut engine = Engine::new();
    let fetcher = Fetcher::data_only();
    let (brand_glyphs, brand_id) = brand_run(&mut engine, &css, &fetcher);
    assert_eq!(brand_glyphs, glyphs, "the @font-face font shapes the text");
    assert_ne!(brand_id, system_id, "from the document, not the system");
    assert_eq!(
        fetcher.failures(),
        [],
        "the WOFF2 source is skipped quietly"
    );

    // Unregistered once layout is done.
    let registered = engine.fonts.collection.family_by_name("presswork 0 Brand");
    assert!(registered.is_none_or(|family| family.fonts().is_empty()));

    // The same engine, a document without the rule: Brand is unknown.
    let (_, fallback_id) = brand_run(&mut engine, "", &fetcher);
    assert_ne!(fallback_id, brand_id);
}

#[test]
fn woff_fonts_are_decoded() {
    let Some((bytes, glyphs, _)) = system_sans() else {
        return;
    };
    let woff = flow::webfont_tests::encode_woff(&bytes);
    let css = format!(
        "@font-face {{ font-family: Brand; src: url({}) format('woff') }}",
        data_url(&woff)
    );
    let fetcher = Fetcher::data_only();
    let (brand_glyphs, _) = brand_run(&mut Engine::new(), &css, &fetcher);
    assert_eq!(fetcher.failures(), []);
    assert_eq!(brand_glyphs, glyphs);
}

#[test]
fn fonts_load_only_when_used_and_failures_are_reported() {
    let fetcher = Fetcher::data_only();
    let css = "@font-face { font-family: Unused; src: url(unused.ttf) }
               @font-face { font-family: Brand; src: url(data:font/ttf,nonsense) }";
    let (glyphs, _) = brand_run(&mut Engine::new(), css, &fetcher);
    assert!(glyphs.iter().all(|&g| g != 0), "falls back to serif");
    let failures = fetcher.failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].url.starts_with("data:font/ttf"));
    assert!(failures[0].error.to_string().contains("not a font"));

    // A family with only WOFF2 sources says so.
    let fetcher = Fetcher::data_only();
    let css = "@font-face { font-family: Brand; src: url(b.woff2) format('woff2') }";
    brand_run(&mut Engine::new(), css, &fetcher);
    let failures = fetcher.failures();
    assert!(
        failures[0].error.to_string().contains("not supported"),
        "{failures:?}"
    );
}

#[test]
fn no_images_shows_alt_text_without_failures() {
    let options = LayoutOptions {
        images: false,
        ..LayoutOptions::default()
    };
    let (pages, failures) = with_failures(&format!("<img src='{PNG}' alt='Logo'>"), &options);
    assert!(images(&pages[0]).is_empty());
    assert_eq!(words(&pages[0]), ["Logo"]);
    assert_eq!(failures, []);
}

/// The text each glyph of `run` maps back to.
fn glyph_texts(run: &TextRun) -> Vec<&str> {
    run.glyphs
        .iter()
        .map(|g| &run.text[g.text_range.clone()])
        .collect()
}

#[test]
fn plain_clusters_map_one_character_per_glyph() {
    let pages = layout("<p lang=ne>कृष्ण किताब</p>");
    let texts = glyph_texts(run_with(&pages[0], "कृ"));
    // कृ is the consonant and the vowel sign below it, each its own
    // glyph, so each maps to its own character.
    assert_eq!(texts[..2], ["क", "\u{943}"]);
    // The conjunct ष्ण is substituted glyphs: they keep the cluster.
    assert!(texts.contains(&"ष\u{94d}ण"), "{texts:?}");
    // ि is drawn before its consonant: both keep the cluster, so the
    // text comes out in reading order.
    let texts = glyph_texts(run_with(&pages[0], "किताब"));
    let ki: Vec<&str> = texts.into_iter().filter(|t| t.contains('ि')).collect();
    assert!(ki.len() >= 2 && ki.iter().all(|t| *t == "कि"), "{ki:?}");
}
