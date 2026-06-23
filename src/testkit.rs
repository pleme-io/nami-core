//! `nami_core::testkit` — the ONE fluent rendering-assertion vocabulary.
//!
//! Every rendering assumption in the pure-Rust browser pipeline (CSS
//! cascade → layout → paint) is verified through this single typed
//! vocabulary, so the whole fleet of rendering tests reads the same way.
//! This operationalizes the PRIME DIRECTIVE (one shared test library, no
//! duplicated assertion shapes), the CLOSED-LOOP MASS-SYNTHESIS rule (the
//! [`Probe`] is what the verification matrix in `tests/css_matrix.rs`
//! drives every row through), and UNREPRESENTABILITY (each rendering
//! assumption is named explicitly and checked, never left implicit).
//!
//! It is **pure** — no GPU, no I/O. The GPU pixel half lives in the
//! consumer (namimado's `render::testkit::PixelProbe`), built on this
//! same conceptual surface but over real pixels.
//!
//! ## The vocabulary
//!
//! ```ignore
//! use nami_core::testkit::Probe;
//! use nami_core::css::{Color, Display, Length, LengthProp};
//!
//! Probe::html("<div class='card'>Hi</div>")
//!     .style(".card").display(Display::Block).color(Color::rgb8(0, 0, 0));
//!
//! Probe::html("<div style='width:200px;height:100px'></div>")
//!     .viewport(1280.0, 800.0)
//!     .layout("div").width(200.0).height(100.0).exists();
//!
//! Probe::html("<div style='background:#3050ff;width:50px;height:50px'></div>")
//!     .paint().rect_with_color(Color::hex("#3050ff"));
//! ```
//!
//! Each chainable assertion returns `Self` and **panics on mismatch**
//! with an EXACT, expected-vs-got message — so a failing rendering
//! assumption is immediately legible.
//!
//! ## How a [`Probe`] drives the pipeline
//!
//! `Probe::html` snapshots the source HTML + extracts any inline
//! `<style>` blocks. Each terminal (`style` / `layout` / `paint`) re-runs
//! the relevant slice of the real pipeline — [`Document::parse`] →
//! [`StyleResolver::resolve`] → [`LayoutEngine::compute`] →
//! [`paint::build_display_list`] — so the vocabulary asserts against the
//! SAME code paths the live browser runs, never a stubbed re-implementation.

use crate::css::cascade::{LengthProp, StyleResolver, StyleSheet, StyledNode, StyledTree};
use crate::css::selector::CompoundSelector;
use crate::css::values::{Color, Display, Length};
use crate::dom::{Document, ElementData, NodeData};
use crate::layout::{LayoutBox, LayoutEngine, Size};
use crate::paint::{self, DisplayList, DrawCmd};

/// Pixel epsilon for f32 layout-coordinate comparisons. Half a logical
/// pixel — tight enough to catch a real layout regression, loose enough
/// to absorb taffy's f32 rounding.
const PX_EPS: f32 = 0.5;

/// Per-channel epsilon for [`Color`] comparisons. sRGB-intent channels
/// are f32 in `0.0..=1.0`; a difference under this is "the same color"
/// (a `u8` channel round-trips to ≈ `1/255 ≈ 0.0039`, well under `0.01`).
const COLOR_EPS: f32 = 0.01;

/// Whether two colors are equal within [`COLOR_EPS`] on every channel.
fn color_eq(a: Color, b: Color) -> bool {
    (a.r - b.r).abs() < COLOR_EPS
        && (a.g - b.g).abs() < COLOR_EPS
        && (a.b - b.b).abs() < COLOR_EPS
        && (a.a - b.a).abs() < COLOR_EPS
}

/// Render a [`Color`] for an assertion message — the canonical
/// `rgba(r, g, b, a)` form (a `Display`-style render of a typed value;
/// TYPED EMISSION surface — no free-form markup composition).
fn show_color(c: Color) -> String {
    let to8 = |f: f32| (f * 255.0).round() as i32;
    format!(
        "rgba({}, {}, {}, {:.3})",
        to8(c.r),
        to8(c.g),
        to8(c.b),
        c.a
    )
}

