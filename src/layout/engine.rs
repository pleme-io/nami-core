//! Taffy-based layout computation.

use taffy::prelude::*;
use tracing::debug;

use crate::css::cascade::{LengthProp, StyledNode, StyledTree};
use crate::css::values::{Length, LengthContext};

/// The root element's font-size (px). Basis for `rem` units; CSS default
/// is 16px and the layout engine has no settings surface yet.
const ROOT_FONT_SIZE: f32 = 16.0;

/// Default single-line height factor applied to the font-size when a
/// `#text` node has no explicit height (taffy has no text intrinsic size).
const LINE_HEIGHT_FACTOR: f32 = 1.4;

/// Viewport dimensions.
#[derive(Debug, Clone, Copy)]
pub struct Size {
    /// Width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
}

impl Size {
    /// Create a new size.
    #[must_use]
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// The text-measurement side-effect seam — the mockable [`Environment`]-
/// style trait of the TYPED-SPEC + INTERPRETER TRIPLET applied to layout.
///
/// A `#text` node carries no intrinsic size in taffy; the layout engine
/// asks a `TextMeasure` how wide + tall its text shapes at a given
/// font-size within an available width, and taffy uses that to size the
/// text leaf (and auto-size the parent block to contain the wrapped
/// lines). Implementations: the deterministic [`MockTextMeasure`] +
/// [`SingleLineMeasure`] in this crate for tests / the width-agnostic
/// default; the real glyphon/cosmic-text `GlyphonTextMeasure` in namimado
/// (the production renderer) so measured height == drawn height.
///
/// Returns the box [`Size`]: `width` is the text run width (clamped to
/// `max_width`), `height` is `line_count * line_height`.
pub trait TextMeasure {
    /// Measure `text` at `font_size_px` within an available `max_width`
    /// (px). The returned [`Size`] is the box the text occupies — its
    /// `height` MUST equal `number-of-wrapped-lines * line_height` so a
    /// long paragraph in a narrow column reports a multi-line height (the
    /// fix for the single-line floor that let text overlap following
    /// content).
    fn measure(&self, text: &str, font_size_px: f32, max_width: f32) -> Size;
}

/// The built-in, width-agnostic measurer: every text run is one line tall
/// (`font_size * LINE_HEIGHT_FACTOR`), exactly the prior single-line floor.
/// [`LayoutEngine::compute`] delegates to [`LayoutEngine::compute_with_measure`]
/// with this so EXISTING behavior + all current callers/tests are preserved
/// with no measurer required. Width is left at the text-run's available
/// width (taffy's `known_dimensions` / available space decide it), so the
/// reported `width` here is `max_width` when finite, else `0.0`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SingleLineMeasure;

impl TextMeasure for SingleLineMeasure {
    fn measure(&self, _text: &str, font_size_px: f32, max_width: f32) -> Size {
        // One line, preserving the historic `font_size * 1.4` floor. Width
        // is the available width when known (finite), else 0 so the leaf
        // does not impose a width — identical to the prior behavior where
        // a #text node set only a height.
        let width = if max_width.is_finite() { max_width } else { 0.0 };
        Size::new(width, font_size_px * LINE_HEIGHT_FACTOR)
    }
}

/// A deterministic [`TextMeasure`] with fixed per-character + line-height
/// metrics — the test/matrix measurer. NO font system, NO shaping; the
/// math is exact so a matrix row can assert the wrapped line-count.
///
/// - `text_width = chars * char_width_em * font_size`
/// - `lines = max(1, ceil(text_width / max_width))`
/// - `height = lines * line_height_factor * font_size`
/// - `width  = min(text_width, max_width)`
#[derive(Debug, Clone, Copy)]
pub struct MockTextMeasure {
    /// Per-character advance, as a fraction of the font-size (em).
    pub char_width_em: f32,
    /// Line height as a multiple of the font-size.
    pub line_height_factor: f32,
}

impl MockTextMeasure {
    /// Construct with explicit metrics.
    #[must_use]
    pub fn new(char_width_em: f32, line_height_factor: f32) -> Self {
        Self {
            char_width_em,
            line_height_factor,
        }
    }
}

impl Default for MockTextMeasure {
    /// Fixed metrics the matrix relies on: each char is `0.5em` wide and the
    /// line height is `1.4 ×` the font-size (matching [`LINE_HEIGHT_FACTOR`]).
    fn default() -> Self {
        Self {
            char_width_em: 0.5,
            line_height_factor: LINE_HEIGHT_FACTOR,
        }
    }
}

impl TextMeasure for MockTextMeasure {
    fn measure(&self, text: &str, font_size_px: f32, max_width: f32) -> Size {
        let chars = text.chars().count() as f32;
        let text_width = chars * self.char_width_em * font_size_px;
        // Wrap only when an available width is known + positive; otherwise
        // the run is one line (a width-agnostic measurement).
        let lines = if max_width.is_finite() && max_width > 0.0 {
            (text_width / max_width).ceil().max(1.0)
        } else {
            1.0
        };
        let height = lines * self.line_height_factor * font_size_px;
        let width = if max_width.is_finite() && max_width > 0.0 {
            text_width.min(max_width)
        } else {
            text_width
        };
        Size::new(width, height)
    }
}

/// The per-node context a `#text` taffy LEAF carries so the measure
/// closure can shape it: the text to measure + its resolved font-size
/// (px). Every non-`#text` node carries the [`NodeContext::default`]
/// (empty text, 0 font-size) — the measure closure only runs for leaves,
/// and a non-text leaf with empty text measures to zero.
#[derive(Debug, Clone, Default)]
pub struct NodeContext {
    /// The text to measure (empty for non-text leaves).
    pub text: String,
    /// The node's resolved font-size in px.
    pub font_size: f32,
}

/// A computed layout box with position and dimensions.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    /// X position relative to the viewport.
    pub x: f32,
    /// Y position relative to the viewport.
    pub y: f32,
    /// Width of the box.
    pub width: f32,
    /// Height of the box.
    pub height: f32,
    /// The tag name or node type.
    pub tag: String,
    /// Index into the styled tree for cross-referencing.
    pub node_index: usize,
    /// Child layout boxes.
    pub children: Vec<LayoutBox>,
}

