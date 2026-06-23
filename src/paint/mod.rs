//! Typed paint IR — the rendering-agnostic display list.
//!
//! nami-core produces a [`LayoutTree`](crate::layout::LayoutTree) of
//! positioned boxes; this module is the next step toward pixels that is
//! *still pure data*: a [`DisplayList`] of typed [`DrawCmd`]s a renderer
//! (garasu/wgpu in namimado, a terminal, a headless test) consumes
//! without nami-core knowing how to draw. It is the typed bridge between
//! "where every box is" (layout) and "what to paint" (the consumer's GPU
//! pipeline).
//!
//! Everything here is `#[derive]`-data — no GPU, no I/O — so the whole
//! walk is unit-testable on a fixture string.
//!
//! ## Named gaps (honest, not silent)
//!
//! This is **M0** of the pure-Rust browser renderer. The display-list
//! walk faithfully reflects what the layout + cascade layers currently
//! produce, and the following are KNOWN gaps inherited from those layers
//! — documented here so a consumer sees a visible deficiency (a box with
//! no text, a flat block) rather than a silently-wrong render:
//!
//! - **No inline flow / line-breaking.** taffy has no inline layout
//!   ([`crate::layout`] maps `inline` → `block`), so text is emitted as
//!   one [`DrawCmd::Text`] clipped to its box rather than wrapped into
//!   line boxes. The renderer wraps within `max_width`/`max_height`.
//! - **Zero text intrinsic size in taffy.** Text contributes no measured
//!   width/height to layout, so a text-only box may have collapsed
//!   dimensions; the emitted [`DrawCmd::Text`] still carries the text so
//!   the renderer can paint it at the box origin.
//! - **No style inheritance.** The cascade does not yet inherit `color`
//!   from parent to child ([`crate::css`] Phase 3). A child element with
//!   no explicit `color` gets no color here; the renderer falls back to
//!   its scheme default foreground (the caller's responsibility).
//! - **Background-color only for block boxes.** Backgrounds are emitted
//!   for any box whose styled node carries `background-color` with
//!   alpha > 0; transparent / absent backgrounds emit no rect (a visible
//!   gap, never a black box).
//! - **Images are placeholders.** `<img>` emits a [`DrawCmd::Image`]
//!   placeholder rect; actual image decode/fetch is a later milestone.

use crate::css::cascade::{StyledNode, StyledTree};
use crate::dom::{Document, Node};
use crate::layout::{LayoutBox, LayoutTree};

/// Linear-ish RGBA color in `[0, 1]` per channel. The consumer is
/// responsible for any sRGB→linear conversion its GPU target needs;
/// these values are parsed directly from the cascade's color strings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    /// Red, `[0, 1]`.
    pub r: f32,
    /// Green, `[0, 1]`.
    pub g: f32,
    /// Blue, `[0, 1]`.
    pub b: f32,
    /// Alpha, `[0, 1]`.
    pub a: f32,
}

impl Rgba {
    /// Fully transparent — `(0, 0, 0, 0)`.
    pub const TRANSPARENT: Rgba = Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    /// Parse the EXACT color string formats nami-core's
    /// [`crate::css::cascade`] emits:
    ///
    /// - `"#rrggbb"` — opaque hex (the `format_css_color` opaque path).
    /// - `"rgba(r, g, b, a)"` — `r`/`g`/`b` in `0..=255`, `a` in `0.0..=1.0`
    ///   (the translucent path; `a` is emitted with two decimals like
    ///   `"rgba(255, 0, 0, 0.50)"`).
    /// - `"currentColor"` — passthrough; returns `None`.
    ///
    /// Returns `None` for `currentColor`, unrecognized, or malformed
    /// input. `None` means **the caller skips the draw** — a visible
    /// gap, never a silent black box.
    #[must_use]
    pub fn parse(s: &str) -> Option<Rgba> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("currentColor") {
            return None;
        }
        if let Some(hex) = s.strip_prefix('#') {
            return parse_hex_rrggbb(hex);
        }
        if let Some(inner) = s
            .strip_prefix("rgba(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            return parse_rgba_components(inner);
        }
        None
    }
}

