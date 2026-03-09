//! CSS cascade and style resolution.

use std::collections::HashMap;

use lightningcss::stylesheet::{ParserOptions, StyleSheet as LcssStyleSheet};
use lightningcss::rules::CssRule;
use lightningcss::properties::Property;
use lightningcss::values::color::CssColor;
use tracing::debug;

use crate::dom::{Document, Node, NodeData};

/// Errors that can occur during CSS operations.
#[derive(Debug, thiserror::Error)]
pub enum CssError {
    /// Failed to parse a CSS stylesheet.
    #[error("CSS parse error: {0}")]
    ParseError(String),
}

/// A parsed CSS stylesheet.
#[derive(Debug, Clone)]
pub struct StyleSheet {
    /// Extracted style rules as (selector_text, properties) pairs.
    pub rules: Vec<StyleRule>,
}

/// A single CSS rule: selector + declarations.
#[derive(Debug, Clone)]
pub struct StyleRule {
    /// The raw selector text (e.g. `"div.main"`, `"#header"`).
    pub selector: String,
    /// Parsed property declarations.
    pub declarations: Vec<Declaration>,
}

/// A single CSS property declaration.
#[derive(Debug, Clone)]
pub struct Declaration {
    /// Property name (e.g. `"color"`, `"margin-left"`).
    pub property: String,
    /// Property value as a string.
    pub value: String,
}

impl StyleSheet {
    /// Parse a CSS string into a `StyleSheet`.
    ///
    /// # Errors
    ///
    /// Returns `CssError::ParseError` if the CSS is malformed.
    pub fn parse(css: &str) -> Result<Self, CssError> {
        let options = ParserOptions::default();
        let parsed = LcssStyleSheet::parse(css, options)
            .map_err(|e| CssError::ParseError(format!("{e}")))?;

        let mut rules = Vec::new();

        for rule in &parsed.rules.0 {
            if let CssRule::Style(style_rule) = rule {
                let selector = format!("{:?}", style_rule.selectors);

                let mut declarations = Vec::new();

                for (property, _importance) in style_rule.declarations.iter() {
                    let (name, value) = extract_property(property);
                    declarations.push(Declaration {
                        property: name,
                        value,
                    });
                }

                rules.push(StyleRule {
                    selector,
                    declarations,
                });
            }
        }

        debug!("parsed {} CSS rules", rules.len());
        Ok(Self { rules })
    }
}

/// Extract a property name and value string from a lightningcss `Property`.
fn extract_property(prop: &Property<'_>) -> (String, String) {
    // Use Debug formatting as a portable way to get the property info.
    // A full implementation would match each property variant.
    let debug_str = format!("{prop:?}");

    // Try to extract a cleaner name from known variants.
    match prop {
        Property::Color(c) => ("color".to_string(), format_css_color(c)),
        Property::BackgroundColor(c) => ("background-color".to_string(), format_css_color(c)),
        Property::Display(d) => ("display".to_string(), format!("{d:?}").to_lowercase()),
        Property::Width(w) => ("width".to_string(), format!("{w:?}")),
        Property::Height(h) => ("height".to_string(), format!("{h:?}")),
        Property::MarginTop(m) => ("margin-top".to_string(), format!("{m:?}")),
        Property::MarginBottom(m) => ("margin-bottom".to_string(), format!("{m:?}")),
        Property::MarginLeft(m) => ("margin-left".to_string(), format!("{m:?}")),
        Property::MarginRight(m) => ("margin-right".to_string(), format!("{m:?}")),
        Property::PaddingTop(p) => ("padding-top".to_string(), format!("{p:?}")),
        Property::PaddingBottom(p) => ("padding-bottom".to_string(), format!("{p:?}")),
        Property::PaddingLeft(p) => ("padding-left".to_string(), format!("{p:?}")),
        Property::PaddingRight(p) => ("padding-right".to_string(), format!("{p:?}")),
        Property::FontSize(s) => ("font-size".to_string(), format!("{s:?}")),
        Property::FontFamily(f) => ("font-family".to_string(), format!("{f:?}")),
        _ => {
            // Fallback: derive name from debug output.
            let name = debug_str
                .split('(')
                .next()
                .unwrap_or("unknown")
                .to_lowercase();
            (name, debug_str)
        }
    }
}

/// Format a CSS color value to a readable string.
fn format_css_color(color: &CssColor) -> String {
    match color {
        CssColor::RGBA(rgba) => {
            if rgba.alpha == 255 {
                format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue)
            } else {
                format!(
                    "rgba({}, {}, {}, {:.2})",
                    rgba.red,
                    rgba.green,
                    rgba.blue,
                    f64::from(rgba.alpha) / 255.0
                )
            }
        }
        CssColor::CurrentColor => "currentColor".to_string(),
        _ => format!("{color:?}"),
    }
}

/// Computed style values for a single element.
#[derive(Debug, Clone, Default)]
pub struct ComputedStyle {
    /// All computed property values, keyed by property name.
    pub properties: HashMap<String, String>,
}

impl ComputedStyle {
    /// Get a property value by name.
    #[must_use]
    pub fn get(&self, property: &str) -> Option<&str> {
        self.properties.get(property).map(String::as_str)
    }

    /// Set a property value.
    pub fn set(&mut self, property: impl Into<String>, value: impl Into<String>) {
        self.properties.insert(property.into(), value.into());
    }

    /// Get the display property, defaulting to `"inline"`.
    #[must_use]
    pub fn display(&self) -> &str {
        self.get("display").unwrap_or("inline")
    }
}