/// Render a [`Length`] for an assertion message.
fn show_length(l: Length) -> String {
    match l {
        Length::Px(p) => format!("{p}px"),
        Length::Em(e) => format!("{e}em"),
        Length::Rem(r) => format!("{r}rem"),
        Length::Percent(p) => format!("{p}%"),
        Length::Vw(v) => format!("{v}vw"),
        Length::Vh(v) => format!("{v}vh"),
        Length::Auto => "auto".to_string(),
    }
}

/// Canonical lowercase keyword for a [`Display`] mode (matches the
/// cascade's own `display_str`).
fn show_display(d: Display) -> &'static str {
    match d {
        Display::Inline => "inline",
        Display::Block => "block",
        Display::Flex => "flex",
        Display::None => "none",
    }
}

/// Map a [`LengthProp`] back to its CSS property name for messages.
fn length_prop_name(p: LengthProp) -> &'static str {
    match p {
        LengthProp::Width => "width",
        LengthProp::Height => "height",
        LengthProp::FontSize => "font-size",
        LengthProp::LineHeight => "line-height",
        LengthProp::MarginTop => "margin-top",
        LengthProp::MarginRight => "margin-right",
        LengthProp::MarginBottom => "margin-bottom",
        LengthProp::MarginLeft => "margin-left",
        LengthProp::PaddingTop => "padding-top",
        LengthProp::PaddingRight => "padding-right",
        LengthProp::PaddingBottom => "padding-bottom",
        LengthProp::PaddingLeft => "padding-left",
    }
}

/// The entry point of the rendering-assertion vocabulary. Holds an HTML
/// source + a viewport; produces typed assertion builders that each
/// re-run the real pipeline.
///
/// The default viewport is `1280x800` (the namimado headless default), so
/// `vw`/`vh` resolve the same way the GPU harness sees them; override with
/// [`Probe::viewport`].
#[derive(Debug, Clone)]
pub struct Probe {
    html: String,
    viewport: Size,
}

impl Probe {
    /// Start a probe over an HTML source string. Inline `<style>` blocks
    /// are honored (the cascade extracts them); external stylesheets are
    /// not fetched (this is the offline, pure path).
    #[must_use]
    pub fn html(html: &str) -> Probe {
        Probe {
            html: html.to_string(),
            viewport: Size::new(1280.0, 800.0),
        }
    }

    /// Override the viewport used for layout + `vw`/`vh` resolution.
    #[must_use]
    pub fn viewport(mut self, w: f32, h: f32) -> Probe {
        self.viewport = Size::new(w, h);
        self
    }

    /// Resolve the document + styled tree (the cascade half of the
    /// pipeline). Shared by the `style` / `layout` / `paint` terminals.
    fn resolve(&self) -> (Document, StyledTree) {
        let doc = Document::parse(&self.html);
        let mut resolver = StyleResolver::new();
        for css in extract_style_blocks(&doc) {
            if let Ok(sheet) = StyleSheet::parse(&css) {
                resolver.add_sheet(sheet);
            }
        }
        let styled = resolver.resolve(&doc);
        (doc, styled)
    }

    /// Assert on the first element matching `selector`'s computed style.
    ///
    /// Runs [`Document::parse`] + [`StyleResolver::resolve`], then finds
    /// the first styled element whose [`ElementData`] satisfies the typed
    /// [`CompoundSelector`] parsed from `selector` (the SAME matcher the
    /// cascade uses). Panics if no element matches.
    ///
    /// # Panics
    /// Panics if no element matches `selector`.
    #[must_use]
    pub fn style(&self, selector: &str) -> StyleAssert {
        let (doc, styled) = self.resolve();
        let sel = CompoundSelector::parse(selector);
        let node = find_first_styled(&doc, &styled, &sel).unwrap_or_else(|| {
            panic!("style({selector}): no element matched the selector");
        });
        StyleAssert {
            selector: selector.to_string(),
            node,
        }
    }