impl LayoutBox {
    /// Check if a point is inside this box.
    #[must_use]
    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }

    /// Find the deepest layout box containing a point.
    #[must_use]
    pub fn hit_test(&self, px: f32, py: f32) -> Option<&LayoutBox> {
        if !self.contains_point(px, py) {
            return None;
        }
        // Check children first (they're on top).
        for child in self.children.iter().rev() {
            if let Some(hit) = child.hit_test(px, py) {
                return Some(hit);
            }
        }
        Some(self)
    }
}

/// The computed layout tree.
#[derive(Debug, Clone)]
pub struct LayoutTree {
    /// Root layout box.
    pub root: LayoutBox,
    /// The viewport size used for this layout.
    pub viewport: Size,
}

/// Layout engine wrapping taffy.
pub struct LayoutEngine {
    tree: TaffyTree<NodeContext>,
}

impl LayoutEngine {
    /// Create a new layout engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tree: TaffyTree::new(),
        }
    }

    /// Compute layout for a styled tree within the given viewport, using
    /// the built-in [`SingleLineMeasure`] (the historic single-line floor).
    ///
    /// This is the behavior-preserving entry point: it delegates to
    /// [`Self::compute_with_measure`] with a width-agnostic measurer, so
    /// every existing caller / test keeps its exact prior output with NO
    /// measurer required. A consumer that wants real wrapped-text heights
    /// (namimado's glyphon measurer, the tests' [`MockTextMeasure`]) calls
    /// [`Self::compute_with_measure`] directly.
    pub fn compute(&mut self, styled_tree: &StyledTree, viewport: Size) -> LayoutTree {
        self.compute_with_measure(styled_tree, viewport, &SingleLineMeasure)
    }

    /// Compute layout for a styled tree within the given viewport, measuring
    /// text via the supplied [`TextMeasure`].
    ///
    /// A `#text` node WITH text becomes a taffy LEAF carrying a
    /// [`NodeContext`] (its text + resolved font-size); taffy invokes the
    /// measure closure with the leaf's `known_dimensions` + `available_space`
    /// and the closure calls `measure(text, font_size, available_width)`,
    /// returning the box [`Size`] (height = wrapped-line-count × line-height).
    /// The parent block then auto-sizes to contain the wrapped text — fixing
    /// the single-line floor that let a long paragraph overlap following
    /// content.
    pub fn compute_with_measure(
        &mut self,
        styled_tree: &StyledTree,
        viewport: Size,
        measure: &dyn TextMeasure,
    ) -> LayoutTree {
        self.tree = TaffyTree::new();

        // The base length-resolution context: the viewport drives vw/vh and
        // the initial percent basis; per-node font-size (for em) is folded
        // in as the recursion descends. rem is always the root font-size.
        let base_ctx = LengthContext::new(
            ROOT_FONT_SIZE,
            ROOT_FONT_SIZE,
            viewport.width,
            viewport.height,
        );
        let root_node = self.build_taffy_node(&styled_tree.root, &base_ctx);

        self.tree
            .compute_layout_with_measure(
                root_node,
                taffy::prelude::Size {
                    width: AvailableSpace::Definite(viewport.width),
                    height: AvailableSpace::Definite(viewport.height),
                },
                // The measure function: taffy calls this for every LEAF.
                // A leaf carrying a non-empty `NodeContext.text` is a text
                // node — measure it; everything else measures to zero (its
                // size comes from its style, not from text).
                |known_dimensions, available_space, _node_id, node_ctx, _style| {
                    measure_leaf(measure, known_dimensions, available_space, node_ctx)
                },
            )
            .ok();

        let root = self.extract_layout(root_node, &styled_tree.root, 0.0, 0.0);

        debug!(
            "computed layout: {} boxes, viewport {}x{}",
            count_boxes(&root),
            viewport.width,
            viewport.height
        );

        LayoutTree { root, viewport }
    }

    fn build_taffy_node(&mut self, styled: &StyledNode, parent_ctx: &LengthContext) -> NodeId {
        // Per-node length context: `em` resolves against THIS node's
        // computed font-size; rem + viewport units stay constant; the
        // percent basis is inherited (set per-axis below where it matters).
        let font_px = styled
            .style
            .font_size()
            .resolve(parent_ctx)
            .unwrap_or(ROOT_FONT_SIZE);
        let ctx = LengthContext {
            font_size: font_px,
            root_font_size: parent_ctx.root_font_size,
            viewport_w: parent_ctx.viewport_w,
            viewport_h: parent_ctx.viewport_h,
            percent_basis: parent_ctx.percent_basis,
        };

        let style = self.styled_to_taffy(styled, &ctx);

        // A `#text` node WITH text is a measured LEAF: it carries a
        // `NodeContext` (its text + resolved font-size) and NO children, so
        // taffy invokes the measure closure to size it (wrapped-line-count ×
        // line-height). This is the seam that replaces the old single-line
        // height floor — the text leaf's height is now the measured height.
        if styled.tag == "#text" {
            if let Some(text) = styled.text.as_deref() {
                if !text.trim().is_empty() {
                    let node_ctx = NodeContext {
                        text: text.to_owned(),
                        font_size: font_px,
                    };
                    return self
                        .tree
                        .new_leaf_with_context(style, node_ctx)
                        .expect("taffy text-leaf creation should not fail");
                }
            }
        }

        let child_nodes: Vec<NodeId> = styled
            .children
            .iter()
            .map(|child| self.build_taffy_node(child, &ctx))
            .collect();

        self.tree
            .new_with_children(style, &child_nodes)
            .expect("taffy node creation should not fail")
    }

    fn styled_to_taffy(&self, styled: &StyledNode, ctx: &LengthContext) -> Style {
        let mut style = Style::default();

        // Map the typed display mode. Inline has no true taffy mode (taffy
        // is flexbox/grid/block only), so it lays out as Block — preserving
        // the prior behavior where every non-none/flex value became Block.
        use crate::css::values::Display as CssDisplay;
        style.display = match styled.style.display() {
            CssDisplay::Flex => Display::Flex,
            CssDisplay::None => Display::None,
            // Block + Inline (and the inline-as-block fallback) → Block.
            CssDisplay::Block | CssDisplay::Inline => Display::Block,
        };

        // Width / height — resolved via the typed length context, so vw/vh/
        // em/rem/% now produce real pixels (Auto → no fixed dimension). For
        // width, % resolves against the inherited percent basis (viewport at
        // root); for height, % resolves against the viewport height.
        if let Some(px) = resolve_dimension(styled.style.length(LengthProp::Width), ctx) {
            style.size.width = Dimension::Length(px);
        }

        // Height if specified. An auto-height `#text` node is NOT given a
        // single-line floor here anymore — it becomes a measured taffy LEAF
        // (see `build_taffy_node`), so the measure closure sizes it to
        // `wrapped-line-count × line-height`. An *explicit* height still
        // wins: it lands as a known dimension the measure closure respects.
        let height_ctx = ctx.with_percent_basis(ctx.viewport_h);
        if let Some(px) = resolve_dimension(styled.style.length(LengthProp::Height), &height_ctx) {
            style.size.height = Dimension::Length(px);
        }

        // Margins. A side that resolves from `Length::Auto` becomes
        // `LengthPercentageAuto::Auto` — NOT left at the 0 default — so taffy
        // distributes free space to it. `margin-left:auto` +
        // `margin-right:auto` on a fixed-width block is what centers it.
        // Every concrete length resolves through the typed context as before.
        style.margin.top = taffy_margin(styled.style.length(LengthProp::MarginTop), ctx);
        style.margin.bottom = taffy_margin(styled.style.length(LengthProp::MarginBottom), ctx);
        style.margin.left = taffy_margin(styled.style.length(LengthProp::MarginLeft), ctx);
        style.margin.right = taffy_margin(styled.style.length(LengthProp::MarginRight), ctx);

        // Padding.
        if let Some(px) = resolve_len(styled.style.length(LengthProp::PaddingTop), ctx) {
            style.padding.top = LengthPercentage::Length(px);
        }
        if let Some(px) = resolve_len(styled.style.length(LengthProp::PaddingBottom), ctx) {
            style.padding.bottom = LengthPercentage::Length(px);
        }
        if let Some(px) = resolve_len(styled.style.length(LengthProp::PaddingLeft), ctx) {
            style.padding.left = LengthPercentage::Length(px);
        }
        if let Some(px) = resolve_len(styled.style.length(LengthProp::PaddingRight), ctx) {
            style.padding.right = LengthPercentage::Length(px);
        }

        style
    }

    fn extract_layout(
        &self,
        node_id: NodeId,
        styled: &StyledNode,
        parent_x: f32,
        parent_y: f32,
    ) -> LayoutBox {
        let layout = self.tree.layout(node_id).expect("node should have layout");
        let x = parent_x + layout.location.x;
        let y = parent_y + layout.location.y;

        let taffy_children: Vec<NodeId> = self.tree.children(node_id).unwrap_or_default();

        let children = taffy_children
            .iter()
            .zip(styled.children.iter())
            .map(|(&child_id, child_styled)| self.extract_layout(child_id, child_styled, x, y))
            .collect();

        LayoutBox {
            x,
            y,
            width: layout.size.width,
            height: layout.size.height,
            tag: styled.tag.clone(),
            node_index: styled.node_index,
            children,
        }
    }
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Taffy's per-leaf measure callback body: ask the [`TextMeasure`] to size
/// a text leaf, honoring taffy's `known_dimensions` (an explicit width/height
/// the style already fixed). A leaf with empty `NodeContext.text` (or no
/// context) measures to zero — its size comes from its style, not text.
///
/// `available_space.width` selects the wrap width: a `Definite(w)` is the
/// real available width; `MinContent`/`MaxContent` map to the longest /
/// unbounded line — for `MaxContent` we pass `f32::INFINITY` (one line),
/// matching CSS's "shrink-to-fit at max-content has no wrap".
fn measure_leaf(
    measure: &dyn TextMeasure,
    known_dimensions: taffy::geometry::Size<Option<f32>>,
    available_space: taffy::geometry::Size<AvailableSpace>,
    node_ctx: Option<&mut NodeContext>,
) -> taffy::geometry::Size<f32> {
    // Both dimensions already known → no measurement needed.
    if let (Some(w), Some(h)) = (known_dimensions.width, known_dimensions.height) {
        return taffy::geometry::Size { width: w, height: h };
    }
    let Some(ctx) = node_ctx else {
        // A non-text leaf (no context) measures to zero — its style sizes it.
        return taffy::geometry::Size {
            width: known_dimensions.width.unwrap_or(0.0),
            height: known_dimensions.height.unwrap_or(0.0),
        };
    };
    if ctx.text.trim().is_empty() {
        return taffy::geometry::Size {
            width: known_dimensions.width.unwrap_or(0.0),
            height: known_dimensions.height.unwrap_or(0.0),
        };
    }

    // The wrap width: a known width wins; else the available width.
    let max_width = known_dimensions.width.unwrap_or(match available_space.width {
        AvailableSpace::Definite(w) => w,
        AvailableSpace::MaxContent => f32::INFINITY,
        // MinContent has no meaningful "longest line" here without shaping;
        // treat as unbounded (one line) — the historic behavior for a run.
        AvailableSpace::MinContent => f32::INFINITY,
    });

    let measured = measure.measure(&ctx.text, ctx.font_size, max_width);
    taffy::geometry::Size {
        // A known width still wins over the measured run width.
        width: known_dimensions.width.unwrap_or(measured.width),
        height: known_dimensions.height.unwrap_or(measured.height),
    }
}