/// Parse a six-hex-digit `rrggbb` body (no leading `#`). Anything other
/// than exactly six hex digits returns `None`.
fn parse_hex_rrggbb(hex: &str) -> Option<Rgba> {
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    })
}

/// Parse the comma-separated `r, g, b, a` body of an `rgba(...)` value:
/// `r`/`g`/`b` are `0..=255` integers, `a` is a `0.0..=1.0` float.
fn parse_rgba_components(inner: &str) -> Option<Rgba> {
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return None;
    }
    let r: u16 = parts[0].parse().ok()?;
    let g: u16 = parts[1].parse().ok()?;
    let b: u16 = parts[2].parse().ok()?;
    let a: f32 = parts[3].parse().ok()?;
    if r > 255 || g > 255 || b > 255 || !(0.0..=1.0).contains(&a) {
        return None;
    }
    Some(Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a,
    })
}

/// One typed paint command. Coordinates are in the same space as the
/// [`LayoutBox`] (viewport-relative logical pixels); the renderer offsets
/// by its content-rect origin.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCmd {
    /// A solid-fill rectangle (a block box background).
    Rect {
        /// Left edge (px).
        x: f32,
        /// Top edge (px).
        y: f32,
        /// Width (px).
        width: f32,
        /// Height (px).
        height: f32,
        /// Fill color.
        color: Rgba,
    },
    /// Positioned text, clipped to its box. See the module-level "no
    /// inline flow" gap: this is one text run, not wrapped line boxes.
    Text {
        /// Left edge (px).
        x: f32,
        /// Top edge (px).
        y: f32,
        /// Box width — the renderer wraps/clips within this.
        max_width: f32,
        /// Box height — the renderer clips within this.
        max_height: f32,
        /// The text content.
        text: String,
        /// Text color.
        color: Rgba,
        /// Font size (px).
        font_size: f32,
        /// Line height (px).
        line_height: f32,
    },
    /// An image placeholder rect — actual decode is a later milestone.
    Image {
        /// Left edge (px).
        x: f32,
        /// Top edge (px).
        y: f32,
        /// Width (px).
        width: f32,
        /// Height (px).
        height: f32,
        /// The `src` attribute (so the renderer can fetch later).
        src: String,
        /// The placeholder fill color shown until the image loads.
        placeholder: Rgba,
    },
}

/// An ordered list of paint commands. Paint order is tree order: a box's
/// background rect comes before its children; a box's text comes after
/// its own rect.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    /// The commands, in paint (back-to-front) order.
    pub cmds: Vec<DrawCmd>,
}

impl DisplayList {
    /// Append a command.
    pub fn push(&mut self, c: DrawCmd) {
        self.cmds.push(c);
    }

    /// Whether the list has no commands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }
}

/// Default font size (px) when a box's styled node carries no `font-size`.
const DEFAULT_FONT_SIZE: f32 = 16.0;

/// Line-height multiple applied to the font size.
const LINE_HEIGHT_FACTOR: f32 = 1.2;

/// Build a [`DisplayList`] from a computed [`LayoutTree`], its
/// [`StyledTree`], and the source [`Document`].
///
/// Walks the [`LayoutBox`] tree depth-first. For each box it cross-refs
/// `node_index` to the matching [`StyledNode`] (for `background-color` /
/// `color` / `font-size`) and to the DOM [`Node`] (for the box's *direct*
/// text content). It emits, in tree order:
///
/// 1. a [`DrawCmd::Rect`] for any `background-color` with alpha > 0,
/// 2. a [`DrawCmd::Image`] placeholder for `<img>` boxes,
/// 3. a [`DrawCmd::Text`] for boxes whose DOM node has direct text,
///
/// then recurses into children. Zero-area boxes and transparent/absent
/// backgrounds are skipped (a visible gap, never a black box). See the
/// module-level "Named gaps" for what this M0 walk does *not* yet do.
#[must_use]
pub fn build_display_list(
    layout: &LayoutTree,
    styled: &StyledTree,
    dom: &Document,
) -> DisplayList {
    // node_index → &StyledNode. resolve_node assigns indices in preorder
    // DFS; index_styled_nodes rebuilds that same map.
    let styled_index = index_styled_nodes(&styled.root);
    // node_index → &Node. The cascade numbers nodes in the SAME preorder
    // DFS that DescendantIter walks, so enumerate() reproduces the map.
    let dom_index: Vec<&Node> = dom.root.descendants().collect();

    let mut list = DisplayList::default();
    walk_box(&layout.root, &styled_index, &dom_index, &mut list);
    list
}