    /// Assert on the layout boxes of elements matching `selector`.
    ///
    /// Runs the full cascade + [`LayoutEngine::compute`] at the probe's
    /// viewport, then collects every [`LayoutBox`] whose `node_index`
    /// cross-references a styled element matching `selector`.
    #[must_use]
    pub fn layout(&self, selector: &str) -> LayoutAssert {
        let (doc, styled) = self.resolve();
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, self.viewport);
        let sel = CompoundSelector::parse(selector);
        // node_index → matches-selector, built from the styled+dom trees.
        let matching = matching_node_indices(&doc, &styled, &sel);
        let mut boxes = Vec::new();
        collect_matching_boxes(&layout.root, &matching, &mut boxes);
        LayoutAssert {
            selector: selector.to_string(),
            boxes,
        }
    }

    /// Assert on the [`DisplayList`] the paint layer produces.
    ///
    /// Runs the full pipeline through [`paint::build_display_list`] at the
    /// probe's viewport.
    #[must_use]
    pub fn paint(&self) -> PaintAssert {
        let (doc, styled) = self.resolve();
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, self.viewport);
        let list = paint::build_display_list(&layout, &styled, &doc);
        PaintAssert { list }
    }
}

/// Extract the text of every `<style>` element in the document — the
/// inline-stylesheet source the cascade resolves against. Mirrors what
/// `NamiNativeEngine` does before `StyleSheet::parse`.
fn extract_style_blocks(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    for node in doc.root.descendants() {
        if let NodeData::Element(el) = &node.data {
            if el.tag.eq_ignore_ascii_case("style") {
                let mut css = String::new();
                for child in &node.children {
                    if let Some(t) = child.as_text() {
                        css.push_str(t);
                    }
                }
                if !css.trim().is_empty() {
                    out.push(css);
                }
            }
        }
    }
    out
}

/// Depth-first: the first styled element whose backing [`ElementData`]
/// matches the compound. The styled tree carries `node_index`, the DOM
/// carries the element; we walk both in the SAME preorder the cascade
/// numbered them with, so the index aligns.
fn find_first_styled(doc: &Document, styled: &StyledTree, sel: &CompoundSelector) -> Option<ComputedClone> {
    let dom: Vec<&crate::dom::Node> = doc.root.descendants().collect();
    let mut found = None;
    walk_first(&styled.root, &dom, sel, &mut found);
    found
}

fn walk_first(
    node: &StyledNode,
    dom: &[&crate::dom::Node],
    sel: &CompoundSelector,
    out: &mut Option<ComputedClone>,
) {
    if out.is_some() {
        return;
    }
    if let Some(el) = dom.get(node.node_index).and_then(|n| element_of(n)) {
        if sel.matches(el) {
            *out = Some(ComputedClone {
                tag: node.tag.clone(),
                style: node.style.clone(),
            });
            return;
        }
    }
    for child in &node.children {
        walk_first(child, dom, sel, out);
        if out.is_some() {
            return;
        }
    }
}

/// Borrow the [`ElementData`] of a DOM node, if it is an element.
fn element_of(node: &crate::dom::Node) -> Option<&ElementData> {
    match &node.data {
        NodeData::Element(el) => Some(el),
        _ => None,
    }
}

/// The set of `node_index` values whose styled element matches a compound.
fn matching_node_indices(
    doc: &Document,
    styled: &StyledTree,
    sel: &CompoundSelector,
) -> Vec<usize> {
    let dom: Vec<&crate::dom::Node> = doc.root.descendants().collect();
    let mut out = Vec::new();
    collect_matching_indices(&styled.root, &dom, sel, &mut out);
    out
}

fn collect_matching_indices(
    node: &StyledNode,
    dom: &[&crate::dom::Node],
    sel: &CompoundSelector,
    out: &mut Vec<usize>,
) {
    if let Some(el) = dom.get(node.node_index).and_then(|n| element_of(n)) {
        if sel.matches(el) {
            out.push(node.node_index);
        }
    }
    for child in &node.children {
        collect_matching_indices(child, dom, sel, out);
    }
}