/// Resolve a typed [`Length`] to pixels against the context. `Auto`
/// returns `None` (the caller leaves the taffy slot at its default).
fn resolve_len(len: Length, ctx: &LengthContext) -> Option<f32> {
    len.resolve(ctx)
}

/// Map a typed margin [`Length`] to a taffy [`LengthPercentageAuto`].
///
/// `Length::Auto` → [`LengthPercentageAuto::Auto`] (taffy distributes free
/// space to the side — the centering mechanism); a concrete length resolves
/// to pixels; an unresolvable (only `Auto`, already handled) falls back to a
/// 0 length. This is the typed seam where `margin:0 auto` becomes real
/// centering: both left+right resolve to `Auto`, so taffy splits the free
/// space evenly.
fn taffy_margin(len: Length, ctx: &LengthContext) -> LengthPercentageAuto {
    match len {
        Length::Auto => LengthPercentageAuto::Auto,
        other => other
            .resolve(ctx)
            .map_or(LengthPercentageAuto::Length(0.0), LengthPercentageAuto::Length),
    }
}

/// Resolve a width/height [`Length`]. Identical to [`resolve_len`] but
/// named for the box-dimension call sites (`Auto` → no fixed dimension).
fn resolve_dimension(len: Length, ctx: &LengthContext) -> Option<f32> {
    len.resolve(ctx)
}