/// A node in the styled tree, pairing a reference to a DOM node with its computed style.
#[derive(Debug, Clone)]
pub struct StyledNode {
    /// Index into the original DOM tree's descendant list.
    pub node_index: usize,
    /// The tag name (if element) or node type description.
    pub tag: String,
    /// Computed style for this node.
    pub style: ComputedStyle,
    /// Child styled nodes.
    pub children: Vec<StyledNode>,
}

/// A tree of styled nodes produced by cascade resolution.
#[derive(Debug, Clone)]
pub struct StyledTree {
    /// The root styled node.
    pub root: StyledNode,
}

/// Resolves styles against a DOM tree.
pub struct StyleResolver {
    /// Collected stylesheets.
    sheets: Vec<StyleSheet>,
}

impl StyleResolver {
    /// Create a new resolver with no stylesheets.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sheets: Vec::new(),
        }
    }

    /// Add a stylesheet to the resolver.
    pub fn add_sheet(&mut self, sheet: StyleSheet) {
        self.sheets.push(sheet);
    }

    /// Resolve styles for all elements in the document.
    ///
    /// Currently applies a simplified cascade: for each element, any matching
    /// rule's declarations are merged in stylesheet order. Specificity is not
    /// yet fully implemented.
    #[must_use]
    pub fn resolve(&self, document: &Document) -> StyledTree {
        let root = self.resolve_node(&document.root, 0);
        StyledTree { root }
    }

    fn resolve_node(&self, node: &Node, index: usize) -> StyledNode {
        let mut style = ComputedStyle::default();
        let tag;

        match &node.data {
            NodeData::Element(el) => {
                tag = el.tag.clone();
                // Apply default block display for known block elements.
                if is_block_element(&el.tag) {
                    style.set("display", "block");
                }
                // Match rules against this element.
                for sheet in &self.sheets {
                    for rule in &sheet.rules {
                        if simple_selector_matches(&rule.selector, el) {
                            for decl in &rule.declarations {
                                style.set(decl.property.clone(), decl.value.clone());
                            }
                        }
                    }
                }
            }
            NodeData::Text(_) => {
                tag = "#text".to_string();
            }
            NodeData::Document => {
                tag = "#document".to_string();
            }
            NodeData::Comment(_) => {
                tag = "#comment".to_string();
            }
        }

        let mut child_index = index + 1;
        let children = node
            .children
            .iter()
            .map(|child| {
                let styled = self.resolve_node(child, child_index);
                child_index += count_descendants(child);
                styled
            })
            .collect();

        StyledNode {
            node_index: index,
            tag,
            style,
            children,
        }
    }
}

impl Default for StyleResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether a tag is a block-level element by default.
fn is_block_element(tag: &str) -> bool {
    matches!(
        tag,
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "ul" | "ol" | "li" | "blockquote" | "pre" | "section"
            | "article" | "header" | "footer" | "nav" | "main" | "form"
            | "table" | "figure" | "figcaption" | "details" | "summary"
    )
}

/// Very simple selector matching (tag name only for now).
///
/// A production implementation would parse the selector properly, but for
/// the initial scaffold this handles the most common case.
fn simple_selector_matches(selector_debug: &str, element: &crate::dom::ElementData) -> bool {
    // The selector field currently stores Debug output from lightningcss selectors.
    // We do a best-effort match on the tag name.
    let lower = selector_debug.to_lowercase();
    lower.contains(&element.tag)
}

/// Count total descendants of a node (including the node itself).
fn count_descendants(node: &Node) -> usize {
    1 + node.children.iter().map(|c| count_descendants(c)).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::Document;

    #[test]
    fn parse_simple_stylesheet() {
        let sheet = StyleSheet::parse("p { color: red; }").unwrap();
        assert!(!sheet.rules.is_empty());
        assert_eq!(sheet.rules[0].declarations[0].property, "color");
    }

    #[test]
    fn parse_multiple_rules() {
        let css = r"
            div { display: block; }
            .highlight { color: yellow; }
            #main { width: 100px; }
        ";
        let sheet = StyleSheet::parse(css).unwrap();
        assert_eq!(sheet.rules.len(), 3);
    }

    #[test]
    fn parse_invalid_css_error() {
        // lightningcss is quite lenient, but completely broken input
        // should still parse (CSS error recovery). This test verifies
        // we don't panic.
        let result = StyleSheet::parse("not css at { all }}}}}");
        // lightningcss recovers from most errors, so this may succeed.
        // The important thing is we don't panic.
        let _ = result;
    }

    #[test]
    fn resolve_styles_basic() {
        let doc = Document::parse("<div><p>Hello</p></div>");
        let sheet = StyleSheet::parse("p { color: red; }").unwrap();

        let mut resolver = StyleResolver::new();
        resolver.add_sheet(sheet);
        let styled = resolver.resolve(&doc);

        // The styled tree should exist and have children.
        assert!(!styled.root.children.is_empty());
    }

    #[test]
    fn computed_style_defaults() {
        let style = ComputedStyle::default();
        assert_eq!(style.display(), "inline");
        assert!(style.get("color").is_none());
    }

    #[test]
    fn block_elements_get_block_display() {
        let doc = Document::parse("<div>content</div>");
        let resolver = StyleResolver::new();
        let styled = resolver.resolve(&doc);

        // Find the div in the styled tree.
        fn find_tag<'a>(node: &'a StyledNode, tag: &str) -> Option<&'a StyledNode> {
            if node.tag == tag {
                return Some(node);
            }
            for child in &node.children {
                if let Some(found) = find_tag(child, tag) {
                    return Some(found);
                }
            }
            None
        }

        let div = find_tag(&styled.root, "div");
        assert!(div.is_some());
        assert_eq!(div.unwrap().style.display(), "block");
    }
}
