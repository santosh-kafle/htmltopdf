//! Stylesheet parsing: style rules, `@media`, `@page` and `style=""`.

use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, DeclarationParser, Delimiter, ParseError, Parser,
    ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
    parse_important,
};
use selectors::SelectorList;

use crate::properties::{PropertyDeclaration, parse_declaration};
use crate::selector::{Simple, parse_selector_list};
use crate::values::{Invalid, Length, PT_PER_PX, Side, Sides};

/// One longhand declaration and whether it is `!important`.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub property: PropertyDeclaration,
    pub important: bool,
}

/// A style rule: `selectors { declarations }`.
#[derive(Debug, Clone)]
pub(crate) struct StyleRule {
    pub selectors: SelectorList<Simple>,
    pub declarations: Vec<Declaration>,
}

/// The `size` descriptor of `@page`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageSizeValue {
    /// A named size such as `A4` or `letter`.
    pub name: Option<String>,
    /// Explicit width and height in points.
    pub dimensions: Option<(f32, f32)>,
    /// `Some(true)` for `landscape`, `Some(false)` for `portrait`.
    pub landscape: Option<bool>,
}

/// The descriptors of the document's `@page` rules.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageRule {
    pub size: Option<PageSizeValue>,
    /// Page margins in points, where given.
    pub margin: Sides<Option<f32>>,
}

impl PageRule {
    /// Overrides this rule's descriptors with those set in `later`.
    pub(crate) fn merge(&mut self, later: &PageRule) {
        if later.size.is_some() {
            self.size.clone_from(&later.size);
        }
        for side in Side::ALL {
            if let Some(margin) = later.margin.get(side) {
                self.margin.set(side, Some(margin));
            }
        }
    }
}

/// A parsed stylesheet.
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub(crate) rules: Vec<StyleRule>,
    pub(crate) page: PageRule,
}

impl Stylesheet {
    /// Parses a stylesheet. Invalid rules and declarations are skipped, as
    /// CSS error handling requires.
    ///
    /// `@media` rules apply when they match print media; `@import`,
    /// `@font-face` and other at-rules are ignored.
    pub fn parse(css: &str) -> Self {
        let mut parser = Parser::new(css);
        let mut sheet = Stylesheet::default();
        sheet.extend(parse_rules(&mut parser));
        sheet
    }

    fn extend(&mut self, rules: Vec<Rule>) {
        for rule in rules {
            match rule {
                Rule::Style(rule) => self.rules.push(rule),
                Rule::Nested(rules) => self.extend(rules),
                Rule::Page(page) => self.page.merge(&page),
            }
        }
    }
}

/// Parses the contents of a `style=""` attribute.
pub fn parse_style_attribute(css: &str) -> Vec<Declaration> {
    let input = css;
    parse_declarations(&mut Parser::new(input))
}

/// Whether a media query list (as in `@media` or `<style media>`) matches
/// print output.
pub fn media_matches(query: &str) -> bool {
    let input = query;
    evaluate_media(&mut Parser::new(input))
}

enum Rule {
    Style(StyleRule),
    /// The rules inside a conditional group rule that applies.
    Nested(Vec<Rule>),
    Page(PageRule),
}

enum AtRulePrelude {
    /// `@media` or `@supports`, with whether its contents apply.
    Conditional(bool),
    /// `@page`, with whether it has no page selector.
    Page(bool),
}

struct RuleParser;

fn parse_rules(input: &mut Parser<'_>) -> Vec<Rule> {
    StyleSheetParser::new(input, &mut RuleParser)
        .flatten()
        .collect()
}

fn skip_rest(input: &mut Parser<'_>) {
    while input.next().is_ok() {}
}

impl<'i> QualifiedRuleParser<'i> for RuleParser {
    type Prelude = SelectorList<Simple>;
    type QualifiedRule = Rule;
    type Error = Invalid;

    fn parse_prelude(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        parse_selector_list(input).map_err(ParseError::custom)
    }

    fn parse_block(
        &mut self,
        selectors: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Rule, ParseError<Self::Error>> {
        Ok(Rule::Style(StyleRule {
            selectors,
            declarations: parse_declarations(input),
        }))
    }
}

impl<'i> AtRuleParser<'i> for RuleParser {
    type Prelude = AtRulePrelude;
    type AtRule = Rule;
    type Error = Invalid;

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        match name.to_ascii_lowercase().as_str() {
            "media" => Ok(AtRulePrelude::Conditional(evaluate_media(input))),
            "supports" => {
                // Feature queries are assumed to pass; unsupported
                // declarations inside are dropped individually anyway.
                skip_rest(input);
                Ok(AtRulePrelude::Conditional(true))
            }
            "page" => {
                let plain = input.is_exhausted();
                skip_rest(input);
                Ok(AtRulePrelude::Page(plain))
            }
            _ => Err(ParseError::from_basic_kind(
                BasicParseErrorKind::AtRuleInvalid,
            )),
        }
    }

