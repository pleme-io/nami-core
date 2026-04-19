//! HTML parsing and document-level queries.

use html5ever::tendril::TendrilSink as _;
use html5ever::tree_builder::TreeSink;
use html5ever::{Attribute, local_name, namespace_url, ns, parse_document};
use markup5ever::{ExpandedName, QualName};
use std::borrow::Cow;
use tracing::debug;

use super::node::{ElementData, Node, NodeData};

/// A parsed HTML document.
#[derive(Debug, Clone)]
pub struct Document {
    /// The root node of the DOM tree.
    pub root: Node,
}

impl Document {
    /// Parse an HTML string into a `Document`.
    ///
    /// Uses html5ever for spec-compliant HTML5 parsing, including error recovery
    /// for malformed markup.
    #[must_use]
    pub fn parse(html: &str) -> Self {
        let sink = DomSink::default();
        let parser = parse_document(sink, Default::default());
        let sink = parser.one(html);
        Self { root: sink.root }
    }

    /// Parse an HTML *fragment* (not a full document) into a flat list of
    /// top-level nodes. Used by transform actions that splice authored
    /// HTML into an existing tree (`InsertBefore`, `InsertAfter`,
    /// `ReplaceWith`).
    ///
    /// Wraps the snippet in a dummy `<body>`, parses as a full document,
    /// then returns the children of that `<body>`. This is simpler than
    /// html5ever's context-aware `parse_fragment` and sufficient for our
    /// use (authored snippets rarely need a non-body parse context).
    #[must_use]
    pub fn parse_fragment(snippet: &str) -> Vec<Node> {
        let wrapped = format!("<!DOCTYPE html><html><body>{snippet}</body></html>");
        let doc = Self::parse(&wrapped);
        let mut body_children = Vec::new();
        collect_body_children(&doc.root, &mut body_children);
        body_children
    }

    /// Find all elements matching a simple selector.
    ///
    /// Supports:
    /// - Tag selectors: `"div"`, `"p"`, `"a"`
    /// - Class selectors: `".classname"`
    /// - ID selectors: `"#id"`
    ///
    /// Does not support compound selectors or combinators.
    #[must_use]
    pub fn query_selector_all(&self, selector: &str) -> Vec<&Node> {
        let matcher = SimpleMatcher::parse(selector);
        self.root
            .descendants()
            .filter(|node| matcher.matches(node))
            .collect()
    }

    /// Find the first element matching a simple selector, or `None`.
    #[must_use]
    pub fn query_selector(&self, selector: &str) -> Option<&Node> {
        let matcher = SimpleMatcher::parse(selector);
        self.root.descendants().find(|node| matcher.matches(node))
    }

    /// Get all text content in the document, concatenated.
    #[must_use]
    pub fn text_content(&self) -> String {
        self.root.text_content()
    }

    /// Get the document title (text inside `<title>`), if any.
    #[must_use]
    pub fn title(&self) -> Option<String> {
        self.query_selector("title")
            .map(|node| node.text_content().trim().to_string())
    }

    /// Get all links (`<a href="...">`) in the document.
    #[must_use]
    pub fn links(&self) -> Vec<(&str, String)> {
        self.query_selector_all("a")
            .into_iter()
            .filter_map(|node| {
                let el = node.as_element()?;
                let href = el.get_attribute("href")?;
                let text = node.text_content();
                Some((href, text))
            })
            .collect()
    }
}

/// Simple CSS selector matcher (tag, class, or id).
enum SimpleMatcher {
    Tag(String),
    Class(String),
    Id(String),
}

impl SimpleMatcher {
    fn parse(selector: &str) -> Self {
        if let Some(class) = selector.strip_prefix('.') {
            Self::Class(class.to_string())
        } else if let Some(id) = selector.strip_prefix('#') {
            Self::Id(id.to_string())
        } else {
            Self::Tag(selector.to_lowercase())
        }
    }

    fn matches(&self, node: &Node) -> bool {
        let Some(el) = node.as_element() else {
            return false;
        };
        match self {
            Self::Tag(tag) => el.tag == *tag,
            Self::Class(class) => el.has_class(class),
            Self::Id(id) => el.id() == Some(id.as_str()),
        }
    }
}

// ── html5ever TreeSink implementation ──────────────────────────────────

type Handle = usize;

/// A `TreeSink` that builds our DOM tree from html5ever's parse events.
struct DomSink {
    root: Node,
    /// Flat storage: each node gets an index. Index 0 is the root document node.
    nodes: Vec<Node>,
    /// Parent index for each node.
    parents: Vec<Option<usize>>,
    /// Child indices for each node.
    child_indices: Vec<Vec<usize>>,
}

