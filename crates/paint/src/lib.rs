//! PDF output.
//!
//! Draws the pages produced by `presswork-layout` with `krilla`, which embeds
//! subsetted fonts and maps glyphs back to text so the PDF can be searched
//! and copied from, and embeds images.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use krilla::color::rgb;
use krilla::error::KrillaError;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image as PdfImage;
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::Fill;
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Data, Document};
use presswork_layout::{Color, FontData, Image, ImageData, ImageFormat, Item, Page, TextRun};

/// Document-level options for the generated PDF.
#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    /// `Title` entry in the PDF metadata.
    pub title: Option<String>,
    /// `Author` entry in the PDF metadata.
    pub author: Option<String>,
}

/// Writing the PDF failed.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to write PDF: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// A written PDF.
#[derive(Debug)]
pub struct Pdf {
    pub bytes: Vec<u8>,
    /// Images left out because they could not be decoded, by source.
    pub broken_images: Vec<String>,
}

/// Renders laid-out pages to a PDF.
pub fn render_pdf(pages: &[Page], options: &PdfOptions) -> Result<Pdf, Error> {
    if let Some(page) = pages
        .iter()
        .find(|page| PageSettings::from_wh(page.size.width, page.size.height).is_none())
    {
        return Err(Error(format!("invalid page size {:?}", page.size)));
    }
    // krilla decodes images only when the document is finished, and one
    // that fails to decode fails the whole document. Leave such images out
    // and write the document again.
    let mut skip = HashSet::new();
    loop {
        let mut images = ImageCache::new(&skip);
        match write(pages, options, &mut images) {
            Ok(bytes) => {
                return Ok(Pdf {
                    bytes,
                    broken_images: images.broken,
                });
            }
            Err(KrillaError::Image(image, _, message)) => match images.key_of(&image) {
                Some(key) if skip.insert(key) => {}
                _ => return Err(Error(format!("image could not be decoded: {message}"))),
            },
            Err(error) => return Err(Error(error.to_string())),
        }
    }
}

fn write(
    pages: &[Page],
    options: &PdfOptions,
    images: &mut ImageCache<'_>,
) -> Result<Vec<u8>, KrillaError> {
    let mut document = Document::new();
    let mut fonts = FontCache::default();

    for page in pages {
        let Some(settings) = PageSettings::from_wh(page.size.width, page.size.height) else {
            unreachable!("page sizes are checked before writing");
        };
        let mut pdf_page = document.start_page_with(settings);
        let mut surface = pdf_page.surface();
        for item in &page.items {
            match item {
                Item::Text(run) => {
                    if let Some(font) = fonts.get(&run.font) {
                        draw_text(&mut surface, run, font);
                    }
                }
                Item::Image(image) => {
                    if let Some(pdf_image) = images.get(&image.data) {
                        draw_image(&mut surface, image, pdf_image);
                    }
                }
                Item::Fill(fill) => {
                    let rect = fill.rect;
                    let Some(rect) = Rect::from_xywh(
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                    ) else {
                        continue;
                    };
                    let mut path = PathBuilder::new();
                    path.push_rect(rect);
                    if let Some(path) = path.finish() {
                        set_color(&mut surface, fill.color);
                        surface.draw_path(&path);
                    }
                }
            }
        }
        surface.finish();
        pdf_page.finish();
    }

    let mut metadata = Metadata::new().creator(format!("presswork {}", env!("CARGO_PKG_VERSION")));
    if let Some(title) = &options.title {
        metadata = metadata.title(title.clone());
    }
    if let Some(author) = &options.author {
        metadata = metadata.authors(vec![author.clone()]);
    }
    document.set_metadata(metadata);
    document.finish()
}

/// Draws `image` scaled into its rectangle.
fn draw_image(surface: &mut Surface<'_>, image: &Image, pdf_image: PdfImage) {
    let rect = image.rect;
    let Some(size) = Size::from_wh(rect.size.width, rect.size.height) else {
        return;
    };
    surface.push_transform(&Transform::from_translate(rect.origin.x, rect.origin.y));
    surface.draw_image(pdf_image, size);
    surface.pop();
}

/// Images prepared for krilla, keyed by the identity of their data.
struct ImageCache<'a> {
    images: HashMap<usize, Option<PdfImage>>,
    /// Images to leave out: they failed to decode on an earlier attempt.
    skip: &'a HashSet<usize>,
    broken: Vec<String>,
}

impl<'a> ImageCache<'a> {
    fn new(skip: &'a HashSet<usize>) -> Self {
        Self {
            images: HashMap::new(),
            skip,
            broken: Vec::new(),
        }
    }

    fn key(data: &Arc<ImageData>) -> usize {
        Arc::as_ptr(data) as usize
    }

