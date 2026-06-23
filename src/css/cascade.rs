//! CSS cascade and style resolution.

use std::collections::HashMap;

use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, StyleSheet as LcssStyleSheet};
use lightningcss::traits::ToCss;
use lightningcss::values::color::CssColor;
use tracing::debug;

use crate::css::selector::{CompoundSelector, parse_selector_list};
use crate::css::values::{Color, Display, Length};
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
    /// The canonical selector text (e.g. `"div.main"`, `"#header"`) from
    /// lightningcss's own `ToCss` serializer. Kept as the human-readable
    /// canonical form; matching goes through [`StyleRule::compounds`].
    pub selector: String,
    /// Typed compound selectors parsed from [`StyleRule::selector`] — the
    /// rightmost compound of each complex selector in the list. The
    /// cascade matches an element when it satisfies **any** compound.
    pub compounds: Vec<CompoundSelector>,
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
                // Canonical selector text via lightningcss's own ToCss
                // serializer (e.g. `".box"`, `"div.card"`, `".nav a"`) —
                // NOT the Debug form. This is the text the typed selector
                // parser reads; it is the contract aligned with the matcher.
                let selector = style_rule
                    .selectors
                    .to_css_string(PrinterOptions::default())
                    .unwrap_or_default();
                let compounds = parse_selector_list(&selector);

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
                    compounds,
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
        // `display` value via lightningcss's canonical serializer — NOT
        // `format!("{d:?}")`, whose Debug form is
        // `"pair(displaypair { outside: block, inside: flow, ... })"`,
        // which `Display::parse` can't read (it falls back to `Inline`,
        // silently dropping `block`/`flex`/`none`). The canonical form is
        // `"block"` / `"flex"` / `"none"` / `"inline"`, which `Display::parse`
        // consumes directly. (Load-bearing fix surfaced by the testkit CSS
        // matrix — the display-mode rows were all resolving to inline.)
        Property::Display(_) => ("display".to_string(), css_value(prop)),
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

/// Which length-valued box property an accessor reads. Names the typed
/// length fields so [`ComputedStyle::length`] is one typed dispatch
/// instead of a string lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthProp {
    /// `width`.
    Width,
    /// `height`.
    Height,
    /// `font-size`.
    FontSize,
    /// `line-height`.
    LineHeight,
    /// `margin-top`.
    MarginTop,
    /// `margin-right`.
    MarginRight,
    /// `margin-bottom`.
    MarginBottom,
    /// `margin-left`.
    MarginLeft,
    /// `padding-top`.
    PaddingTop,
    /// `padding-right`.
    PaddingRight,
    /// `padding-bottom`.
    PaddingBottom,
    /// `padding-left`.
    PaddingLeft,
}

/// Computed style values for a single element — the CSS typescape.
///
/// Handled properties are stored as **typed** values (parsed once at the
/// cascade boundary in [`ComputedStyle::set`]); every other property falls
/// into [`ComputedStyle::other`] as a string so unknown CSS round-trips
/// through the compat [`ComputedStyle::get`] surface unchanged.
///
/// [`Default`] mirrors the CSS initial values the layout engine relied on:
/// `width`/`height`/`line-height`/`font-size` are `Auto` (no fixed size;
/// `font_size()` resolves Auto→16px), and margins/paddings are `0px`.
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    /// `display` mode (default [`Display::Inline`]).
    pub display: Display,
    /// `color` (text), `None` when unset.
    pub color: Option<Color>,
    /// `background-color`, `None` when unset.
    pub background_color: Option<Color>,
    /// `width` (default [`Length::Auto`]).
    pub width: Length,
    /// `height` (default [`Length::Auto`]).
    pub height: Length,
    /// `font-size` (default [`Length::Px(16.0)`]).
    pub font_size: Length,
    /// `line-height` (default [`Length::Auto`]).
    pub line_height: Length,
    /// `margin-top` (default [`Length::Px(0.0)`]).
    pub margin_top: Length,
    /// `margin-right` (default [`Length::Px(0.0)`]).
    pub margin_right: Length,
    /// `margin-bottom` (default [`Length::Px(0.0)`]).
    pub margin_bottom: Length,
    /// `margin-left` (default [`Length::Px(0.0)`]).
    pub margin_left: Length,
    /// `padding-top` (default [`Length::Px(0.0)`]).
    pub padding_top: Length,
    /// `padding-right` (default [`Length::Px(0.0)`]).
    pub padding_right: Length,
    /// `padding-bottom` (default [`Length::Px(0.0)`]).
    pub padding_bottom: Length,
    /// `padding-left` (default [`Length::Px(0.0)`]).
    pub padding_left: Length,
    /// `font-family` (raw value), `None` when unset.
    pub font_family: Option<String>,
    /// Every property without a typed field — kept as raw strings so
    /// unhandled CSS still round-trips through [`ComputedStyle::get`].
    pub other: HashMap<String, String>,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::default(),
            color: None,
            background_color: None,
            width: Length::Auto,
            height: Length::Auto,
            font_size: Length::Auto,
            line_height: Length::Auto,
            // Box-edge initial values are 0px (taffy treats unset as 0
            // too; this keeps the prior layout behavior exactly).
            margin_top: Length::Px(0.0),
            margin_right: Length::Px(0.0),
            margin_bottom: Length::Px(0.0),
            margin_left: Length::Px(0.0),
            padding_top: Length::Px(0.0),
            padding_right: Length::Px(0.0),
            padding_bottom: Length::Px(0.0),
            padding_left: Length::Px(0.0),
            font_family: None,
            other: HashMap::new(),
        }
    }
}

