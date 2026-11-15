//! Computed styles.
//!
//! Until the cascade exists, styles come only from the built-in defaults
//! below: a small subset of the HTML user-agent stylesheet, plus the
//! presentational attributes (`align`) that wkhtmltopdf-era templates use.
//! Author stylesheets and `style=""` are not applied yet.

use presswork_dom::ElementData;

/// The `display` value, limited to what layout supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    None,
    Block,
    Inline,
    ListItem,
    Table,
    TableRowGroup,
    TableRow,
    TableCell,
    TableCaption,
}

impl Display {
    /// Whether the box takes part in block layout (as opposed to inline).
    pub fn is_block_level(self) -> bool {
        !matches!(self, Display::Inline | Display::None)
    }
}

/// A generic font family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFamily {
    Serif,
    SansSerif,
    Monospace,
}

/// Horizontal alignment of inline content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
}

/// Sizes in points for the four sides of a box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// The style of one element, with every length resolved to points.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub font_family: FontFamily,
    /// Font size in points.
    pub font_size: f32,
    /// Font weight, 100 to 900.
    pub font_weight: f32,
    pub italic: bool,
    /// `white-space: pre`: keep spaces and line breaks, never wrap.
    pub preformatted: bool,
    pub text_align: TextAlign,
    /// Content language from the `lang` attribute, e.g. `"ne"`. Inherited.
    /// Used for font fallback and shaping.
    pub lang: Option<String>,
    pub margin: Edges,
    pub padding: Edges,
}

/// The browser default font size: 16px.
pub const DEFAULT_FONT_SIZE: f32 = 12.0;

/// Points per CSS pixel.
const PX: f32 = 0.75;

impl ComputedStyle {
    /// The style of the document root's parent: inherited defaults only.
    pub fn initial() -> Self {
        Self {
            display: Display::Block,
            font_family: FontFamily::Serif,
            font_size: DEFAULT_FONT_SIZE,
            font_weight: 400.0,
            italic: false,
            preformatted: false,
            text_align: TextAlign::Left,
            lang: None,
            margin: Edges::default(),
            padding: Edges::default(),
        }
    }

    /// Computes the style of `element`, whose parent has style `parent`.
    pub fn for_element(parent: &ComputedStyle, element: &ElementData) -> Self {
        // Inherited properties start from the parent; the rest reset.
        let mut style = Self {
            display: Display::Inline,
            margin: Edges::default(),
            padding: Edges::default(),
            ..parent.clone()
        };
        let em = |v: f32, size: f32| v * size;

        let name = element.name.as_str();
        style.display = display_for(name);

        // Font size first, since `em` margins depend on it.
        let scale = match name {
            "h1" => Some(2.0),
            "h2" => Some(1.5),
            "h3" => Some(1.17),
            "h5" => Some(0.83),
            "h6" => Some(0.67),
            "small" | "sub" | "sup" => Some(0.83),
            "big" => Some(1.2),
            _ => None,
        };
        if let Some(scale) = scale {
            style.font_size = parent.font_size * scale;
        }
        let size = style.font_size;

        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "b" | "strong" | "th" => {
                style.font_weight = 700.0;
            }
            "i" | "em" | "cite" | "var" | "dfn" | "address" => style.italic = true,
            "pre" | "code" | "kbd" | "samp" | "tt" => style.font_family = FontFamily::Monospace,
            _ => {}
        }
        if name == "pre" {
            style.preformatted = true;
        }

        let vertical = match name {
            "p" | "ul" | "ol" | "dl" | "pre" | "blockquote" | "figure" => Some(1.0),
            "h1" => Some(0.67),
            "h2" => Some(0.83),
            "h3" => Some(1.0),
            "h4" => Some(1.33),
            "h5" => Some(1.67),
            "h6" => Some(2.33),
            "hr" => Some(0.5),
            _ => None,
        };
        if let Some(v) = vertical {
            style.margin.top = em(v, size);
            style.margin.bottom = em(v, size);
        }
        match name {
            "body" => style.margin = uniform(8.0 * PX),
            "blockquote" | "figure" => {
                style.margin.left = 40.0 * PX;
                style.margin.right = 40.0 * PX;
            }
            "dd" => style.margin.left = 40.0 * PX,
            "ul" | "ol" => style.padding.left = 40.0 * PX,
            "td" | "th" => style.padding = uniform(1.0 * PX),
            _ => {}
        }