/// Collect every layout box whose `node_index` is in `matching`.
fn collect_matching_boxes(lbox: &LayoutBox, matching: &[usize], out: &mut Vec<LayoutBox>) {
    if matching.contains(&lbox.node_index) {
        out.push(lbox.clone());
    }
    for child in &lbox.children {
        collect_matching_boxes(child, matching, out);
    }
}

/// A detached snapshot of a matched element's tag + computed style — the
/// [`StyleAssert`] owns it so it doesn't borrow the transient styled tree.
#[derive(Debug, Clone)]
struct ComputedClone {
    #[allow(dead_code)]
    tag: String,
    style: crate::css::cascade::ComputedStyle,
}

// ── StyleAssert ─────────────────────────────────────────────────────

/// Chainable assertions over one element's [`ComputedStyle`]. Every
/// method returns `Self` and panics with an exact
/// `style(<sel>).<prop>: expected <x>, got <y>` message on mismatch.
#[derive(Debug, Clone)]
pub struct StyleAssert {
    selector: String,
    node: ComputedClone,
}

impl StyleAssert {
    /// Assert the typed `display` mode.
    pub fn display(self, expected: Display) -> StyleAssert {
        let got = self.node.style.display();
        assert!(
            got == expected,
            "style({}).display: expected {}, got {}",
            self.selector,
            show_display(expected),
            show_display(got)
        );
        self
    }

    /// Assert the typed `color`.
    pub fn color(self, expected: Color) -> StyleAssert {
        match self.node.style.color() {
            Some(got) => assert!(
                color_eq(got, expected),
                "style({}).color: expected {}, got {}",
                self.selector,
                show_color(expected),
                show_color(got)
            ),
            None => panic!(
                "style({}).color: expected {}, got <unset>",
                self.selector,
                show_color(expected)
            ),
        }
        self
    }

    /// Assert the typed `background-color`.
    pub fn background(self, expected: Color) -> StyleAssert {
        match self.node.style.background_color() {
            Some(got) => assert!(
                color_eq(got, expected),
                "style({}).background: expected {}, got {}",
                self.selector,
                show_color(expected),
                show_color(got)
            ),
            None => panic!(
                "style({}).background: expected {}, got <unset>",
                self.selector,
                show_color(expected)
            ),
        }
        self
    }

    /// Assert a typed length-valued box property.
    pub fn length(self, which: LengthProp, expected: Length) -> StyleAssert {
        let got = self.node.style.length(which);
        assert!(
            got == expected,
            "style({}).{}: expected {}, got {}",
            self.selector,
            length_prop_name(which),
            show_length(expected),
            show_length(got)
        );
        self
    }

    /// Assert a raw (string) property value via the compat
    /// [`ComputedStyle::get`] surface — for properties without a typed
    /// field, and for asserting the canonical string of a typed one.
    pub fn raw(self, prop: &str, value: &str) -> StyleAssert {
        match self.node.style.get(prop) {
            Some(got) => assert!(
                got == value,
                "style({}).{prop}: expected {value:?}, got {got:?}",
                self.selector
            ),
            None => panic!(
                "style({}).{prop}: expected {value:?}, got <unset>",
                self.selector
            ),
        }
        self
    }

    /// Assert a property is **unset** (the compat surface returns `None`).
    /// The negative assertion for inheritance / non-application proofs.
    pub fn missing(self, prop: &str) -> StyleAssert {
        let got = self.node.style.get(prop);
        assert!(
            got.is_none(),
            "style({}).{prop}: expected <unset>, got {got:?}",
            self.selector
        );
        self
    }
}

// ── LayoutAssert ────────────────────────────────────────────────────

/// Chainable assertions over the layout boxes matching a selector. f32
/// comparisons use a 0.5px epsilon. Geometry assertions (`width` /
/// `height` / `pos`) operate on the FIRST matching box; `exists` /
/// `count` operate on the whole match set.
#[derive(Debug, Clone)]
pub struct LayoutAssert {
    selector: String,
    boxes: Vec<LayoutBox>,
}