impl ComputedStyle {
    /// Set a property by name, **parsing into the typed field** when the
    /// property is one this typescape handles; otherwise the raw string
    /// lands in [`ComputedStyle::other`]. This is the cascade's
    /// parse-once boundary.
    ///
    /// Per ★★ UNREPRESENTABILITY: a value that fails to parse for a typed
    /// field is **dropped** (the field keeps its prior/default value),
    /// never stored as a silently-wrong typed value. An unparseable color
    /// → the field stays `None` (a visible gap, never a wrong color); an
    /// unparseable length → the field keeps its default (never a wrong
    /// box size).
    pub fn set(&mut self, property: impl Into<String>, value: impl Into<String>) {
        let property = property.into();
        let value = value.into();
        match property.as_str() {
            "display" => self.display = Display::parse(&value),
            "color" => {
                if let Some(c) = Color::parse(&value) {
                    self.color = Some(c);
                }
                // currentColor / unparseable: leave prior value (None).
            }
            "background-color" => {
                if let Some(c) = Color::parse(&value) {
                    self.background_color = Some(c);
                }
            }
            "width" => set_len(&mut self.width, &value),
            "height" => set_len(&mut self.height, &value),
            "font-size" => set_len(&mut self.font_size, &value),
            "line-height" => set_len(&mut self.line_height, &value),
            "margin-top" => set_len(&mut self.margin_top, &value),
            "margin-right" => set_len(&mut self.margin_right, &value),
            "margin-bottom" => set_len(&mut self.margin_bottom, &value),
            "margin-left" => set_len(&mut self.margin_left, &value),
            "padding-top" => set_len(&mut self.padding_top, &value),
            "padding-right" => set_len(&mut self.padding_right, &value),
            "padding-bottom" => set_len(&mut self.padding_bottom, &value),
            "padding-left" => set_len(&mut self.padding_left, &value),
            "font-family" => self.font_family = Some(value),
            _ => {
                self.other.insert(property, value);
            }
        }
    }

    /// Compat string accessor — reconstructs the canonical string form
    /// of a typed field, or reads [`ComputedStyle::other`] for unhandled
    /// properties. Returns an **owned** `String` (the typed fields have no
    /// stored string to borrow).
    #[must_use]
    pub fn get(&self, property: &str) -> Option<String> {
        match property {
            "display" => Some(display_str(self.display).to_string()),
            "color" => self.color.map(color_to_css),
            "background-color" => self.background_color.map(color_to_css),
            "width" => length_to_css(self.width),
            "height" => length_to_css(self.height),
            "font-size" => length_to_css(self.font_size),
            "line-height" => length_to_css(self.line_height),
            "margin-top" => length_to_css(self.margin_top),
            "margin-right" => length_to_css(self.margin_right),
            "margin-bottom" => length_to_css(self.margin_bottom),
            "margin-left" => length_to_css(self.margin_left),
            "padding-top" => length_to_css(self.padding_top),
            "padding-right" => length_to_css(self.padding_right),
            "padding-bottom" => length_to_css(self.padding_bottom),
            "padding-left" => length_to_css(self.padding_left),
            "font-family" => self.font_family.clone(),
            other => self.other.get(other).cloned(),
        }
    }

    // ── Typed accessors — the strongly-expressed consumer surface ──

    /// The typed `display` mode.
    #[must_use]
    pub fn display(&self) -> Display {
        self.display
    }