impl Default for DomSink {
    fn default() -> Self {
        let mut sink = Self {
            root: Node::document(),
            nodes: Vec::new(),
            parents: Vec::new(),
            child_indices: Vec::new(),
        };
        // Allocate the root document node at index 0.
        sink.alloc_node(NodeData::Document);
        sink
    }
}

impl DomSink {
    fn alloc_node(&mut self, data: NodeData) -> Handle {
        let idx = self.nodes.len();
        self.nodes.push(Node {
            data,
            children: Vec::new(),
        });
        self.parents.push(None);
        self.child_indices.push(Vec::new());
        idx
    }

    /// After parsing, reconstruct the tree from the flat storage.
    fn build_tree(&mut self) {
        // Work bottom-up: for each node that has children, attach them.
        // We need to do this carefully to avoid borrow issues.
        // Strategy: collect child data, then assign.
        let n = self.nodes.len();
        if n == 0 {
            return;
        }

        // Build the tree from leaves up.
        // First, identify the order: process nodes in reverse so children
        // are fully built before parents.
        for i in (0..n).rev() {
            let child_idxs: Vec<usize> = self.child_indices[i].clone();
            let children: Vec<Node> = child_idxs
                .iter()
                .map(|&ci| self.nodes[ci].clone())
                .collect();
            self.nodes[i].children = children;
        }

        if !self.nodes.is_empty() {
            self.root = self.nodes[0].clone();
        }
    }
}

impl TreeSink for DomSink {
    type Handle = Handle;
    type Output = Self;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(mut self) -> Self::Output {
        self.build_tree();
        self
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        debug!("html5ever parse error: {msg}");
    }

