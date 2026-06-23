//! CSS cascade and style resolution.

use std::collections::HashMap;

use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, StyleSheet as LcssStyleSheet};
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
    //
    // Length/dimension/font values use lightningcss's own canonical
    // serializer (`value_to_css_string`) — NOT `format!("{:?}")`. The
    // Debug form emits `"LengthPercentage(Dimension(Px(200.0)))"`, which
    // neither the layout engine's `parse_px_value` nor the paint layer's
    // parsers can read; the canonical form is `"200px"`, which both
    // consume. This keeps the cascade's emitted value contract aligned
    // with every downstream reader (load-bearing fix, not a per-consumer
    // workaround).
    match prop {
        Property::Color(c) => ("color".to_string(), format_css_color(c)),
        Property::BackgroundColor(c) => ("background-color".to_string(), format_css_color(c)),
        // `background:` shorthand — extract the (final layer's) color so the
        // paint layer's "background-color" lookup finds it. Full shorthand
        // expansion (image/position/repeat/...) is M1 cascade work; the color
        // is the load-bearing part for rendering block backgrounds, and real
        // pages overwhelmingly use the shorthand. (Phase 3 shorthand expansion.)
        Property::Background(layers) => (
            "background-color".to_string(),
            layers
                .last()
                .map_or_else(String::new, |layer| format_css_color(&layer.color)),
        ),
        Property::Display(d) => ("display".to_string(), format!("{d:?}").to_lowercase()),
        Property::Width(_) => ("width".to_string(), css_value(prop)),
        Property::Height(_) => ("height".to_string(), css_value(prop)),
        Property::MarginTop(_) => ("margin-top".to_string(), css_value(prop)),
        Property::MarginBottom(_) => ("margin-bottom".to_string(), css_value(prop)),
        Property::MarginLeft(_) => ("margin-left".to_string(), css_value(prop)),
        Property::MarginRight(_) => ("margin-right".to_string(), css_value(prop)),
        Property::PaddingTop(_) => ("padding-top".to_string(), css_value(prop)),
        Property::PaddingBottom(_) => ("padding-bottom".to_string(), css_value(prop)),
        Property::PaddingLeft(_) => ("padding-left".to_string(), css_value(prop)),
        Property::PaddingRight(_) => ("padding-right".to_string(), css_value(prop)),
        Property::FontSize(_) => ("font-size".to_string(), css_value(prop)),
        Property::FontFamily(_) => ("font-family".to_string(), css_value(prop)),
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

/// Serialize a property's *value* to its canonical CSS string via
/// lightningcss's own `ToCss` (e.g. `"200px"`, `"14px"`, `"sans-serif"`).
/// Falls back to the Debug form only if serialization fails (it does not
/// for the length/dimension/font properties this is used on).
fn css_value(prop: &Property<'_>) -> String {
    prop.value_to_css_string(PrinterOptions::default())
        .unwrap_or_else(|_| format!("{prop:?}"))
}

/// CSS-standard inherited properties (the text-relevant subset). Seeded from
/// the parent's computed style before an element's own rules apply, so a child
/// without its own `color`/`font-*` shows the parent's. Non-inherited
/// properties (display, width, height, background-color, margin/padding) are
/// deliberately absent. (Phase 3 cascade: property inheritance.)
const INHERITED_PROPERTIES: &[&str] = &[
    "color",
    "font-size",
    "font-family",
    "font-weight",
    "font-style",
    "line-height",
    "text-align",
    "letter-spacing",
    "white-space",
    "visibility",
];

/// Parse an inline `style="..."` attribute's declarations by reusing the
/// stylesheet parser (wrap the block in a universal rule). Inline styles are
/// the highest-priority author origin; supporting them lets real pages that
/// style via the `style` attribute render. (Phase 5: inline-style parsing.)
fn parse_inline_style(style_attr: &str) -> Vec<Declaration> {
    StyleSheet::parse(&format!("*{{{style_attr}}}"))
        .ok()
        .and_then(|sheet| sheet.rules.into_iter().next())
        .map_or_else(Vec::new, |rule| rule.declarations)
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
        Self { sheets: Vec::new() }
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
        let root = self.resolve_node(&document.root, 0, None);
        StyledTree { root }
    }

    fn resolve_node(
        &self,
        node: &Node,
        index: usize,
        parent_style: Option<&ComputedStyle>,
    ) -> StyledNode {
        let mut style = ComputedStyle::default();
        // Inheritance: seed the CSS-standard inherited properties from the
        // parent BEFORE this element's own rules, so a child without its own
        // `color`/`font-*` shows the parent's. Own rules + inline style below
        // override. (Phase 3 cascade: property inheritance.)
        if let Some(parent) = parent_style {
            for &prop in INHERITED_PROPERTIES {
                if let Some(value) = parent.get(prop) {
                    style.set(prop, value.to_string());
                }
            }
        }
        let tag;

        match &node.data {
            NodeData::Element(el) => {
                tag = el.tag.clone();
                // UA display defaults. Non-rendered elements (head/style/script/
                // title/meta/...) are display:none so their text content never
                // paints (otherwise raw CSS/JS source leaks into the page);
                // block elements default to block; everything else stays inline.
                if is_non_rendered_element(&el.tag) {
                    style.set("display", "none");
                } else if is_block_element(&el.tag) {
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
                // Inline `style=""` attribute — highest-priority author origin,
                // applied last so it wins over sheet rules + inheritance.
                if let Some(inline) = el.get_attribute("style") {
                    for decl in parse_inline_style(inline) {
                        style.set(decl.property, decl.value);
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
                let styled = self.resolve_node(child, child_index, Some(&style));
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
        "div"
            | "p"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "blockquote"
            | "pre"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "nav"
            | "main"
            | "form"
            | "table"
            | "figure"
            | "figcaption"
            | "details"
            | "summary"
    )
}

/// Elements whose content is never rendered — UA `display: none`. Their text
/// (raw CSS, JS source, document metadata) must not paint into the page.
fn is_non_rendered_element(tag: &str) -> bool {
    matches!(
        tag,
        "head" | "style" | "script" | "title" | "meta" | "link" | "base" | "noscript" | "template"
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
    1 + node
        .children
        .iter()
        .map(|c| count_descendants(c))
        .sum::<usize>()
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

    /// Recursive depth-first tag lookup for the M1 cascade tests.
    fn find_styled<'a>(node: &'a StyledNode, tag: &str) -> Option<&'a StyledNode> {
        if node.tag == tag {
            return Some(node);
        }
        node.children.iter().find_map(|c| find_styled(c, tag))
    }

    #[test]
    fn inline_style_attribute_applies() {
        // Phase 5: inline style="" parsing — the value must land in the
        // element's computed style (highest-priority author origin).
        let doc = Document::parse("<div style=\"color: #ff0000\">x</div>");
        let resolver = StyleResolver::new();
        let styled = resolver.resolve(&doc);
        let div = find_styled(&styled.root, "div").expect("div");
        assert_eq!(div.style.get("color"), Some("#ff0000"));
    }

    #[test]
    fn background_shorthand_becomes_background_color() {
        // Phase 3: the `background:` shorthand's color is extracted under the
        // "background-color" key the paint layer reads.
        let sheet = StyleSheet::parse("div { background: #112233; }").unwrap();
        let decls = &sheet.rules[0].declarations;
        assert!(
            decls
                .iter()
                .any(|d| d.property == "background-color" && d.value == "#112233"),
            "background shorthand should yield background-color=#112233, got {decls:?}"
        );
    }

    #[test]
    fn color_inherits_to_descendant() {
        // Phase 3: a child without its own `color` inherits the parent's.
        let doc = Document::parse("<div><span>x</span></div>");
        let sheet = StyleSheet::parse("div { color: #00ff00; }").unwrap();
        let mut resolver = StyleResolver::new();
        resolver.add_sheet(sheet);
        let styled = resolver.resolve(&doc);
        let span = find_styled(&styled.root, "span").expect("span");
        assert_eq!(span.style.get("color"), Some("#00ff00"));
    }

    #[test]
    fn non_rendered_elements_are_display_none() {
        // <style>/<script>/<head> content must not paint — they are UA
        // display:none, so their raw text never leaks into the page.
        let doc = Document::parse("<style>body{color:red}</style><div>x</div>");
        let resolver = StyleResolver::new();
        let styled = resolver.resolve(&doc);
        let style_el = find_styled(&styled.root, "style").expect("style element");
        assert_eq!(style_el.style.display(), "none");
    }

    #[test]
    fn non_inherited_property_does_not_leak_to_child() {
        // background-color is NOT inherited — a child must not pick up the
        // parent's background.
        let doc = Document::parse("<div><span>x</span></div>");
        let sheet = StyleSheet::parse("div { background-color: #123456; }").unwrap();
        let mut resolver = StyleResolver::new();
        resolver.add_sheet(sheet);
        let styled = resolver.resolve(&doc);
        let span = find_styled(&styled.root, "span").expect("span");
        assert_eq!(span.style.get("background-color"), None);
    }
}