    /// The typed `color`, if set.
    #[must_use]
    pub fn color(&self) -> Option<Color> {
        self.color
    }

    /// The typed `background-color`, if set.
    #[must_use]
    pub fn background_color(&self) -> Option<Color> {
        self.background_color
    }

    /// The typed `font-size`, defaulting to `16px` when unset.
    #[must_use]
    pub fn font_size(&self) -> Length {
        match self.font_size {
            // Default font-size is 16px when the cascade left it Auto
            // (i.e. nobody set it). A set value (even Px(0)) is honored.
            Length::Auto => Length::Px(16.0),
            other => other,
        }
    }

    /// The typed length for a named box property.
    #[must_use]
    pub fn length(&self, which: LengthProp) -> Length {
        match which {
            LengthProp::Width => self.width,
            LengthProp::Height => self.height,
            LengthProp::FontSize => self.font_size(),
            LengthProp::LineHeight => self.line_height,
            LengthProp::MarginTop => self.margin_top,
            LengthProp::MarginRight => self.margin_right,
            LengthProp::MarginBottom => self.margin_bottom,
            LengthProp::MarginLeft => self.margin_left,
            LengthProp::PaddingTop => self.padding_top,
            LengthProp::PaddingRight => self.padding_right,
            LengthProp::PaddingBottom => self.padding_bottom,
            LengthProp::PaddingLeft => self.padding_left,
        }
    }
}

/// Parse `value` into a [`Length`], assigning it to `slot` only on a
/// successful parse. An unparseable length is dropped — `slot` keeps its
/// prior value (UNREPRESENTABILITY: never a wrong length).
fn set_len(slot: &mut Length, value: &str) {
    if let Some(len) = Length::parse(value) {
        *slot = len;
    }
}

/// Canonical lowercase string for a [`Display`] mode.
fn display_str(d: Display) -> &'static str {
    match d {
        Display::Inline => "inline",
        Display::Block => "block",
        Display::Flex => "flex",
        Display::None => "none",
    }
}

/// Serialize a [`Color`] to a canonical CSS string (`#rrggbb` opaque,
/// `rgba(r, g, b, a.aa)` translucent) for the compat `get()` surface.
/// This is a `Display`-style render of a typed value (TYPED EMISSION
/// surface #3), not free-form string composition of markup.
fn color_to_css(c: Color) -> String {
    let r = (c.r * 255.0).round() as u8;
    let g = (c.g * 255.0).round() as u8;
    let b = (c.b * 255.0).round() as u8;
    if (c.a - 1.0).abs() < f32::EPSILON {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r}, {g}, {b}, {a:.2})", a = c.a)
    }
}