impl LayoutAssert {
    fn first(&self) -> &LayoutBox {
        self.boxes.first().unwrap_or_else(|| {
            panic!(
                "layout({}): no layout box matched (cannot assert geometry)",
                self.selector
            )
        })
    }

    /// Assert the first matching box's width (±0.5px).
    pub fn width(self, px: f32) -> LayoutAssert {
        let got = self.first().width;
        assert!(
            (got - px).abs() < PX_EPS,
            "layout({}).width: expected {px}px, got {got}px",
            self.selector
        );
        self
    }

    /// Assert the first matching box's height (±0.5px).
    pub fn height(self, px: f32) -> LayoutAssert {
        let got = self.first().height;
        assert!(
            (got - px).abs() < PX_EPS,
            "layout({}).height: expected {px}px, got {got}px",
            self.selector
        );
        self
    }

    /// Assert the first matching box's `(x, y)` position (±0.5px each).
    pub fn pos(self, x: f32, y: f32) -> LayoutAssert {
        let b = self.first();
        assert!(
            (b.x - x).abs() < PX_EPS && (b.y - y).abs() < PX_EPS,
            "layout({}).pos: expected ({x}, {y}), got ({}, {})",
            self.selector,
            b.x,
            b.y
        );
        self
    }

    /// Assert at least one box matched.
    pub fn exists(self) -> LayoutAssert {
        assert!(
            !self.boxes.is_empty(),
            "layout({}).exists: expected ≥1 box, got 0",
            self.selector
        );
        self
    }

    /// Assert the exact number of matching boxes.
    pub fn count(self, n: usize) -> LayoutAssert {
        let got = self.boxes.len();
        assert!(
            got == n,
            "layout({}).count: expected {n}, got {got}",
            self.selector
        );
        self
    }
}

// ── PaintAssert ─────────────────────────────────────────────────────

/// Chainable assertions over the paint [`DisplayList`]. Color matches use
/// the channel epsilon; text matches are substring containment.
#[derive(Debug, Clone)]
pub struct PaintAssert {
    list: DisplayList,
}

impl PaintAssert {
    /// Assert some `Rect` command paints ≈ `color`.
    pub fn rect_with_color(self, color: Color) -> PaintAssert {
        let found = self.list.cmds.iter().any(|c| match c {
            DrawCmd::Rect { color: rc, .. } => color_eq(*rc, color),
            _ => false,
        });
        assert!(
            found,
            "paint().rect_with_color: expected a Rect ≈ {}, got none ({} cmds: {})",
            show_color(color),
            self.list.cmds.len(),
            self.summarize_rects()
        );
        self
    }

    /// Assert some `Text` command's content contains `substr`.
    pub fn text(self, substr: &str) -> PaintAssert {
        let found = self.list.cmds.iter().any(|c| match c {
            DrawCmd::Text { text, .. } => text.contains(substr),
            _ => false,
        });
        assert!(
            found,
            "paint().text: expected a Text containing {substr:?}, got none ({})",
            self.summarize_texts()
        );
        self
    }

    /// Assert some `Text` command contains `substr` AND is painted ≈ `color`.
    pub fn text_with_color(self, substr: &str, color: Color) -> PaintAssert {
        let found = self.list.cmds.iter().any(|c| match c {
            DrawCmd::Text { text, color: tc, .. } => text.contains(substr) && color_eq(*tc, color),
            _ => false,
        });
        assert!(
            found,
            "paint().text_with_color: expected a Text containing {substr:?} in {}, got none ({})",
            show_color(color),
            self.summarize_texts()
        );
        self
    }

    /// Assert NO `Text` command contains `substr` — the negative.
    pub fn no_text(self, substr: &str) -> PaintAssert {
        let offending = self.list.cmds.iter().find_map(|c| match c {
            DrawCmd::Text { text, .. } if text.contains(substr) => Some(text.clone()),
            _ => None,
        });
        assert!(
            offending.is_none(),
            "paint().no_text: expected NO Text containing {substr:?}, got {:?}",
            offending
        );
        self
    }

