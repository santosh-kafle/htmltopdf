//! Generated content: the text of `::before` and `::after` boxes.
//!
//! Counters and quotes depend on everything before them in the document,
//! while layout visits some subtrees more than once (table cells and
//! inline-blocks are measured, then laid out). So the text is resolved in
//! one pass over the document before layout, and layout only looks it up.

use std::collections::HashMap;
use std::sync::Arc;

use presswork_dom::{Document, NodeId};
use presswork_style::{
    ComputedStyle, Content, ContentItem, Display, ListStyleType, Pseudo, Stylist,
};

/// A `::before` or `::after` box: its style and its text.
#[derive(Debug)]
pub(crate) struct PseudoBox {
    pub style: ComputedStyle,
    pub text: String,
}

/// The generated boxes of one element.
#[derive(Debug, Default)]
pub(crate) struct Generated {
    pub before: Option<PseudoBox>,
    pub after: Option<PseudoBox>,
}

impl Generated {
    pub fn get(&self, pseudo: Pseudo) -> Option<&PseudoBox> {
        match pseudo {
            Pseudo::Before => self.before.as_ref(),
            Pseudo::After => self.after.as_ref(),
        }
    }
}

/// The generated boxes of a document, by element.
pub(crate) type GeneratedContent = HashMap<NodeId, Generated>;

/// Elements that never have `::before` or `::after` boxes: replaced and
/// void elements.
const NO_PSEUDO: [&str; 5] = ["img", "br", "input", "select", "textarea"];

/// Quote marks by nesting depth (the initial value of `quotes` in
/// browsers for most languages).
const QUOTES: [(&str, &str); 2] = [("\u{201C}", "\u{201D}"), ("\u{2018}", "\u{2019}")];

/// Resolves the generated content of `doc`.
pub(crate) fn resolve(doc: &Document, styles: &Stylist) -> GeneratedContent {
    let mut resolver = Resolver {
        doc,
        styles,
        counters: Vec::new(),
        quote_depth: 0,
        out: GeneratedContent::new(),
    };
    // The user-agent stylesheet only generates quotes for `<q>`: without
    // one, and without rules of the document's own, nothing to do.
    let needed = styles.has_generated_content() || doc.find_element("q").is_some();
    if needed && let Some(root) = doc.document_element() {
        let style = styles.style(doc, root, None);
        resolver.element(root, &style, None);
    }
    resolver.out
}

/// A counter instance (CSS Lists 3, section 4.4).
struct Counter {
    name: Arc<str>,
    value: i32,
    /// The parent of the element that created it: the counter is in scope
    /// for that element's following siblings and their descendants, so it
    /// ends with the parent.
    scope: Option<NodeId>,
}

struct Resolver<'a> {
    doc: &'a Document,
    styles: &'a Stylist,
    /// Innermost last.
    counters: Vec<Counter>,
    quote_depth: usize,
    out: GeneratedContent,
}

