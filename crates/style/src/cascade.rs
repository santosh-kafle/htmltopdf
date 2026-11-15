//! The cascade: deciding which declarations apply to an element.

use presswork_dom::{Document, NodeId};
use selectors::context::SelectorCaches;
use selectors::parser::Selector;

use crate::computed::ComputedStyle;
use crate::hints::presentational_hints;
use crate::properties::{Display, LonghandId, PropertyDeclaration};
use crate::selector::{ElementRef, Simple};
use crate::stylesheet::{Declaration, PageRule, Stylesheet, media_matches, parse_style_attribute};

/// The built-in stylesheet for HTML.
const USER_AGENT_CSS: &str = include_str!("ua.css");

/// Where a stylesheet comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    UserAgent,
    Author,
}

/// Cascade precedence, lowest first (CSS Cascade 4, section 6.1, without
/// user styles, animations or transitions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    UserAgent,
    /// Presentational hints: author level, below every author rule.
    Hint,
    Author,
    AuthorImportant,
    UserAgentImportant,
}

/// One selector of a style rule with the rule's declarations.
#[derive(Debug)]
struct CascadeRule {
    origin: Origin,
    selector: Selector<Simple>,
    specificity: u32,
    /// Index into [`Stylist::blocks`].
    block: usize,
}

/// Specificity of a `style=""` attribute: above any selector.
const INLINE_SPECIFICITY: u32 = u32::MAX;

/// All the styles of one document; computes the style of its elements.
#[derive(Debug)]
pub struct Stylist {
    /// In source order, which is also cascade order within a level.
    rules: Vec<CascadeRule>,
    blocks: Vec<Vec<Declaration>>,
    page: PageRule,
}

impl Default for Stylist {
    fn default() -> Self {
        Self::new()
    }
}

impl Stylist {
    /// A stylist with only the built-in user-agent stylesheet.
    pub fn new() -> Self {
        let mut stylist = Self {
            rules: Vec::new(),
            blocks: Vec::new(),
            page: PageRule::default(),
        };
        stylist.add(Origin::UserAgent, Stylesheet::parse(USER_AGENT_CSS));
        stylist
    }

    /// A stylist for `doc`: the user-agent stylesheet plus the document's
    /// `<style>` elements that apply to print.
    pub fn for_document(doc: &Document) -> Self {
        let mut stylist = Self::new();
        for node in doc.descendants(doc.root()) {
            let Some(element) = doc.element(node) else {
                continue;
            };
            let is_css = element
                .attr("type")
                .is_none_or(|t| t.trim().is_empty() || t.trim().eq_ignore_ascii_case("text/css"));
            let applies = element.attr("media").is_none_or(media_matches);
            if element.name == "style" && is_css && applies {
                stylist.add_stylesheet(&doc.text_content(node));
            }
        }
        stylist
    }

    /// Adds an author stylesheet, after any added earlier.
    pub fn add_stylesheet(&mut self, css: &str) {
        self.add(Origin::Author, Stylesheet::parse(css));
    }

    fn add(&mut self, origin: Origin, sheet: Stylesheet) {
        self.page.merge(&sheet.page);
        for rule in sheet.rules {
            let block = self.blocks.len();
            self.blocks.push(rule.declarations);
            for selector in rule.selectors.slice() {
                // Generated content is not supported, so rules for
                // pseudo-elements never apply.
                if selector.has_pseudo_element() {
                    continue;
                }
                self.rules.push(CascadeRule {
                    origin,
                    specificity: selector.specificity(),
                    selector: selector.clone(),
                    block,
                });
            }
        }
    }

    /// The document's `@page` descriptors.
    pub fn page(&self) -> &PageRule {
        &self.page
    }