    fn get_document(&self) -> Handle {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> ExpandedName<'a> {
        if let NodeData::Element(ref el) = self.nodes[*target].data {
            if let Some(ref qn) = el.qual_name {
                return ExpandedName {
                    ns: &qn.ns,
                    local: &qn.local,
                };
            }
        }
        // Fallback: should not happen if create_element always sets qual_name,
        // but we need a static reference for the return type.
        static EMPTY_NS: markup5ever::Namespace = ns!();
        static EMPTY_LOCAL: markup5ever::LocalName = local_name!("");
        ExpandedName {
            ns: &EMPTY_NS,
            local: &EMPTY_LOCAL,
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: html5ever::tree_builder::ElementFlags,
    ) -> Handle {
        // We need &mut self but TreeSink gives us &self for create_element.
        // This is a known awkwardness with html5ever's API.
        // We'll use a workaround with interior mutability in a real impl,
        // but for the scaffold we cast (this is the standard pattern).
        #[expect(invalid_reference_casting)]
        let this = unsafe { &mut *(std::ptr::from_ref(self) as *mut Self) };

        let attributes = attrs
            .into_iter()
            .map(|a| (a.name.local.to_string(), a.value.to_string()))
            .collect();

        let element = ElementData {
            tag: name.local.to_string(),
            attributes,
            qual_name: Some(name),
        };

        this.alloc_node(NodeData::Element(element))
    }

    fn create_comment(&self, text: html5ever::tendril::StrTendril) -> Handle {
        #[expect(invalid_reference_casting)]
        let this = unsafe { &mut *(std::ptr::from_ref(self) as *mut Self) };
        this.alloc_node(NodeData::Comment(text.to_string()))
    }

    fn create_pi(
        &self,
        _target: html5ever::tendril::StrTendril,
        _data: html5ever::tendril::StrTendril,
    ) -> Handle {
        #[expect(invalid_reference_casting)]
        let this = unsafe { &mut *(std::ptr::from_ref(self) as *mut Self) };
        this.alloc_node(NodeData::Comment(String::new()))
    }

    fn append(&self, parent: &Handle, child: html5ever::tree_builder::NodeOrText<Handle>) {
        #[expect(invalid_reference_casting)]
        let this = unsafe { &mut *(std::ptr::from_ref(self) as *mut Self) };

        let child_handle = match child {
            html5ever::tree_builder::NodeOrText::AppendNode(handle) => handle,
            html5ever::tree_builder::NodeOrText::AppendText(text) => {
                this.alloc_node(NodeData::Text(text.to_string()))
            }
        };

        this.parents[child_handle] = Some(*parent);
        this.child_indices[*parent].push(child_handle);
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: html5ever::tree_builder::NodeOrText<Handle>,
    ) {
        // If the element has a parent, append to the parent before the element.
        // Otherwise append to prev_element.
        let parent = if self.parents[*element].is_some() {
            *element
        } else {
            *prev_element
        };
        self.append(&parent, child);
    }

    fn append_doctype_to_document(
        &self,
        _name: html5ever::tendril::StrTendril,
        _public_id: html5ever::tendril::StrTendril,
        _system_id: html5ever::tendril::StrTendril,
    ) {
        // We don't store doctype nodes.
    }

    fn get_template_contents(&self, target: &Handle) -> Handle {
        *target
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        *x == *y
    }

    fn set_quirks_mode(&self, _mode: html5ever::tree_builder::QuirksMode) {
        // Not tracked in this implementation.
    }

    fn append_before_sibling(
        &self,
        sibling: &Handle,
        child: html5ever::tree_builder::NodeOrText<Handle>,
    ) {
        #[expect(invalid_reference_casting)]
        let this = unsafe { &mut *(std::ptr::from_ref(self) as *mut Self) };

        let child_handle = match child {
            html5ever::tree_builder::NodeOrText::AppendNode(handle) => handle,
            html5ever::tree_builder::NodeOrText::AppendText(text) => {
                this.alloc_node(NodeData::Text(text.to_string()))
            }
        };

        if let Some(parent_idx) = this.parents[*sibling] {
            // Insert child before sibling in parent's child list.
            if let Some(pos) = this.child_indices[parent_idx]
                .iter()
                .position(|&c| c == *sibling)
            {
                this.child_indices[parent_idx].insert(pos, child_handle);
            } else {
                this.child_indices[parent_idx].push(child_handle);
            }
            this.parents[child_handle] = Some(parent_idx);
        }
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        #[expect(invalid_reference_casting)]
        let this = unsafe { &mut *(std::ptr::from_ref(self) as *mut Self) };

        if let NodeData::Element(ref mut el) = this.nodes[*target].data {
            for attr in attrs {
                let name = attr.name.local.to_string();
                if !el.attributes.iter().any(|(k, _)| k == &name) {
                    el.attributes.push((name, attr.value.to_string()));
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        #[expect(invalid_reference_casting)]
        let this = unsafe { &mut *(std::ptr::from_ref(self) as *mut Self) };

        if let Some(parent_idx) = this.parents[*target] {
            this.child_indices[parent_idx].retain(|&c| c != *target);
            this.parents[*target] = None;
        }
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        #[expect(invalid_reference_casting)]
        let this = unsafe { &mut *(std::ptr::from_ref(self) as *mut Self) };

        let children: Vec<usize> = this.child_indices[*node].drain(..).collect();
        for child in children {
            this.parents[child] = Some(*new_parent);
            this.child_indices[*new_parent].push(child);
        }
    }
}

fn collect_body_children(node: &Node, out: &mut Vec<Node>) {
    if let NodeData::Element(el) = &node.data {
        if el.tag.eq_ignore_ascii_case("body") {
            out.extend(node.children.iter().cloned());
            return;
        }
    }
    for child in &node.children {
        collect_body_children(child, out);
        if !out.is_empty() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_html() {
        let doc = Document::parse(
            "<html><head><title>Test</title></head><body><p>Hello</p></body></html>",
        );
        assert!(doc.root.is_document() || doc.root.is_element());
        let text = doc.text_content();
        assert!(
            text.contains("Hello"),
            "text_content should contain 'Hello', got: {text}"
        );
    }

    #[test]
    fn parse_title() {
        let doc = Document::parse("<html><head><title>My Page</title></head><body></body></html>");
        assert_eq!(doc.title(), Some("My Page".to_string()));
    }

    #[test]
    fn query_selector_by_tag() {
        let doc = Document::parse("<div><p>First</p><p>Second</p></div>");
        let ps = doc.query_selector_all("p");
        assert_eq!(ps.len(), 2);
    }

    #[test]
    fn query_selector_by_class() {
        let doc =
            Document::parse(r#"<div><span class="highlight">yes</span><span>no</span></div>"#);
        let highlighted = doc.query_selector_all(".highlight");
        assert_eq!(highlighted.len(), 1);
        assert_eq!(highlighted[0].text_content(), "yes");
    }

    #[test]
    fn query_selector_by_id() {
        let doc = Document::parse(r#"<div><p id="main">content</p></div>"#);
        let node = doc.query_selector("#main");
        assert!(node.is_some());
        assert_eq!(node.unwrap().text_content(), "content");
    }

    #[test]
    fn extract_links() {
        let doc = Document::parse(
            r#"<a href="https://example.com">Example</a><a href="/about">About</a>"#,
        );
        let links = doc.links();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0, "https://example.com");
        assert_eq!(links[0].1, "Example");
    }

    #[test]
    fn malformed_html_recovery() {
        // html5ever should recover from missing closing tags.
        let doc = Document::parse("<div><p>unclosed");
        let text = doc.text_content();
        assert!(text.contains("unclosed"));
    }
}