        if let Some(lang) = element.attr("lang").or_else(|| element.attr("xml:lang")) {
            let lang = lang.trim();
            style.lang = (!lang.is_empty()).then(|| lang.to_owned());
        }

        // Alignment: `<center>`, `<th>` and the `align` attribute.
        match name {
            "center" | "th" => style.text_align = TextAlign::Center,
            "table" => style.text_align = TextAlign::Left,
            _ => {}
        }
        if let Some(align) = element.attr("align") {
            match align.to_ascii_lowercase().as_str() {
                "left" => style.text_align = TextAlign::Left,
                "right" => style.text_align = TextAlign::Right,
                "center" | "middle" => style.text_align = TextAlign::Center,
                _ => {}
            }
        }
        style
    }
}

fn uniform(v: f32) -> Edges {
    Edges {
        top: v,
        right: v,
        bottom: v,
        left: v,
    }
}

fn display_for(name: &str) -> Display {
    match name {
        "head" | "script" | "style" | "title" | "meta" | "link" | "base" | "template"
        | "noembed" | "noframes" | "area" | "datalist" | "param" | "source" | "track" => {
            Display::None
        }
        "html" | "body" | "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol"
        | "dl" | "dt" | "dd" | "pre" | "blockquote" | "address" | "section" | "article"
        | "header" | "footer" | "nav" | "aside" | "main" | "hr" | "form" | "fieldset"
        | "figure" | "figcaption" | "center" | "legend" | "details" | "summary" | "menu"
        | "dir" | "hgroup" | "search" | "noscript" => Display::Block,
        "li" => Display::ListItem,
        "table" => Display::Table,
        "thead" | "tbody" | "tfoot" => Display::TableRowGroup,
        "tr" => Display::TableRow,
        "td" | "th" => Display::TableCell,
        "caption" => Display::TableCaption,
        _ => Display::Inline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presswork_dom::Attribute;

    fn element(name: &str, attrs: &[(&str, &str)]) -> ElementData {
        ElementData {
            name: name.into(),
            namespace: String::new(),
            attributes: attrs
                .iter()
                .map(|(n, v)| Attribute {
                    name: (*n).into(),
                    value: (*v).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn headings_are_bold_and_scaled() {
        let root = ComputedStyle::initial();
        let h1 = ComputedStyle::for_element(&root, &element("h1", &[]));
        assert_eq!(h1.display, Display::Block);
        assert_eq!(h1.font_size, 24.0);
        assert_eq!(h1.font_weight, 700.0);
        assert!((h1.margin.top - 0.67 * 24.0).abs() < 1e-4);
    }

    #[test]
    fn inherits_font_but_not_margins() {
        let root = ComputedStyle::initial();
        let p = ComputedStyle::for_element(&root, &element("p", &[]));
        let b = ComputedStyle::for_element(&p, &element("b", &[]));
        assert_eq!(b.display, Display::Inline);
        assert_eq!(b.font_weight, 700.0);
        assert_eq!(b.margin, Edges::default());
    }

    #[test]
    fn align_attribute_sets_text_align() {
        let root = ComputedStyle::initial();
        let td = ComputedStyle::for_element(&root, &element("td", &[("align", "RIGHT")]));
        assert_eq!(td.text_align, TextAlign::Right);
        let th = ComputedStyle::for_element(&root, &element("th", &[]));
        assert_eq!(th.text_align, TextAlign::Center);
    }

    #[test]
    fn hidden_elements() {
        let root = ComputedStyle::initial();
        for name in ["head", "script", "style", "template"] {
            let s = ComputedStyle::for_element(&root, &element(name, &[]));
            assert_eq!(s.display, Display::None, "{name}");
        }
    }
}