    /// Computes the style of `node`. `parent` is the computed style of its
    /// parent element, or `None` for the root element.
    ///
    /// Non-element nodes get the inherited style.
    pub fn style(
        &self,
        doc: &Document,
        node: NodeId,
        parent: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        let initial;
        let parent = match parent {
            Some(parent) => parent,
            None => {
                initial = ComputedStyle::initial();
                &initial
            }
        };
        let mut style = ComputedStyle::inheriting(parent);
        let (Some(element), Some(data)) = (ElementRef::new(doc, node), doc.element(node)) else {
            return style;
        };

        if let Some(lang) = data.attr("lang").or_else(|| data.attr("xml:lang")) {
            let lang = lang.trim();
            style.lang = (!lang.is_empty()).then(|| lang.into());
        }

        // Gather every declaration that applies, with its cascade position.
        let mut applicable: Vec<(Level, u32, PropertyDeclaration)> = Vec::new();
        let mut caches = SelectorCaches::default();
        for rule in &self.rules {
            if !element.matches(&rule.selector, &mut caches) {
                continue;
            }
            for declaration in &self.blocks[rule.block] {
                let level = match (rule.origin, declaration.important) {
                    (Origin::UserAgent, false) => Level::UserAgent,
                    (Origin::UserAgent, true) => Level::UserAgentImportant,
                    (Origin::Author, false) => Level::Author,
                    (Origin::Author, true) => Level::AuthorImportant,
                };
                applicable.push((level, rule.specificity, declaration.property.clone()));
            }
        }
        for hint in presentational_hints(doc, node, data) {
            applicable.push((Level::Hint, 0, hint));
        }
        if let Some(inline) = data.attr("style") {
            for declaration in parse_style_attribute(inline) {
                let level = if declaration.important {
                    Level::AuthorImportant
                } else {
                    Level::Author
                };
                applicable.push((level, INLINE_SPECIFICITY, declaration.property));
            }
        }

        // Stable sort: ties keep source order, so later declarations win.
        applicable.sort_by_key(|(level, specificity, _)| (*level, *specificity));

        // `em` lengths depend on the element's font size: apply it first.
        let is_root = doc.document_element() == Some(node);
        let is_font_size = |d: &PropertyDeclaration| d.id() == LonghandId::FontSize;
        for (_, _, declaration) in applicable.iter().filter(|(_, _, d)| is_font_size(d)) {
            style.apply(declaration, parent);
        }
        if is_root {
            style.root_font_size = style.font_size;
        }
        for (_, _, declaration) in applicable.iter().filter(|(_, _, d)| !is_font_size(d)) {
            style.apply(declaration, parent);
        }

        // The root element always generates a block box.
        if is_root && style.display == Display::Inline {
            style.display = Display::Block;
        }
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::properties::{TextAlign, VerticalAlign};
    use crate::values::{Color, Dimension};
    use presswork_dom::parse_html;

    /// The computed style of the first `target` element in `html`.
    fn style_of(html: &str, target: &str) -> ComputedStyle {
        let doc = parse_html(html.as_bytes());
        let stylist = Stylist::for_document(&doc);
        let node = doc.find_element(target).expect("target exists");

        let mut chain = vec![node];
        while let Some(parent) = doc.node(*chain.last().unwrap()).parent {
            if doc.element(parent).is_none() {
                break;
            }
            chain.push(parent);
        }
        let mut style = None;
        for &id in chain.iter().rev() {
            style = Some(stylist.style(&doc, id, style.as_ref()));
        }
        style.unwrap()
    }

    const RED: Color = Color::rgb(255, 0, 0);
    const BLUE: Color = Color::rgb(0, 0, 255);

    #[test]
    fn user_agent_defaults() {
        let h1 = style_of("<h1>x</h1>", "h1");
        assert_eq!(h1.display, Display::Block);
        assert_eq!(h1.font_size, 24.0);
        assert_eq!(h1.font_weight, 700.0);
        assert!((h1.margin.top.resolve(0.0).unwrap() - 0.67 * 24.0).abs() < 1e-3);
        assert_eq!(
            style_of("<script>x</script>", "script").display,
            Display::None
        );
        assert_eq!(style_of("<p hidden>x</p>", "p").display, Display::None);
        assert_eq!(style_of("<body>", "body").margin.left, Dimension::Pt(6.0));
    }

    #[test]
    fn specificity_beats_source_order() {
        let html = "<style>#a { color: red } p.x { color: blue } p { color: green }</style>
                    <p id=a class=x>";
        assert_eq!(style_of(html, "p").color, RED);
    }

    #[test]
    fn later_rules_win_ties() {
        let html = "<style>p { color: red } p { color: blue }</style><p>";
        assert_eq!(style_of(html, "p").color, BLUE);
    }

    #[test]
    fn important_and_inline_styles() {
        let html = "<style>p { color: red !important } #a { color: green }</style>
                    <p id=a style='color: blue'>";
        assert_eq!(style_of(html, "p").color, RED);
        let html = "<style>#a { color: red }</style><p id=a style='color: blue'>";
        assert_eq!(style_of(html, "p").color, BLUE);
        let html = "<style>p { color: red !important }</style>
                    <p style='color: blue !important'>";
        assert_eq!(style_of(html, "p").color, BLUE);
    }

    #[test]
    fn author_rules_override_hints_and_hints_override_the_user_agent() {
        let th = style_of("<table><tr><th align=right>", "th");
        assert_eq!(th.text_align, TextAlign::Right);
        let th = style_of(
            "<style>th { text-align: left }</style><table><tr><th align=right>",
            "th",
        );
        assert_eq!(th.text_align, TextAlign::Left);
    }

    #[test]
    fn inheritance_and_em_units() {
        let html = "<style>div { font-size: 20pt; color: red } p { margin-left: 2em; font-size: 50% }</style>
                    <div><p><b>x</b></p></div>";
        let p = style_of(html, "p");
        assert_eq!(p.font_size, 10.0);
        assert_eq!(p.margin.left, Dimension::Pt(20.0));
        let b = style_of(html, "b");
        assert_eq!((b.color, b.font_size, b.font_weight), (RED, 10.0, 700.0));
        assert_eq!(b.margin.left, Dimension::ZERO);
    }

    #[test]
    fn rem_uses_the_root_font_size() {
        let html =
            "<style>html { font-size: 10pt } p { font-size: 30pt; padding-left: 2rem }</style><p>";
        assert_eq!(style_of(html, "p").padding.left, Dimension::Pt(20.0));
    }

    #[test]
    fn cells_inherit_vertical_align_from_the_row_group() {
        let html = "<table><tbody valign=bottom><tr><td>";
        assert_eq!(style_of(html, "td").vertical_align, VerticalAlign::Bottom);
        assert_eq!(
            style_of("<table><tr><td>", "td").vertical_align,
            VerticalAlign::Middle
        );
    }

    #[test]
    fn lang_is_inherited() {
        let html = "<html lang=ne><body><p><span>";
        assert_eq!(style_of(html, "span").lang.as_deref(), Some("ne"));
    }

    #[test]
    fn style_elements_for_other_media_are_ignored() {
        let html = "<style media=screen>p { color: red }</style>
                    <style media=print>p { font-size: 20pt }</style><p>";
        let p = style_of(html, "p");
        assert_eq!((p.color, p.font_size), (Color::BLACK, 20.0));
    }

    #[test]
    fn page_rule_is_collected() {
        let doc = parse_html(b"<style>@page { margin: 1in }</style>");
        assert_eq!(Stylist::for_document(&doc).page().margin.top, Some(72.0));
    }
}
