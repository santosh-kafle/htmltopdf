//! Document tree for the presswork engine.
//!
//! Nodes live in a flat arena owned by [`Document`] and refer to each other
//! by [`NodeId`]. This keeps the tree cheap to traverse and avoids
//! reference-counted pointers throughout the pipeline.
//!
//! [`parse_html`] builds a document from HTML using `html5ever`. The
//! [`resource`] module defines how the resources a document refers to
//! (images, stylesheets, fonts) are loaded.

mod parse;
pub mod resource;

use std::collections::HashMap;
use std::fmt;

pub use parse::parse_html;

/// Index of a node inside a [`Document`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    /// The document root node.
    pub const ROOT: NodeId = NodeId(0);

    fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A single attribute on an element, e.g. `class="invoice"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

/// Element-specific data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementData {
    /// Lowercase local name, e.g. `"div"`.
    pub name: String,
    /// Namespace URL, e.g. `"http://www.w3.org/1999/xhtml"`.
    pub namespace: String,
    pub attributes: Vec<Attribute>,
}

impl ElementData {
    /// Returns the value of the attribute `name`, if present.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }
}

/// What kind of node this is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    /// A detached fragment, such as the contents of a `<template>`.
    Fragment,
    Element(ElementData),
    Text(String),
    Comment(String),
}

/// A node in the document tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

/// An arena-backed document tree.
#[derive(Debug, Clone)]
pub struct Document {
    nodes: Vec<Node>,
    /// Contents of `<template>` elements, which are not part of the tree.
    template_contents: HashMap<NodeId, NodeId>,
    /// Whether the parser switched to quirks or limited-quirks mode.
    pub quirks_mode: bool,
}

impl Document {
    /// Creates an empty document containing only the root node.
    pub fn new() -> Self {
        Self {
            nodes: vec![Node {
                kind: NodeKind::Document,
                parent: None,
                children: Vec::new(),
            }],
            template_contents: HashMap::new(),
            quirks_mode: false,
        }
    }

    /// Returns the root node id.
    pub fn root(&self) -> NodeId {
        NodeId::ROOT
    }

    /// Returns the node with the given id.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not belong to this document.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Number of nodes in the document, including the root.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Always `false`: a document contains at least its root node.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    /// Creates a new node that is not attached to the tree yet.
    pub fn create(&mut self, kind: NodeKind) -> NodeId {
        let index = u32::try_from(self.nodes.len()).expect("document exceeds u32::MAX nodes");
        self.nodes.push(Node {
            kind,
            parent: None,
            children: Vec::new(),
        });
        NodeId(index)
    }

    /// Creates a new node and appends it as the last child of `parent`.
    pub fn append(&mut self, parent: NodeId, kind: NodeKind) -> NodeId {
        let id = self.create(kind);
        self.append_child(parent, id);
        id
    }

    /// Moves `child` to be the last child of `parent`.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        self.node_mut(child).parent = Some(parent);
        self.node_mut(parent).children.push(child);
    }

    /// Moves `child` to be the sibling immediately before `sibling`.
    ///
    /// # Panics
    ///
    /// Panics if `sibling` has no parent.
    pub fn insert_before(&mut self, sibling: NodeId, child: NodeId) {
        self.detach(child);
        let parent = self.node(sibling).parent.expect("sibling has no parent");
        let siblings = &mut self.node_mut(parent).children;
        let position = siblings
            .iter()
            .position(|&c| c == sibling)
            .expect("sibling is a child of its parent");
        siblings.insert(position, child);
        self.node_mut(child).parent = Some(parent);
    }

    /// Removes `id` from its parent, if it has one. The node and its
    /// descendants stay in the arena.
    pub fn detach(&mut self, id: NodeId) {
        if let Some(parent) = self.node_mut(id).parent.take() {
            self.node_mut(parent).children.retain(|&c| c != id);
        }
    }

    /// The sibling immediately before `id`, if any.
    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.node(id).parent?;
        let siblings = &self.node(parent).children;
        let position = siblings.iter().position(|&c| c == id)?;
        position.checked_sub(1).map(|i| siblings[i])
    }

    /// Element data of `id`, if it is an element.
    pub fn element(&self, id: NodeId) -> Option<&ElementData> {
        match &self.node(id).kind {
            NodeKind::Element(data) => Some(data),
            _ => None,
        }
    }

    /// The root element, normally `<html>`.
    pub fn document_element(&self) -> Option<NodeId> {
        self.children(self.root())
            .find(|&id| self.element(id).is_some())
    }

    /// All nodes below `id` in document order, not including `id`.
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut stack: Vec<NodeId> = self.node(id).children.iter().rev().copied().collect();
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            stack.extend(self.node(next).children.iter().rev().copied());
            Some(next)
        })
    }

    /// The first element named `name` in document order.
    pub fn find_element(&self, name: &str) -> Option<NodeId> {
        self.descendants(self.root())
            .find(|&id| self.element(id).is_some_and(|e| e.name == name))
    }

    /// The concatenated text of all text nodes below `id`.
    pub fn text_content(&self, id: NodeId) -> String {
        self.descendants(id)
            .filter_map(|id| match &self.node(id).kind {
                NodeKind::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The document title from `<title>`, with whitespace collapsed.
    pub fn title(&self) -> Option<String> {
        let title = self.find_element("title")?;
        let text = self.text_content(title);
        let collapsed = text.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
        (!collapsed.is_empty()).then_some(collapsed)
    }

    /// Replaces the children of `id` with one text node holding `text`, as
    /// setting `textContent` does in a browser.
    pub fn set_text_content(&mut self, id: NodeId, text: &str) {
        for child in std::mem::take(&mut self.nodes[id.index()].children) {
            self.nodes[child.index()].parent = None;
        }
        if !text.is_empty() {
            self.append(id, NodeKind::Text(text.to_owned()));
        }
    }

    /// Iterates over the children of `id` in document order.
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.node(id).children.iter().copied()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(name: &str) -> NodeKind {
        NodeKind::Element(ElementData {
            name: name.to_owned(),
            namespace: parse::html_namespace(),
            attributes: Vec::new(),
        })
    }

    #[test]
    fn set_text_content_replaces_children() {
        let mut doc = parse_html(b"<p>Page <b>x</b> of <i>y</i></p>");
        let p = doc.find_element("p").unwrap();
        let b = doc.find_element("b").unwrap();
        doc.set_text_content(b, "2");
        assert_eq!(doc.text_content(p), "Page 2 of y");
        doc.set_text_content(p, "");
        assert_eq!(doc.children(p).count(), 0);
        assert_eq!(doc.node(b).parent, None);
    }

    #[test]
    fn new_document_has_only_root() {
        let doc = Document::new();
        assert_eq!(doc.len(), 1);
        assert_eq!(doc.node(doc.root()).kind, NodeKind::Document);
    }

    #[test]
    fn append_links_parent_and_children() {
        let mut doc = Document::new();
        let html = doc.append(doc.root(), element("html"));
        let body = doc.append(html, element("body"));
        let text = doc.append(body, NodeKind::Text("hello".into()));

        assert_eq!(doc.node(body).parent, Some(html));
        assert_eq!(doc.children(html).collect::<Vec<_>>(), vec![body]);
        assert_eq!(doc.children(body).collect::<Vec<_>>(), vec![text]);
    }

    #[test]
    fn insert_before_and_detach_move_nodes() {
        let mut doc = Document::new();
        let a = doc.append(doc.root(), element("a"));
        let b = doc.append(doc.root(), element("b"));
        let c = doc.create(element("c"));
        doc.insert_before(b, c);
        assert_eq!(doc.children(doc.root()).collect::<Vec<_>>(), [a, c, b]);
        assert_eq!(doc.previous_sibling(b), Some(c));

        doc.append_child(a, b);
        assert_eq!(doc.children(doc.root()).collect::<Vec<_>>(), [a, c]);
        assert_eq!(doc.node(b).parent, Some(a));
        assert_eq!(doc.descendants(doc.root()).collect::<Vec<_>>(), [a, b, c]);
    }

    #[test]
    fn attribute_lookup() {
        let data = ElementData {
            name: "p".into(),
            namespace: parse::html_namespace(),
            attributes: vec![Attribute {
                name: "class".into(),
                value: "lead".into(),
            }],
        };
        assert_eq!(data.attr("class"), Some("lead"));
        assert_eq!(data.attr("id"), None);
    }
}