impl Resolver<'_> {
    fn element(&mut self, node: NodeId, style: &ComputedStyle, parent: Option<NodeId>) {
        // Elements without boxes do not change counters.
        if style.display == Display::None {
            return;
        }
        self.change_counters(style, parent);
        let name = self.doc.element(node).map(|e| e.name.as_str());
        let pseudos = !name.is_some_and(|name| NO_PSEUDO.contains(&name));

        let mut generated = Generated::default();
        if pseudos {
            generated.before = self.pseudo(node, Pseudo::Before, style);
        }
        for child in self.doc.children(node) {
            if self.doc.element(child).is_some() {
                let child_style = self.styles.style(self.doc, child, Some(style));
                self.element(child, &child_style, Some(node));
            }
        }
        if pseudos {
            generated.after = self.pseudo(node, Pseudo::After, style);
        }
        if generated.before.is_some() || generated.after.is_some() {
            self.out.insert(node, generated);
        }
        // Counters created by the children (and the pseudo-elements) end
        // with this element.
        self.counters.retain(|counter| counter.scope != Some(node));
    }

    /// The `pseudo` box of `node`, if it has one. Its counter changes
    /// apply, as a child of `node`.
    fn pseudo(&mut self, node: NodeId, pseudo: Pseudo, style: &ComputedStyle) -> Option<PseudoBox> {
        let pseudo_style = self.styles.pseudo_style(self.doc, node, pseudo, style)?;
        self.change_counters(&pseudo_style, Some(node));
        let Content::Items(items) = &pseudo_style.content else {
            return None;
        };
        let text = self.text(node, &Arc::clone(items));
        Some(PseudoBox {
            style: pseudo_style,
            text,
        })
    }

    /// Applies `counter-reset`, then `counter-increment`, then
    /// `counter-set` of an element whose parent is `parent`.
    fn change_counters(&mut self, style: &ComputedStyle, parent: Option<NodeId>) {
        for (name, value) in style.counter_reset.iter() {
            // A sibling's counter of the same name is replaced, not nested.
            match self.counters.iter_mut().rev().find(|c| c.name == *name) {
                Some(counter) if counter.scope == parent => counter.value = *value,
                _ => self.counters.push(Counter {
                    name: Arc::clone(name),
                    value: *value,
                    scope: parent,
                }),
            }
        }
        for (name, value) in style.counter_increment.iter() {
            self.innermost(name, parent).value += *value;
        }
        for (name, value) in style.counter_set.iter() {
            self.innermost(name, parent).value = *value;
        }
    }

    /// The innermost counter `name`, created at zero if there is none.
    fn innermost(&mut self, name: &Arc<str>, parent: Option<NodeId>) -> &mut Counter {
        let index = match self.counters.iter().rposition(|c| c.name == *name) {
            Some(index) => index,
            None => {
                self.counters.push(Counter {
                    name: Arc::clone(name),
                    value: 0,
                    scope: parent,
                });
                self.counters.len() - 1
            }
        };
        &mut self.counters[index]
    }

    /// The text of a `content` value on a pseudo-element of `node`.
    fn text(&mut self, node: NodeId, items: &[ContentItem]) -> String {
        let mut text = String::new();
        for item in items {
            match item {
                ContentItem::Text(value) => text.push_str(value),
                ContentItem::Counter { name, style } => {
                    let value = self
                        .counters
                        .iter()
                        .rev()
                        .find(|c| c.name == *name)
                        .map_or(0, |c| c.value);
                    text.push_str(&counter_text(i64::from(value), *style));
                }
                ContentItem::Counters {
                    name,
                    separator,
                    style,
                } => {
                    let values: Vec<String> = self
                        .counters
                        .iter()
                        .filter(|c| c.name == *name)
                        .map(|c| counter_text(i64::from(c.value), *style))
                        .collect();
                    if values.is_empty() {
                        text.push_str(&counter_text(0, *style));
                    } else {
                        text.push_str(&values.join(separator));
                    }
                }
                ContentItem::Attr(name) => {
                    if let Some(value) = self.doc.element(node).and_then(|e| e.attr(name)) {
                        text.push_str(value);
                    }
                }
                ContentItem::OpenQuote => {
                    text.push_str(QUOTES[self.quote_depth.min(QUOTES.len() - 1)].0);
                    self.quote_depth += 1;
                }
                ContentItem::CloseQuote => {
                    if self.quote_depth > 0 {
                        self.quote_depth -= 1;
                        text.push_str(QUOTES[self.quote_depth.min(QUOTES.len() - 1)].1);
                    }
                }
                ContentItem::NoOpenQuote => self.quote_depth += 1,
                ContentItem::NoCloseQuote => self.quote_depth = self.quote_depth.saturating_sub(1),
            }
        }
        text
    }
}

/// A counter value in a list style: `3` → `3`, `c`, `iii`, `३`.
pub(super) fn counter_text(value: i64, style: ListStyleType) -> String {
    match style {
        ListStyleType::None => String::new(),
        ListStyleType::Disc => "•".into(),
        ListStyleType::Circle => "◦".into(),
        ListStyleType::Square => "▪".into(),
        ListStyleType::Decimal => value.to_string(),
        ListStyleType::DecimalLeadingZero => match value {
            0..=9 => format!("0{value}"),
            -9..=-1 => format!("-0{}", -value),
            _ => value.to_string(),
        },
        ListStyleType::Devanagari => value
            .to_string()
            .chars()
            .map(|ch| match ch.to_digit(10) {
                Some(digit) => char::from_u32(0x0966 + digit).unwrap_or(ch),
                None => ch,
            })
            .collect(),
        ListStyleType::LowerAlpha => alphabetic(value, false),
        ListStyleType::UpperAlpha => alphabetic(value, true),
        ListStyleType::LowerRoman => roman(value, false),
        ListStyleType::UpperRoman => roman(value, true),
    }
}

/// `1` → `a`, `26` → `z`, `27` → `aa`. Falls back to decimal below 1.
fn alphabetic(mut n: i64, upper: bool) -> String {
    if n < 1 {
        return n.to_string();
    }
    let base = if upper { b'A' } else { b'a' };
    let mut letters = Vec::new();
    while n > 0 {
        n -= 1;
        letters.push(base + (n % 26) as u8);
        n /= 26;
    }
    letters.reverse();
    String::from_utf8(letters).expect("ASCII letters")
}

/// Roman numerals for 1 to 3999; decimal outside that range.
fn roman(mut n: i64, upper: bool) -> String {
    if !(1..=3999).contains(&n) {
        return n.to_string();
    }
    const NUMERALS: [(i64, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, numeral) in NUMERALS {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    if upper { out.to_uppercase() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_styles() {
        assert_eq!(counter_text(1, ListStyleType::LowerAlpha), "a");
        assert_eq!(counter_text(27, ListStyleType::UpperAlpha), "AA");
        assert_eq!(counter_text(1994, ListStyleType::UpperRoman), "MCMXCIV");
        assert_eq!(counter_text(4, ListStyleType::LowerRoman), "iv");
        assert_eq!(counter_text(0, ListStyleType::LowerRoman), "0");
        assert_eq!(counter_text(7, ListStyleType::DecimalLeadingZero), "07");
        assert_eq!(counter_text(2081, ListStyleType::Devanagari), "२०८१");
        assert_eq!(counter_text(-5, ListStyleType::Devanagari), "-५");
    }
}
