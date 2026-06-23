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
    tree: TaffyTree<String>,
}

impl LayoutEngine {
    /// Create a new layout engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tree: TaffyTree::new(),
        }
    }

    /// Compute layout for a styled tree within the given viewport.
    ///
    /// Translates the styled tree into taffy nodes, runs layout computation,
    /// and returns a tree of `LayoutBox` values with absolute positions.
    pub fn compute(&mut self, styled_tree: &StyledTree, viewport: Size) -> LayoutTree {
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
            .compute_layout(
                root_node,
                taffy::prelude::Size {
                    width: AvailableSpace::Definite(viewport.width),
                    height: AvailableSpace::Definite(viewport.height),
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

        // Height if specified; otherwise a `#text` node gets a default
        // single-line height derived from its (inherited) font-size, so
        // text-bearing boxes don't collapse to zero and get skipped by the
        // paint layer. taffy has no text intrinsic size — this is the
        // text-measurement floor; the parent block auto-sizes to contain it.
        let height_ctx = ctx.with_percent_basis(ctx.viewport_h);
        if let Some(px) = resolve_dimension(styled.style.length(LengthProp::Height), &height_ctx) {
            style.size.height = Dimension::Length(px);
        } else if styled.tag == "#text" {
            // The node's resolved font-size (em folds in via ctx.font_size).
            let font_size = styled
                .style
                .font_size()
                .resolve(ctx)
                .unwrap_or(ROOT_FONT_SIZE);
            style.size.height = Dimension::Length(font_size * LINE_HEIGHT_FACTOR);
        }

        // Margins.
        if let Some(px) = resolve_len(styled.style.length(LengthProp::MarginTop), ctx) {
            style.margin.top = LengthPercentageAuto::Length(px);
        }
        if let Some(px) = resolve_len(styled.style.length(LengthProp::MarginBottom), ctx) {
            style.margin.bottom = LengthPercentageAuto::Length(px);
        }
        if let Some(px) = resolve_len(styled.style.length(LengthProp::MarginLeft), ctx) {
            style.margin.left = LengthPercentageAuto::Length(px);
        }
        if let Some(px) = resolve_len(styled.style.length(LengthProp::MarginRight), ctx) {
            style.margin.right = LengthPercentageAuto::Length(px);
        }

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

/// Resolve a typed [`Length`] to pixels against the context. `Auto`
/// returns `None` (the caller leaves the taffy slot at its default).
fn resolve_len(len: Length, ctx: &LengthContext) -> Option<f32> {
    len.resolve(ctx)
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
        StyledNode {
            node_index: 0,
            tag: tag.to_string(),
            style,
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