    fn parse_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Rule, ParseError<Self::Error>> {
        Ok(match prelude {
            AtRulePrelude::Conditional(true) => Rule::Nested(parse_rules(input)),
            AtRulePrelude::Page(true) => Rule::Page(parse_page_rule(input)),
            AtRulePrelude::Conditional(false) | AtRulePrelude::Page(false) => {
                skip_rest(input);
                Rule::Nested(Vec::new())
            }
        })
    }
}

/// Parses a declaration block: the body of a style rule or a `style=""`.
fn parse_declarations(input: &mut Parser<'_>) -> Vec<Declaration> {
    RuleBodyParser::new(input, &mut DeclarationListParser)
        .flatten()
        .flatten()
        .collect()
}

struct DeclarationListParser;

impl<'i> DeclarationParser<'i> for DeclarationListParser {
    type Declaration = Vec<Declaration>;
    type Error = Invalid;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<Vec<Declaration>, ParseError<Invalid>> {
        let mut properties = Vec::new();
        input.parse_until_before(Delimiter::Bang, |input| {
            parse_declaration(&name, input, &mut properties).map_err(ParseError::custom)
        })?;
        let important = input.try_parse(parse_important).is_ok();
        input.expect_exhausted()?;
        Ok(properties
            .into_iter()
            .map(|property| Declaration {
                property,
                important,
            })
            .collect())
    }
}

// Declaration blocks contain no nested rules; these use the defaults,
// which reject everything.
impl AtRuleParser<'_> for DeclarationListParser {
    type Prelude = ();
    type AtRule = Vec<Declaration>;
    type Error = Invalid;
}

impl QualifiedRuleParser<'_> for DeclarationListParser {
    type Prelude = ();
    type QualifiedRule = Vec<Declaration>;
    type Error = Invalid;
}

impl RuleBodyItemParser<'_, Vec<Declaration>, Invalid> for DeclarationListParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// Parses the descriptors of an `@page` rule. Margin boxes and descriptors
/// other than `size` and `margin` are ignored.
fn parse_page_rule(input: &mut Parser<'_>) -> PageRule {
    let mut parser = PageDescriptorParser(PageRule::default());
    for _ in RuleBodyParser::new(input, &mut parser) {}
    parser.0
}

struct PageDescriptorParser(PageRule);

/// A page length in points; `em` is relative to the default 12pt.
fn page_length(input: &mut Parser<'_>) -> Result<f32, Invalid> {
    Length::parse_non_negative(input).map(|l| l.to_pt(12.0, 12.0))
}

impl<'i> DeclarationParser<'i> for PageDescriptorParser {
    type Declaration = ();
    type Error = Invalid;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<(), ParseError<Invalid>> {
        let name = name.to_ascii_lowercase();
        let side = |name: &str| match name {
            "margin-top" => Some(Side::Top),
            "margin-right" => Some(Side::Right),
            "margin-bottom" => Some(Side::Bottom),
            "margin-left" => Some(Side::Left),
            _ => None,
        };
        if name == "size" {
            let size = parse_page_size(input).map_err(ParseError::custom)?;
            input.expect_exhausted()?;
            self.0.size = Some(size);
        } else if name == "margin" {
            let mut values = vec![page_length(input).map_err(ParseError::custom)?];
            while values.len() < 4
                && let Ok(value) = input.try_parse(page_length)
            {
                values.push(value);
            }
            input.expect_exhausted()?;
            let (top, right, bottom, left) = match values[..] {
                [a] => (a, a, a, a),
                [a, b] => (a, b, a, b),
                [a, b, c] => (a, b, c, b),
                _ => (values[0], values[1], values[2], values[3]),
            };
            self.0.margin = Sides {
                top: Some(top),
                right: Some(right),
                bottom: Some(bottom),
                left: Some(left),
            };
        } else if let Some(side) = side(&name) {
            let value = page_length(input).map_err(ParseError::custom)?;
            input.expect_exhausted()?;
            self.0.margin.set(side, Some(value));
        } else {
            return Err(ParseError::custom(Invalid));
        }
        Ok(())
    }
}

impl AtRuleParser<'_> for PageDescriptorParser {
    type Prelude = ();
    type AtRule = ();
    type Error = Invalid;
}

impl QualifiedRuleParser<'_> for PageDescriptorParser {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = Invalid;
}

impl RuleBodyItemParser<'_, (), Invalid> for PageDescriptorParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// `size: A4 landscape`, `size: 210mm 297mm`, `size: landscape`, ...
fn parse_page_size(input: &mut Parser<'_>) -> Result<PageSizeValue, Invalid> {
    let mut size = PageSizeValue::default();
    if let Ok(width) = input.try_parse(page_length) {
        let height = input.try_parse(page_length).unwrap_or(width);
        size.dimensions = Some((width, height));
        return Ok(size);
    }
    while let Ok(ident) = input.try_parse(|i| i.expect_ident_cloned()) {
        match ident.to_ascii_lowercase().as_str() {
            "landscape" => size.landscape = Some(true),
            "portrait" => size.landscape = Some(false),
            "auto" => {}
            name if size.name.is_none() => size.name = Some(name.to_owned()),
            _ => return Err(Invalid),
        }
    }
    Ok(size)
}