/// Serialize a [`Length`] to a canonical CSS string for the compat
/// `get()` surface. `Auto` returns `None` (matches the prior behavior of
/// "no value stored"); the layout engine reads typed lengths directly.
fn length_to_css(l: Length) -> Option<String> {
    let s = match l {
        Length::Px(p) => format!("{p}px"),
        Length::Em(e) => format!("{e}em"),
        Length::Rem(r) => format!("{r}rem"),
        Length::Percent(p) => format!("{p}%"),
        Length::Vw(v) => format!("{v}vw"),
        Length::Vh(v) => format!("{v}vh"),
        Length::Auto => return None,
    };
    Some(s)
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
        // override. (Phase 3 cascade: property inheritance.) Values come from
        // the parent's typed fields via the compat `get()` and re-parse into
        // this node's typed fields via `set()` — typed in, typed out.
        if let Some(parent) = parent_style {
            for &prop in INHERITED_PROPERTIES {
                if let Some(value) = parent.get(prop) {
                    style.set(prop, value);
                }
            }
        }
        let tag;

        match &node.data {
            NodeData::Element(el) => {
                tag = el.tag.clone();
                // UA display defaults — typed. Non-rendered elements
                // (head/style/script/title/meta/...) are Display::None so
                // their text content never paints (otherwise raw CSS/JS
                // source leaks into the page); block elements default to
                // Display::Block; everything else stays Display::Inline.
                if is_non_rendered_element(&el.tag) {
                    style.display = Display::None;
                } else if is_block_element(&el.tag) {
                    style.display = Display::Block;
                }
                // Match rules against this element via typed compound
                // selectors — an element matches a rule when it satisfies
                // ANY of the rule's rightmost compounds.
                for sheet in &self.sheets {
                    for rule in &sheet.rules {
                        if rule.compounds.iter().any(|c| c.matches(el)) {
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
        assert_eq!(style.display(), Display::Inline);
        assert!(style.color().is_none());
        assert!(style.get("color").is_none());
        // font-size accessor defaults to 16px even though the field is Auto.
        assert_eq!(style.font_size(), Length::Px(16.0));
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
        assert_eq!(div.unwrap().style.display(), Display::Block);
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
        // Typed: the color parsed into the typed field.
        assert_eq!(div.style.color(), Color::parse("#ff0000"));
        // Compat string surface still resolves (now Option<String>).
        assert_eq!(div.style.get("color").as_deref(), Some("#ff0000"));
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
        assert_eq!(span.style.color(), Color::parse("#00ff00"));
        assert_eq!(span.style.get("color").as_deref(), Some("#00ff00"));
    }

    #[test]
    fn non_rendered_elements_are_display_none() {
        // <style>/<script>/<head> content must not paint — they are UA
        // display:none, so their raw text never leaks into the page.
        let doc = Document::parse("<style>body{color:red}</style><div>x</div>");
        let resolver = StyleResolver::new();
        let styled = resolver.resolve(&doc);
        let style_el = find_styled(&styled.root, "style").expect("style element");
        assert_eq!(style_el.style.display(), Display::None);
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
        assert_eq!(span.style.background_color(), None);
        assert_eq!(span.style.get("background-color"), None);
    }

    // ── New typed-cascade tests ──────────────────────────────────────

    #[test]
    fn class_selector_matches_typed() {
        // .box {background-color:#ff0000} must color a <div class="box">
        // but NOT a bare <div> — the substring-hack bug is gone.
        let doc = Document::parse("<div class=\"box\">x</div><div>y</div>");
        let sheet = StyleSheet::parse(".box { background-color: #ff0000 }").unwrap();
        let mut resolver = StyleResolver::new();
        resolver.add_sheet(sheet);
        let styled = resolver.resolve(&doc);

        // Collect all divs.
        let mut divs = Vec::new();
        fn collect<'a>(n: &'a StyledNode, out: &mut Vec<&'a StyledNode>) {
            if n.tag == "div" {
                out.push(n);
            }
            for c in &n.children {
                collect(c, out);
            }
        }
        collect(&styled.root, &mut divs);
        assert_eq!(divs.len(), 2);
        // The .box div is red; the bare div has no background.
        assert_eq!(divs[0].style.background_color(), Color::parse("#ff0000"));
        assert_eq!(divs[1].style.background_color(), None);
    }

    #[test]
    fn id_selector_matches_typed() {
        let doc = Document::parse("<p id=\"lead\">x</p><p>y</p>");
        let sheet = StyleSheet::parse("#lead { color: blue }").unwrap();
        let mut resolver = StyleResolver::new();
        resolver.add_sheet(sheet);
        let styled = resolver.resolve(&doc);
        let mut ps = Vec::new();
        fn collect<'a>(n: &'a StyledNode, out: &mut Vec<&'a StyledNode>) {
            if n.tag == "p" {
                out.push(n);
            }
            for c in &n.children {
                collect(c, out);
            }
        }
        collect(&styled.root, &mut ps);
        assert_eq!(ps[0].style.color(), Color::parse("blue"));
        assert_eq!(ps[1].style.color(), None);
    }

    #[test]
    fn compound_selector_requires_both() {
        // div.card colors only a <div class="card">.
        let doc =
            Document::parse("<div class=\"card\">a</div><div>b</div><span class=\"card\">c</span>");
        let sheet = StyleSheet::parse("div.card { color: red }").unwrap();
        let mut resolver = StyleResolver::new();
        resolver.add_sheet(sheet);
        let styled = resolver.resolve(&doc);
        let mut nodes = Vec::new();
        fn collect<'a>(n: &'a StyledNode, out: &mut Vec<&'a StyledNode>) {
            out.push(n);
            for c in &n.children {
                collect(c, out);
            }
        }
        collect(&styled.root, &mut nodes);
        let div_card = nodes
            .iter()
            .find(|n| n.tag == "div")
            .filter(|n| n.style.color() == Color::parse("red"));
        assert!(div_card.is_some(), "div.card should be red");
        // The bare span.card must NOT match div.card.
        let span = nodes.iter().find(|n| n.tag == "span").unwrap();
        assert_eq!(span.style.color(), None);
    }

    #[test]
    fn typed_length_round_trips_through_set_get() {
        let mut style = ComputedStyle::default();
        style.set("width", "50vw");
        assert_eq!(style.width, Length::Vw(50.0));
        assert_eq!(style.get("width").as_deref(), Some("50vw"));
        style.set("margin-left", "2em");
        assert_eq!(style.margin_left, Length::Em(2.0));
    }

    #[test]
    fn set_drops_unparseable_value_never_silent_wrong() {
        let mut style = ComputedStyle::default();
        // Seed a good value, then try to overwrite with garbage.
        style.set("width", "100px");
        style.set("width", "not-a-length");
        // The garbage is dropped — the prior good value stays.
        assert_eq!(style.width, Length::Px(100.0));
        // An unparseable color leaves the field None (a visible gap).
        style.set("color", "notacolor");
        assert_eq!(style.color(), None);
    }

    #[test]
    fn unhandled_property_falls_into_other() {
        let mut style = ComputedStyle::default();
        style.set("text-align", "center");
        assert_eq!(style.get("text-align").as_deref(), Some("center"));
        assert_eq!(
            style.other.get("text-align").map(String::as_str),
            Some("center")
        );
    }

    #[test]
    fn font_size_inherits_typed() {
        let doc = Document::parse("<div><span>x</span></div>");
        let sheet = StyleSheet::parse("div { font-size: 24px }").unwrap();
        let mut resolver = StyleResolver::new();
        resolver.add_sheet(sheet);
        let styled = resolver.resolve(&doc);
        let span = find_styled(&styled.root, "span").expect("span");
        assert_eq!(span.style.font_size(), Length::Px(24.0));
    }
}

/// The same cascade assertions, re-expressed through the ONE
/// `nami_core::testkit` vocabulary — the standardization layer. These
/// run only under `--features testkit`; the hand-rolled tests above keep
/// the default-build coverage. Adding the vocabulary form here proves the
/// cascade reads the same as every other rendering test.
#[cfg(all(test, feature = "testkit"))]
mod testkit_migrated {
    use crate::css::cascade::LengthProp;
    use crate::css::values::{Color, Display, Length};
    use crate::testkit::Probe;

    #[test]
    fn class_selector_matches_typed() {
        Probe::html("<style>.box{background-color:#ff0000}</style>\
                     <div class=\"box\">x</div><div id=\"bare\">y</div>")
            .style(".box")
            .background(Color::hex("#ff0000"));
        Probe::html("<style>.box{background-color:#ff0000}</style>\
                     <div class=\"box\">x</div><div id=\"bare\">y</div>")
            .style("#bare")
            .missing("background-color");
    }

    #[test]
    fn id_selector_matches_typed() {
        Probe::html("<style>#lead{color:blue}</style><p id=\"lead\">x</p>")
            .style("#lead")
            .color(Color::rgb8(0, 0, 255));
    }

    #[test]
    fn compound_selector_requires_both() {
        Probe::html("<style>div.card{color:red}</style>\
                     <div class=\"card\">a</div><span class=\"card\">c</span>")
            .style("div.card")
            .color(Color::rgb8(255, 0, 0));
        Probe::html("<style>div.card{color:red}</style>\
                     <div class=\"card\">a</div><span class=\"card\">c</span>")
            .style("span")
            .missing("color");
    }

    #[test]
    fn color_inherits_to_descendant() {
        Probe::html("<style>div{color:#00ff00}</style><div><span>x</span></div>")
            .style("span")
            .color(Color::hex("#00ff00"));
    }

    #[test]
    fn non_inherited_property_does_not_leak() {
        Probe::html("<style>div{background-color:#123456}</style><div><span>x</span></div>")
            .style("span")
            .missing("background-color");
    }

    #[test]
    fn font_size_inherits_typed() {
        Probe::html("<style>div{font-size:24px}</style><div><span>x</span></div>")
            .style("span")
            .length(LengthProp::FontSize, Length::Px(24.0));
    }

    #[test]
    fn inline_style_attribute_applies() {
        Probe::html("<div style=\"color:#ff0000\">x</div>")
            .style("div")
            .color(Color::hex("#ff0000"))
            .raw("color", "#ff0000");
    }

    #[test]
    fn background_shorthand_becomes_background_color() {
        Probe::html("<style>div{background:#112233;width:10px;height:10px}</style><div></div>")
            .style("div")
            .background(Color::hex("#112233"));
    }

    #[test]
    fn block_elements_get_block_display() {
        Probe::html("<div>content</div>").style("div").display(Display::Block);
    }

    #[test]
    fn non_rendered_elements_are_display_none() {
        Probe::html("<style>body{color:red}</style><div>x</div>")
            .style("style")
            .display(Display::None);
    }
}