    /// Assert the exact number of `Rect` commands.
    pub fn rect_count(self, n: usize) -> PaintAssert {
        let got = self
            .list
            .cmds
            .iter()
            .filter(|c| matches!(c, DrawCmd::Rect { .. }))
            .count();
        assert!(
            got == n,
            "paint().rect_count: expected {n}, got {got} ({})",
            self.summarize_rects()
        );
        self
    }

    /// Compact dump of the list's Rect colors for failure messages.
    fn summarize_rects(&self) -> String {
        let rects: Vec<String> = self
            .list
            .cmds
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Rect { color, .. } => Some(show_color(*color)),
                _ => None,
            })
            .collect();
        format!("rects=[{}]", rects.join(", "))
    }

    /// Compact dump of the list's Text contents for failure messages.
    fn summarize_texts(&self) -> String {
        let texts: Vec<String> = self
            .list
            .cmds
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(format!("{text:?}")),
                _ => None,
            })
            .collect();
        format!("texts=[{}]", texts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Color ergonomic constructors ────────────────────────────────

    #[test]
    fn rgb8_and_hex_match_parse() {
        assert!(color_eq(Color::rgb8(255, 0, 0), Color::parse("red").unwrap()));
        assert!(color_eq(Color::hex("#3050ff"), Color::parse("#3050ff").unwrap()));
    }

    #[test]
    #[should_panic(expected = "invalid hex color literal")]
    fn hex_panics_on_garbage() {
        let _ = Color::hex("not-a-hex");
    }

    // ── Probe::style ────────────────────────────────────────────────

    #[test]
    fn style_display_and_color() {
        Probe::html("<div class=\"card\" style=\"color:#00ff00\">hi</div>")
            .style(".card")
            .display(Display::Block)
            .color(Color::rgb8(0, 255, 0));
    }

    #[test]
    fn style_background_from_style_block() {
        Probe::html("<style>.box{background-color:#3050ff}</style><div class=\"box\">x</div>")
            .style(".box")
            .background(Color::hex("#3050ff"));
    }

    #[test]
    fn style_length_and_raw() {
        Probe::html("<div id=\"h\" style=\"width:200px\">x</div>")
            .style("#h")
            .length(LengthProp::Width, Length::Px(200.0))
            .raw("width", "200px");
    }

    #[test]
    fn style_missing_for_unset() {
        Probe::html("<div>x</div>").style("div").missing("background-color");
    }

    #[test]
    #[should_panic(expected = "style(.card).color: expected")]
    fn style_color_mismatch_panics_with_message() {
        Probe::html("<div class=\"card\" style=\"color:#ff0000\">x</div>")
            .style(".card")
            .color(Color::rgb8(0, 0, 255));
    }

    #[test]
    #[should_panic(expected = "no element matched")]
    fn style_no_match_panics() {
        let _ = Probe::html("<div>x</div>").style(".nope");
    }

    // ── Probe::layout ───────────────────────────────────────────────

    #[test]
    fn layout_width_height_exists() {
        Probe::html("<div style=\"width:200px;height:100px\"></div>")
            .layout("div")
            .exists()
            .width(200.0)
            .height(100.0);
    }

    #[test]
    fn layout_vw_resolves_against_viewport() {
        Probe::html("<div style=\"width:50vw;height:40px\"></div>")
            .viewport(1280.0, 800.0)
            .layout("div")
            .width(640.0);
    }

    #[test]
    fn layout_count() {
        Probe::html("<p>a</p><p>b</p><p>c</p>")
            .layout("p")
            .count(3);
    }

    // ── Probe::paint ────────────────────────────────────────────────

    #[test]
    fn paint_rect_and_text() {
        Probe::html(
            "<style>div{background-color:#3050ff;width:50px;height:50px} \
             p{color:#ffffff;height:20px}</style><div><p>Hello</p></div>",
        )
        .paint()
        .rect_with_color(Color::hex("#3050ff"))
        .text("Hello")
        .text_with_color("Hello", Color::rgb8(255, 255, 255));
    }

    #[test]
    fn paint_no_text_negative() {
        Probe::html("<div style=\"width:10px;height:10px;background-color:#000000\"></div>")
            .paint()
            .no_text("Hello");
    }
}
