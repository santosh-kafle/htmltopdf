//! Document tree for the presswork engine.
//!
//! Nodes live in a flat arena owned by [`Document`] and refer to each other
//! by [`NodeId`]. This keeps the tree cheap to traverse and avoids
//! reference-counted pointers throughout the pipeline.

use std::fmt;

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

    /// Creates a new node and appends it as the last child of `parent`.
    pub fn append(&mut self, parent: NodeId, kind: NodeKind) -> NodeId {
        let index = u32::try_from(self.nodes.len()).expect("document exceeds u32::MAX nodes");
        let id = NodeId(index);
        self.nodes.push(Node {
            kind,
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes[parent.index()].children.push(id);
        id
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
            attributes: Vec::new(),
        })
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
    fn attribute_lookup() {
        let data = ElementData {
            name: "p".into(),
            attributes: vec![Attribute {
                name: "class".into(),
                value: "lead".into(),
            }],
        };
        assert_eq!(data.attr("class"), Some("lead"));
        assert_eq!(data.attr("id"), None);
    }
}