/// Count total layout boxes in a tree.
fn count_boxes(layout_box: &LayoutBox) -> usize {
    1 + layout_box
        .children
        .iter()
        .map(|c| count_boxes(c))
        .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::cascade::{ComputedStyle, StyledNode, StyledTree};

    fn make_styled_node(tag: &str, display: &str, children: Vec<StyledNode>) -> StyledNode {
        let mut style = ComputedStyle::default();
        style.set("display", display);
        // A `#text` node carries text so it becomes a measured leaf (a text
        // node with no text would measure to zero); every other tag is `None`.
        let text = if tag == "#text" {
            Some("text".to_string())
        } else {
            None
        };
        StyledNode {
            node_index: 0,
            tag: tag.to_string(),
            style,
            text,
            children,
        }
    }

    #[test]
    fn compute_simple_layout() {
        let styled = StyledTree {
            root: make_styled_node(
                "div",
                "block",
                vec![
                    make_styled_node("#text", "block", vec![]),
                    make_styled_node("p", "block", vec![]),
                ],
            ),
        };

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(800.0, 600.0));

        assert_eq!(layout.viewport.width, 800.0);
        assert_eq!(layout.viewport.height, 600.0);
        assert_eq!(layout.root.width, 800.0);
    }

    #[test]
    fn layout_with_fixed_width() {
        let mut style = ComputedStyle::default();
        style.set("display", "block");
        style.set("width", "200px");

        let styled = StyledTree {
            root: StyledNode {
                node_index: 0,
                tag: "div".to_string(),
                style,
                text: None,
                children: vec![],
            },
        };

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(800.0, 600.0));
        assert!((layout.root.width - 200.0).abs() < 0.01);
    }

    #[test]
    fn hit_test() {
        let layout_box = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
            tag: "div".to_string(),
            node_index: 0,
            children: vec![LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 100.0,
                height: 50.0,
                tag: "p".to_string(),
                node_index: 1,
                children: vec![],
            }],
        };

        // Hit the inner box.
        let hit = layout_box.hit_test(20.0, 20.0);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().tag, "p");

        // Hit the outer box but not the inner.
        let hit = layout_box.hit_test(500.0, 500.0);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().tag, "div");

        // Miss everything.
        let hit = layout_box.hit_test(900.0, 700.0);
        assert!(hit.is_none());
    }

    #[test]
    fn resolve_len_helper() {
        let ctx = LengthContext::new(20.0, 16.0, 1000.0, 500.0);
        assert_eq!(resolve_len(Length::Px(100.0), &ctx), Some(100.0));
        assert_eq!(resolve_len(Length::Em(2.0), &ctx), Some(40.0));
        assert_eq!(resolve_len(Length::Auto, &ctx), None);
    }

    /// Build a styled node by directly setting a typed CSS declaration.
    fn styled_with(tag: &str, prop: &str, value: &str) -> StyledTree {
        let mut style = ComputedStyle::default();
        style.set("display", "block");
        style.set(prop, value);
        StyledTree {
            root: StyledNode {
                node_index: 0,
                tag: tag.to_string(),
                style,
                text: None,
                children: vec![],
            },
        }
    }

    #[test]
    fn vw_width_resolves_to_half_viewport() {
        // width:50vw on a 1280-wide viewport ≈ 640px (the units page proof).
        let styled = styled_with("div", "width", "50vw");
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(1280.0, 800.0));
        assert!(
            (layout.root.width - 640.0).abs() < 0.5,
            "50vw of 1280 should be ~640, got {}",
            layout.root.width
        );
    }

    #[test]
    fn vh_height_resolves_to_fraction_of_viewport() {
        let styled = styled_with("div", "height", "25vh");
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(1280.0, 800.0));
        assert!(
            (layout.root.height - 200.0).abs() < 0.5,
            "25vh of 800 should be ~200, got {}",
            layout.root.height
        );
    }

    #[test]
    fn rem_width_resolves_against_root_font() {
        // 10rem × 16px root = 160px.
        let styled = styled_with("div", "width", "10rem");
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(800.0, 600.0));
        assert!(
            (layout.root.width - 160.0).abs() < 0.5,
            "10rem should be 160px, got {}",
            layout.root.width
        );
    }

    #[test]
    fn em_width_resolves_against_node_font_size() {
        // font-size:32px + width:5em → 160px (em uses THIS node's font-size).
        let mut style = ComputedStyle::default();
        style.set("display", "block");
        style.set("font-size", "32px");
        style.set("width", "5em");
        let styled = StyledTree {
            root: StyledNode {
                node_index: 0,
                tag: "div".to_string(),
                style,
                text: None,
                children: vec![],
            },
        };
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(800.0, 600.0));
        assert!(
            (layout.root.width - 160.0).abs() < 0.5,
            "5em at 32px font should be 160px, got {}",
            layout.root.width
        );
    }

    #[test]
    fn percent_width_resolves_against_viewport_basis() {
        // width:50% resolves against the percent basis (viewport at root).
        let styled = styled_with("div", "width", "50%");
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(1000.0, 800.0));
        assert!(
            (layout.root.width - 500.0).abs() < 0.5,
            "50% of 1000 should be 500, got {}",
            layout.root.width
        );
    }

    #[test]
    fn text_node_gets_default_height() {
        // An auto-sized #text node must not collapse to zero (else the paint
        // layer skips it). It gets a default single-line height, and the
        // parent block auto-sizes to contain it.
        let styled = StyledTree {
            root: make_styled_node(
                "p",
                "block",
                vec![make_styled_node("#text", "inline", vec![])],
            ),
        };
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(800.0, 600.0));
        assert!(
            layout.root.children[0].height > 0.0,
            "auto-sized #text node should have non-zero height"
        );
        assert!(
            layout.root.height > 0.0,
            "parent block should auto-size to contain its text"
        );
    }

    #[test]
    fn display_none_produces_zero_size() {
        let styled = StyledTree {
            root: make_styled_node(
                "div",
                "block",
                vec![make_styled_node("span", "none", vec![])],
            ),
        };

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(800.0, 600.0));
        // The hidden child should have zero dimensions.
        assert_eq!(layout.root.children[0].width, 0.0);
        assert_eq!(layout.root.children[0].height, 0.0);
    }

    // ── Text measurement + wrapping (the TextMeasure seam) ────────────

    #[test]
    fn single_line_measure_is_width_agnostic_one_line() {
        // The built-in SingleLineMeasure: one line, height = font*1.4.
        let m = SingleLineMeasure;
        let sz = m.measure("anything long or short", 20.0, 100.0);
        assert!((sz.height - 20.0 * LINE_HEIGHT_FACTOR).abs() < 1e-3);
    }

    #[test]
    fn mock_measure_short_word_is_one_line() {
        // A short word fits in the available width → exactly one line.
        // "hi" = 2 chars × 0.5em × 16px = 16px < 200px max_width.
        let m = MockTextMeasure::default();
        let sz = m.measure("hi", 16.0, 200.0);
        assert!(
            (sz.height - 1.0 * m.line_height_factor * 16.0).abs() < 1e-3,
            "short word should be one line tall, got {}",
            sz.height
        );
    }

    #[test]
    fn mock_measure_long_text_wraps_to_n_lines() {
        // 40 chars × 0.5em × 16px = 320px of text at a 100px max_width →
        // ceil(320/100) = 4 lines. height = 4 × 1.4 × 16 = 89.6px.
        let m = MockTextMeasure::default();
        let text: String = std::iter::repeat('x').take(40).collect();
        let sz = m.measure(&text, 16.0, 100.0);
        let expected_lines = 4.0_f32;
        assert!(
            (sz.height - expected_lines * m.line_height_factor * 16.0).abs() < 1e-3,
            "40 chars at 100px should wrap to 4 lines, got height {}",
            sz.height
        );
        // Width is clamped to the max_width when the run overflows.
        assert!((sz.width - 100.0).abs() < 1e-3);
    }

    #[test]
    fn compute_with_measure_auto_sizes_parent_to_wrapped_height() {
        // A <p> with a long #text child, in a NARROW fixed-width column, must
        // auto-size to the WRAPPED multi-line height (not a single-line floor).
        // The text node carries the string; MockTextMeasure gives exact math.
        let mut style = ComputedStyle::default();
        style.set("display", "block");
        style.set("width", "100px");
        let text: String = std::iter::repeat('x').take(40).collect();
        let mut text_style = ComputedStyle::default();
        text_style.set("display", "inline");
        let styled = StyledTree {
            root: StyledNode {
                node_index: 0,
                tag: "p".to_string(),
                style,
                text: None,
                children: vec![StyledNode {
                    node_index: 1,
                    tag: "#text".to_string(),
                    style: text_style,
                    text: Some(text),
                    children: vec![],
                }],
            },
        };
        let measure = MockTextMeasure::default();
        let mut engine = LayoutEngine::new();
        let layout = engine.compute_with_measure(&styled, Size::new(800.0, 600.0), &measure);
        // 40 chars × 0.5em × 16px (default font) = 320px text; at the p's
        // 100px width → 4 lines × 1.4 × 16 = 89.6px.
        let expected = 4.0 * measure.line_height_factor * 16.0;
        let text_box = &layout.root.children[0];
        assert!(
            (text_box.height - expected).abs() < 0.5,
            "wrapped #text box should be {expected}px tall, got {}",
            text_box.height
        );
        // The parent block auto-sizes to contain the wrapped text.
        assert!(
            layout.root.height >= expected - 0.5,
            "parent block should contain the {expected}px wrapped text, got {}",
            layout.root.height
        );
    }

    #[test]
    fn compute_default_keeps_single_line_floor() {
        // `compute()` (no measurer) preserves the historic single-line floor:
        // a long #text in a narrow column is still ONE line tall.
        let mut style = ComputedStyle::default();
        style.set("display", "block");
        style.set("width", "100px");
        let text: String = std::iter::repeat('x').take(40).collect();
        let styled = StyledTree {
            root: StyledNode {
                node_index: 0,
                tag: "p".to_string(),
                style,
                text: None,
                children: vec![StyledNode {
                    node_index: 1,
                    tag: "#text".to_string(),
                    style: ComputedStyle::default(),
                    text: Some(text),
                    children: vec![],
                }],
            },
        };
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(800.0, 600.0));
        let text_box = &layout.root.children[0];
        // One line: 16px font × 1.4 = 22.4px (SingleLineMeasure floor).
        assert!(
            (text_box.height - 16.0 * LINE_HEIGHT_FACTOR).abs() < 0.5,
            "compute() must keep the single-line floor, got {}",
            text_box.height
        );
    }
}