    fn get(&mut self, data: &Arc<ImageData>) -> Option<PdfImage> {
        let key = Self::key(data);
        if let Some(image) = self.images.get(&key) {
            return image.clone();
        }
        let image = if self.skip.contains(&key) {
            None
        } else {
            let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = data.bytes.clone();
            let bytes = Data::from(bytes);
            // Smooth scaling, as browsers do.
            let image = match data.format {
                ImageFormat::Png => PdfImage::from_png(bytes, true),
                ImageFormat::Jpeg => PdfImage::from_jpeg(bytes, true),
                ImageFormat::Gif => PdfImage::from_gif(bytes, true),
                ImageFormat::Webp => PdfImage::from_webp(bytes, true),
            };
            image.ok()
        };
        if image.is_none() {
            self.broken.push(data.source.clone());
        }
        self.images.insert(key, image.clone());
        image
    }

    /// The key of a krilla image made by this cache.
    fn key_of(&self, image: &PdfImage) -> Option<usize> {
        self.images
            .iter()
            .find(|(_, cached)| cached.as_ref() == Some(image))
            .map(|(key, _)| *key)
    }
}

/// Sets the fill color for the drawing operations that follow.
fn set_color(surface: &mut Surface<'_>, color: Color) {
    let opacity = NormalizedF32::new(f32::from(color.a) / 255.0).unwrap_or(NormalizedF32::ONE);
    surface.set_fill(Some(Fill {
        paint: rgb::Color::new(color.r, color.g, color.b).into(),
        opacity,
        rule: Default::default(),
    }));
}

fn draw_text(surface: &mut Surface<'_>, run: &TextRun, font: Font) {
    let size = run.font_size;
    if size <= 0.0 || run.color.is_transparent() {
        return;
    }
    set_color(surface, run.color);
    // krilla wants advances and offsets as fractions of the font size, with
    // y pointing up.
    let glyphs: Vec<KrillaGlyph> = run
        .glyphs
        .iter()
        .map(|g| {
            KrillaGlyph::new(
                GlyphId::new(g.id),
                g.advance / size,
                g.x_offset / size,
                -g.y_offset / size,
                0.0,
                g.text_range.clone(),
                None,
            )
        })
        .collect();
    surface.draw_glyphs(
        Point::from_xy(run.origin.x, run.origin.y),
        &glyphs,
        font,
        &run.text,
        size,
        false,
    );
}

/// Parsed fonts, keyed by the font blob and collection index.
#[derive(Default)]
struct FontCache {
    fonts: HashMap<(u64, u32), Option<Font>>,
}

impl FontCache {
    fn get(&mut self, data: &FontData) -> Option<Font> {
        self.fonts
            .entry((data.data.id(), data.index))
            .or_insert_with(|| {
                let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(data.data.clone());
                Font::new(bytes.into(), data.index)
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presswork_layout::{DataUrls, Engine, LayoutOptions};
    use presswork_style::Stylist;

    fn pages(html: &str) -> Vec<Page> {
        let doc = presswork_dom::parse_html(html.as_bytes());
        let styles = Stylist::for_document(&doc);
        let output = Engine::new().layout(&doc, &styles, &LayoutOptions::default(), &DataUrls);
        assert_eq!(output.failures, []);
        output.pages
    }

    fn contains(pdf: &[u8], needle: &[u8]) -> bool {
        pdf.windows(needle.len()).any(|w| w == needle)
    }

    /// A 2x1 PNG: one red pixel, one blue.
    const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DAwPAfAAcAAf9+CLHQAAAAAElFTkSuQmCC";

    #[test]
    fn writes_a_pdf_with_text() {
        let pages = pages("<p style='color: #c00; background: #eee; border: 1px solid'>Hello</p>");
        let pdf = render_pdf(
            &pages,
            &PdfOptions {
                title: Some("Test".into()),
                author: None,
            },
        )
        .unwrap();
        assert!(pdf.bytes.starts_with(b"%PDF-"));
        assert!(contains(&pdf.bytes, b"Font"), "font is embedded");
    }

    #[test]
    fn embeds_images() {
        let pages = pages(&format!("<img src='{PNG}' width=200>"));
        let pdf = render_pdf(&pages, &PdfOptions::default()).unwrap();
        assert_eq!(pdf.broken_images, Vec::<String>::new());
        assert!(contains(&pdf.bytes, b"/Subtype/Image"), "image is embedded");
    }

    #[test]
    fn leaves_out_images_that_do_not_decode() {
        // A valid PNG header and size, then garbage instead of pixel data.
        let mut png = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        png.extend([0, 0, 0, 13]);
        png.extend(b"IHDR");
        png.extend([0, 0, 0, 2, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0, 0, 0, 0]);
        png.extend([0xde; 64]);
        let data = Arc::new(ImageData::new(png, "broken.png").unwrap());
        let page = Page {
            size: presswork_layout::Size {
                width: 100.0,
                height: 100.0,
            },
            items: vec![Item::Image(Image {
                rect: presswork_layout::Rect::new(0.0, 0.0, 10.0, 10.0),
                data,
            })],
        };
        let pdf = render_pdf(&[page], &PdfOptions::default()).unwrap();
        assert!(pdf.bytes.starts_with(b"%PDF-"));
        assert_eq!(pdf.broken_images, ["broken.png"]);
    }
}
