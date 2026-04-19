//! Core DOM node types.

use markup5ever::QualName;
use std::fmt;

/// The kind of data a DOM node holds.
#[derive(Clone)]
pub enum NodeData {
    /// The root document node.
    Document,
    /// An HTML element with tag name and attributes.
    Element(ElementData),
    /// A text node.
    Text(String),
    /// An HTML comment.
    Comment(String),
}

impl fmt::Debug for NodeData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document => write!(f, "Document"),
            Self::Element(el) => write!(f, "<{}>", el.tag),
            Self::Text(t) => {
                let preview = if t.len() > 40 { &t[..40] } else { t.as_str() };
                write!(f, "Text({preview:?})")
            }
            Self::Comment(c) => write!(f, "Comment({c:?})"),
        }
    }
}

/// Data associated with an HTML element node.
#[derive(Debug, Clone)]
pub struct ElementData {
    /// The tag name (e.g. `"div"`, `"a"`, `"p"`).
    pub tag: String,
    /// Attribute key-value pairs.
    pub attributes: Vec<(String, String)>,
    /// The qualified name from html5ever (stored for TreeSink compatibility).
    pub(crate) qual_name: Option<QualName>,
}

impl ElementData {
    /// Create a new element with the given tag and no attributes.
    #[must_use]
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            attributes: Vec::new(),
            qual_name: None,
        }
    }

    /// Create a new element with the given tag and attributes.
    #[must_use]
    pub fn with_attributes(tag: impl Into<String>, attributes: Vec<(String, String)>) -> Self {
        Self {
            tag: tag.into(),
            attributes,
            qual_name: None,
        }
    }

    /// Get the value of an attribute by name.
    #[must_use]
    pub fn get_attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Check whether this element has a given class.
    #[must_use]
    pub fn has_class(&self, class: &str) -> bool {
        self.get_attribute("class")
            .is_some_and(|classes| classes.split_whitespace().any(|c| c == class))
    }

    /// Get the `id` attribute if present.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.get_attribute("id")
    }
}

/// A node in the DOM tree.
#[derive(Debug, Clone)]
pub struct Node {
    /// The data this node holds.
    pub data: NodeData,
    /// Child nodes.
    pub children: Vec<Node>,
}

impl Node {
    /// Create a new document node.
    #[must_use]
    pub fn document() -> Self {
        Self {
            data: NodeData::Document,
            children: Vec::new(),
        }
    }

    /// Create a new element node.
    #[must_use]
    pub fn element(element_data: ElementData) -> Self {
        Self {
            data: NodeData::Element(element_data),
            children: Vec::new(),
        }
    }

    /// Create a new text node.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            data: NodeData::Text(content.into()),
            children: Vec::new(),
        }
    }

    /// Create a new comment node.
    #[must_use]
    pub fn comment(content: impl Into<String>) -> Self {
        Self {
            data: NodeData::Comment(content.into()),
            children: Vec::new(),
        }
    }

    /// Returns `true` if this is a document node.
    #[must_use]
    pub fn is_document(&self) -> bool {
        matches!(self.data, NodeData::Document)
    }

    /// Returns `true` if this is an element node.
    #[must_use]
    pub fn is_element(&self) -> bool {
        matches!(self.data, NodeData::Element(_))
    }

    /// Returns `true` if this is a text node.
    #[must_use]
    pub fn is_text(&self) -> bool {
        matches!(self.data, NodeData::Text(_))
    }

    /// Returns the element data if this is an element node.
    #[must_use]
    pub fn as_element(&self) -> Option<&ElementData> {
        match &self.data {
            NodeData::Element(el) => Some(el),
            _ => None,
        }
    }

    /// Returns the text content if this is a text node.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match &self.data {
            NodeData::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Recursively collect all text content from this node and its descendants.
    #[must_use]
    pub fn text_content(&self) -> String {
        let mut result = String::new();
        self.collect_text(&mut result);
        result
    }

    fn collect_text(&self, buf: &mut String) {
        match &self.data {
            NodeData::Text(t) => buf.push_str(t),
            NodeData::Document | NodeData::Element(_) => {
                for child in &self.children {
                    child.collect_text(buf);
                }
            }
            NodeData::Comment(_) => {}
        }
    }

    /// Append a child node.
    pub fn append_child(&mut self, child: Node) {
        self.children.push(child);
    }

    /// Iterate over all descendant nodes in depth-first order.
    pub fn descendants(&self) -> DescendantIter<'_> {
        DescendantIter { stack: vec![self] }
    }
}

/// Depth-first iterator over all descendant nodes.
pub struct DescendantIter<'a> {
    stack: Vec<&'a Node>,
}

impl<'a> Iterator for DescendantIter<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        // Push children in reverse so leftmost child is visited first.
        for child in node.children.iter().rev() {
            self.stack.push(child);
        }
        Some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_attribute_lookup() {
        let el = ElementData::with_attributes(
            "a",
            vec![
                ("href".into(), "https://example.com".into()),
                ("class".into(), "link primary".into()),
            ],
        );
        assert_eq!(el.get_attribute("href"), Some("https://example.com"));
        assert!(el.has_class("link"));
        assert!(el.has_class("primary"));
        assert!(!el.has_class("secondary"));
    }

    #[test]
    fn text_content_recursive() {
        let mut div = Node::element(ElementData::new("div"));
        div.append_child(Node::text("Hello, "));
        let mut span = Node::element(ElementData::new("span"));
        span.append_child(Node::text("world!"));
        div.append_child(span);
        assert_eq!(div.text_content(), "Hello, world!");
    }

    #[test]
    fn descendants_iteration() {
        let mut root = Node::document();
        let mut div = Node::element(ElementData::new("div"));
        div.append_child(Node::text("inner"));
        root.append_child(div);
        root.append_child(Node::text("outer"));

        let count = root.descendants().count();
        // root + div + "inner" + "outer" = 4
        assert_eq!(count, 4);
    }
}