/// The same layout assertions, re-expressed through the ONE
/// `nami_core::testkit` vocabulary. Runs only under `--features testkit`;
/// the hand-rolled tests above keep the default-build coverage.
#[cfg(all(test, feature = "testkit"))]
mod testkit_migrated {
    use crate::testkit::Probe;

    #[test]
    fn layout_with_fixed_width() {
        Probe::html("<div style=\"width:200px;height:50px\"></div>")
            .layout("div")
            .width(200.0);
    }

    #[test]
    fn vw_width_resolves_to_half_viewport() {
        Probe::html("<div style=\"width:50vw;height:40px\"></div>")
            .viewport(1280.0, 800.0)
            .layout("div")
            .width(640.0);
    }

    #[test]
    fn vh_height_resolves_to_fraction_of_viewport() {
        Probe::html("<div style=\"width:40px;height:25vh\"></div>")
            .viewport(1280.0, 800.0)
            .layout("div")
            .height(200.0);
    }

    #[test]
    fn rem_width_resolves_against_root_font() {
        Probe::html("<div style=\"width:10rem;height:40px\"></div>")
            .layout("div")
            .width(160.0);
    }

    #[test]
    fn em_width_resolves_against_node_font_size() {
        Probe::html("<div style=\"font-size:32px;width:5em;height:40px\"></div>")
            .layout("div")
            .width(160.0);
    }

    #[test]
    fn percent_width_resolves_against_viewport_basis() {
        Probe::html("<div style=\"width:50%;height:40px\"></div>")
            .viewport(1000.0, 800.0)
            .layout("div")
            .width(500.0);
    }
}