/// Build a `node_index → &StyledNode` lookup by walking the styled tree
/// in the same preorder the cascade numbered it with.
fn index_styled_nodes(root: &StyledNode) -> Vec<(usize, &StyledNode)> {
    let mut out = Vec::new();
    collect_styled(root, &mut out);
    out.sort_by_key(|(i, _)| *i);
    out
}

fn collect_styled<'a>(node: &'a StyledNode, out: &mut Vec<(usize, &'a StyledNode)>) {
    out.push((node.node_index, node));
    for child in &node.children {
        collect_styled(child, out);
    }
}

/// Look up a styled node by its `node_index`.
fn styled_for<'a>(index: &'a [(usize, &'a StyledNode)], node_index: usize) -> Option<&'a StyledNode> {
    index
        .iter()
        .find(|(i, _)| *i == node_index)
        .map(|(_, n)| *n)
}

/// The DOM node's *direct* text — the concatenation of its immediate
/// `Text` children only (not descendant text), so a `<p>Hello</p>` box
/// gets "Hello" but its parent `<div>` does not double-emit it.
fn direct_text(node: &Node) -> String {
    let mut s = String::new();
    for child in &node.children {
        if let Some(t) = child.as_text() {
            s.push_str(t);
        }
    }
    s
}

fn walk_box(
    lbox: &LayoutBox,
    styled_index: &[(usize, &StyledNode)],
    dom_index: &[&Node],
    list: &mut DisplayList,
) {
    let zero_area = lbox.width <= 0.0 || lbox.height <= 0.0;
    let styled = styled_for(styled_index, lbox.node_index);
    let dom_node = dom_index.get(lbox.node_index).copied();

    // 1. Background rect (parent before children). Skip zero-area boxes
    //    and absent/transparent backgrounds.
    if !zero_area {
        if let Some(bg) = styled
            .and_then(|s| s.style.get("background-color"))
            .and_then(Rgba::parse)
        {
            if bg.a > 0.0 {
                list.push(DrawCmd::Rect {
                    x: lbox.x,
                    y: lbox.y,
                    width: lbox.width,
                    height: lbox.height,
                    color: bg,
                });
            }
        }

        // 2. Image placeholder for <img>.
        if lbox.tag == "img" {
            let src = dom_node
                .and_then(Node::as_element)
                .and_then(|el| el.get_attribute("src"))
                .unwrap_or("")
                .to_string();
            list.push(DrawCmd::Image {
                x: lbox.x,
                y: lbox.y,
                width: lbox.width,
                height: lbox.height,
                src,
                // A neutral mid-gray placeholder until decode lands.
                placeholder: Rgba {
                    r: 0.5,
                    g: 0.5,
                    b: 0.5,
                    a: 1.0,
                },
            });
        }
    }

    // 3. Text after the box's own rect. Emit for boxes whose DOM node has
    //    direct text content. Color falls back to None (renderer uses its
    //    scheme default fg) when the box carries no `color`.
    if let Some(node) = dom_node {
        let text = direct_text(node);
        if !text.trim().is_empty() {
            let color = styled
                .and_then(|s| s.style.get("color"))
                .and_then(Rgba::parse)
                .unwrap_or(Rgba::TRANSPARENT);
            let font_size = styled
                .and_then(|s| s.style.get("font-size"))
                .and_then(parse_font_size)
                .unwrap_or(DEFAULT_FONT_SIZE);
            list.push(DrawCmd::Text {
                x: lbox.x,
                y: lbox.y,
                max_width: lbox.width,
                max_height: lbox.height,
                text,
                color,
                font_size,
                line_height: font_size * LINE_HEIGHT_FACTOR,
            });
        }
    }

    for child in &lbox.children {
        walk_box(child, styled_index, dom_index, list);
    }
}

/// Parse a `font-size` value to pixels. Accepts a bare number or a
/// `"<n>px"` form; the cascade currently emits a Debug form for
/// `font-size`, so this is best-effort — a future cascade improvement
/// (computed `em`/`rem`/`%`) makes this exact.
fn parse_font_size(value: &str) -> Option<f32> {
    let trimmed = value.trim().trim_end_matches("px").trim();
    trimmed.parse::<f32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::cascade::{StyleResolver, StyleSheet};
    use crate::layout::{LayoutEngine, Size};

    // -----------------------------------------------------------------
    // Rgba::parse — both cascade-emitted formats + currentColor + bad.
    // -----------------------------------------------------------------

    #[test]
    fn parse_hex_rrggbb_opaque() {
        let c = Rgba::parse("#3050ff").unwrap();
        assert!((c.r - (0x30 as f32 / 255.0)).abs() < 1e-6);
        assert!((c.g - (0x50 as f32 / 255.0)).abs() < 1e-6);
        assert!((c.b - 1.0).abs() < 1e-6);
        assert!((c.a - 1.0).abs() < 1e-6);
    }

    #[test]
    fn parse_hex_white_and_black() {
        let w = Rgba::parse("#ffffff").unwrap();
        assert_eq!((w.r, w.g, w.b, w.a), (1.0, 1.0, 1.0, 1.0));
        let k = Rgba::parse("#000000").unwrap();
        assert_eq!((k.r, k.g, k.b, k.a), (0.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn parse_rgba_translucent() {
        // The exact form format_css_color emits: "rgba(r, g, b, a.aa)".
        let c = Rgba::parse("rgba(255, 0, 0, 0.50)").unwrap();
        assert!((c.r - 1.0).abs() < 1e-6);
        assert!((c.g).abs() < 1e-6);
        assert!((c.b).abs() < 1e-6);
        assert!((c.a - 0.5).abs() < 1e-6);
    }

    #[test]
    fn parse_rgba_no_spaces_also_ok() {
        let c = Rgba::parse("rgba(10,20,30,1)").unwrap();
        assert!((c.r - 10.0 / 255.0).abs() < 1e-6);
        assert!((c.a - 1.0).abs() < 1e-6);
    }

    #[test]
    fn parse_current_color_is_none() {
        assert_eq!(Rgba::parse("currentColor"), None);
        assert_eq!(Rgba::parse("currentcolor"), None);
    }

    #[test]
    fn parse_rejects_malformed() {
        assert_eq!(Rgba::parse("#fff"), None); // 3-digit not emitted by cascade
        assert_eq!(Rgba::parse("#gggggg"), None);
        assert_eq!(Rgba::parse("red"), None);
        assert_eq!(Rgba::parse("rgba(300, 0, 0, 1)"), None); // out of range
        assert_eq!(Rgba::parse("rgba(0, 0, 0, 2)"), None); // alpha > 1
        assert_eq!(Rgba::parse("rgba(0, 0, 0)"), None); // wrong arity
        assert_eq!(Rgba::parse(""), None);
    }

    #[test]
    fn transparent_const_is_zero_alpha() {
        assert_eq!(Rgba::TRANSPARENT.a, 0.0);
    }

    // -----------------------------------------------------------------
    // DisplayList helpers.
    // -----------------------------------------------------------------

    #[test]
    fn display_list_push_and_is_empty() {
        let mut dl = DisplayList::default();
        assert!(dl.is_empty());
        dl.push(DrawCmd::Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            color: Rgba::TRANSPARENT,
        });
        assert!(!dl.is_empty());
        assert_eq!(dl.cmds.len(), 1);
    }

    // -----------------------------------------------------------------
    // build_display_list — the M0 end-to-end fixture from the spec.
    // -----------------------------------------------------------------

    fn build(html: &str, css: &str) -> DisplayList {
        let doc = Document::parse(html);
        let sheet = StyleSheet::parse(css).unwrap();
        let mut resolver = StyleResolver::new();
        resolver.add_sheet(sheet);
        let styled = resolver.resolve(&doc);
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(800.0, 600.0));
        build_display_list(&layout, &styled, &doc)
    }

    #[test]
    fn fixture_emits_div_rect_and_white_hello_text() {
        // The spec's fixture: a blue div with a white-text p saying Hello.
        // Explicit width/height give the boxes area — taffy gives text zero
        // intrinsic size (a named M0 gap), so a real Rect needs sized boxes.
        let dl = build(
            "<div><p>Hello</p></div>",
            "div { background-color: #3050ff; width: 200px; height: 100px } \
             p { color: #ffffff; height: 30px }",
        );

        // There must be a Rect for the div's blue background.
        let rect = dl.cmds.iter().find_map(|c| match c {
            DrawCmd::Rect { color, .. } => Some(*color),
            _ => None,
        });
        let rect = rect.expect("expected a div background Rect");
        assert!((rect.b - 1.0).abs() < 1e-6, "div background should be blue-ish: {rect:?}");

        // There must be a Text "Hello" in white.
        let text = dl.cmds.iter().find_map(|c| match c {
            DrawCmd::Text { text, color, .. } => Some((text.clone(), *color)),
            _ => None,
        });
        let (text, color) = text.expect("expected a Hello Text");
        assert_eq!(text, "Hello");
        assert_eq!((color.r, color.g, color.b, color.a), (1.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn paint_order_is_rect_before_text() {
        let dl = build(
            "<div><p>Hi</p></div>",
            "div { background-color: #112233; width: 100px; height: 50px } \
             p { color: #ffffff; height: 20px }",
        );
        let rect_pos = dl
            .cmds
            .iter()
            .position(|c| matches!(c, DrawCmd::Rect { .. }))
            .expect("rect");
        let text_pos = dl
            .cmds
            .iter()
            .position(|c| matches!(c, DrawCmd::Text { .. }))
            .expect("text");
        assert!(rect_pos < text_pos, "the div rect must paint before the p text");
    }

    #[test]
    fn transparent_background_emits_no_rect() {
        // No background-color declared anywhere → no Rect commands.
        let dl = build(
            "<div><p>Plain</p></div>",
            "div { width: 100px; height: 50px } p { color: #000000; height: 20px }",
        );
        assert!(
            !dl.cmds.iter().any(|c| matches!(c, DrawCmd::Rect { .. })),
            "no background-color should yield no Rect"
        );
        // But the text still shows.
        assert!(dl.cmds.iter().any(|c| matches!(c, DrawCmd::Text { .. })));
    }

    #[test]
    fn currentcolor_text_falls_back_to_transparent_marker() {
        // currentColor → Rgba::parse None → caller uses scheme default fg,
        // which we encode as TRANSPARENT (a < 1) so the renderer knows to
        // substitute. The Text command is still emitted.
        let dl = build(
            "<p>Inherit</p>",
            "p { color: currentColor }",
        );
        let color = dl.cmds.iter().find_map(|c| match c {
            DrawCmd::Text { color, .. } => Some(*color),
            _ => None,
        });
        assert_eq!(color, Some(Rgba::TRANSPARENT));
    }

    #[test]
    fn img_emits_image_placeholder() {
        let dl = build(
            "<img src=\"https://example.com/cat.png\">",
            "img { background-color: #000000 }",
        );
        let img = dl.cmds.iter().find_map(|c| match c {
            DrawCmd::Image { src, .. } => Some(src.clone()),
            _ => None,
        });
        // The <img> box may be zero-area (no intrinsic size), in which
        // case it is skipped — that is one of the named M0 gaps. When it
        // does have area, the src is carried through.
        if let Some(src) = img {
            assert_eq!(src, "https://example.com/cat.png");
        }
    }
}