/// The viewport width media features are evaluated against: an A4 page.
const MEDIA_WIDTH_PX: f32 = 595.28 / PT_PER_PX;

/// Evaluates a media query list for print output, consuming the input.
///
/// Media types are honored (`print` and `all` match, `screen` does not);
/// `min-width` / `max-width` compare against an A4 page; other features
/// are assumed to match.
fn evaluate_media(input: &mut Parser<'_>) -> bool {
    let mut any = false;
    let mut empty = true;
    loop {
        let query = input.parse_until_before(Delimiter::Comma, |input| {
            let (mut negated, mut type_matches, mut features_match) = (false, true, true);
            let mut tokens = 0;
            while let Ok(token) = input.next() {
                tokens += 1;
                match token.clone() {
                    Token::Ident(ident) => match ident.to_ascii_lowercase().as_str() {
                        "not" => negated = true,
                        "only" | "and" => {}
                        "all" | "print" => type_matches = true,
                        _ => type_matches = false,
                    },
                    Token::ParenthesisBlock => {
                        let matches = input
                            .parse_nested_block(|i| {
                                Ok::<_, ParseError<Invalid>>(evaluate_feature(i))
                            })
                            .unwrap_or(true);
                        features_match &= matches;
                    }
                    _ => {}
                }
            }
            Ok::<_, ParseError<Invalid>>(
                (tokens > 0).then_some((type_matches && features_match) != negated),
            )
        });
        if let Ok(Some(matches)) = query {
            empty = false;
            any |= matches;
        }
        if input.next().is_err() {
            break;
        }
    }
    // An empty media query list matches everything.
    empty || any
}

fn evaluate_feature(input: &mut Parser<'_>) -> bool {
    let Ok(name) = input.expect_ident_cloned() else {
        skip_rest(input);
        return true;
    };
    let name = name.to_ascii_lowercase();
    if input.expect_colon().is_err() {
        skip_rest(input);
        return true;
    }
    let width = Length::parse(input).map(|l| l.to_pt(12.0, 12.0) / PT_PER_PX);
    skip_rest(input);
    match (name.as_str(), width) {
        ("min-width", Ok(width)) => MEDIA_WIDTH_PX >= width,
        ("max-width", Ok(width)) => MEDIA_WIDTH_PX <= width,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::{Color, ColorValue};
    use PropertyDeclaration as D;

    #[test]
    fn parses_rules_and_skips_invalid_ones() {
        let sheet = Stylesheet::parse(
            "p { color: red; bogus: 1; margin-top: 2pt }
             p >> { color: blue }
             @import url(x.css);
             @font-face { font-family: X }
             h1, h2 { display: none !important }",
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(sheet.rules[1].selectors.slice().len(), 2);
        assert!(sheet.rules[1].declarations[0].important);
    }

    #[test]
    fn style_attribute() {
        let decls = parse_style_attribute("color: #00f; width: 50% !important; nonsense");
        assert_eq!(
            decls,
            [
                Declaration {
                    property: D::Color(ColorValue::Color(Color::rgb(0, 0, 255))),
                    important: false
                },
                Declaration {
                    property: D::Width(crate::values::Dim::Percent(0.5)),
                    important: true
                },
            ]
        );
    }

    #[test]
    fn media_rules_apply_for_print_only() {
        let sheet = Stylesheet::parse(
            "@media print { a { color: red } }
             @media screen { b { color: red } }
             @media screen, print { c { color: red } }
             @media not screen { d { color: red } }
             @media (max-width: 600px) { e { color: red } }
             @media all and (min-width: 600px) { f { color: red } }
             @supports (display: grid) { g { color: red } }",
        );
        assert_eq!(sheet.rules.len(), 5);
    }

    #[test]
    fn media_attribute() {
        assert!(media_matches(""));
        assert!(media_matches("print"));
        assert!(media_matches("screen, print"));
        assert!(!media_matches("screen"));
        assert!(!media_matches("only screen and (min-width: 100px)"));
    }

    #[test]
    fn page_rules_merge_in_order() {
        let sheet = Stylesheet::parse(
            "@page { size: A4 landscape; margin: 1in 2in }
             @page :first { margin: 0 }
             @page { margin-top: 36pt }",
        );
        let size = sheet.page.size.unwrap();
        assert_eq!(size.name.as_deref(), Some("a4"));
        assert_eq!(size.landscape, Some(true));
        assert_eq!(
            sheet.page.margin,
            Sides {
                top: Some(36.0),
                right: Some(144.0),
                bottom: Some(72.0),
                left: Some(144.0)
            }
        );

        let sheet = Stylesheet::parse("@page { size: 100mm 200mm }");
        let (w, h) = sheet.page.size.unwrap().dimensions.unwrap();
        assert!((w - 283.46).abs() < 0.01 && (h - 566.93).abs() < 0.01);
    }
}
