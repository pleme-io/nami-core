//! Taffy-based layout computation.

use taffy::prelude::*;
use tracing::debug;

use crate::css::cascade::{StyledNode, StyledTree};

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

        let root_node = self.build_taffy_node(&styled_tree.root);

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

    fn build_taffy_node(&mut self, styled: &StyledNode) -> NodeId {
        let style = self.styled_to_taffy(styled);

        let child_nodes: Vec<NodeId> = styled
            .children
            .iter()
            .map(|child| self.build_taffy_node(child))
            .collect();

        self.tree
            .new_with_children(style, &child_nodes)
            .expect("taffy node creation should not fail")
    }

    fn styled_to_taffy(&self, styled: &StyledNode) -> Style {
        let mut style = Style::default();

        // Map display property.
        match styled.style.display() {
            "block" => {
                style.display = Display::Block;
            }
            "flex" => {
                style.display = Display::Flex;
            }
            "none" => {
                style.display = Display::None;
            }
            // "inline" and other values default to Block for layout purposes
            // since taffy doesn't have a true inline layout mode.
            _ => {
                style.display = Display::Block;
            }
        }

        // Parse width if specified.
        if let Some(w) = styled.style.get("width") {
            if let Some(px) = parse_px_value(w) {
                style.size.width = Dimension::Length(px);
            }
        }

        // Parse height if specified.
        if let Some(h) = styled.style.get("height") {
            if let Some(px) = parse_px_value(h) {
                style.size.height = Dimension::Length(px);
            }
        }

        // Parse margins.
        if let Some(m) = styled.style.get("margin-top") {
            if let Some(px) = parse_px_value(m) {
                style.margin.top = LengthPercentageAuto::Length(px);
            }
        }
        if let Some(m) = styled.style.get("margin-bottom") {
            if let Some(px) = parse_px_value(m) {
                style.margin.bottom = LengthPercentageAuto::Length(px);
            }
        }
        if let Some(m) = styled.style.get("margin-left") {
            if let Some(px) = parse_px_value(m) {
                style.margin.left = LengthPercentageAuto::Length(px);
            }
        }
        if let Some(m) = styled.style.get("margin-right") {
            if let Some(px) = parse_px_value(m) {
                style.margin.right = LengthPercentageAuto::Length(px);
            }
        }

        // Parse padding.
        if let Some(p) = styled.style.get("padding-top") {
            if let Some(px) = parse_px_value(p) {
                style.padding.top = LengthPercentage::Length(px);
            }
        }
        if let Some(p) = styled.style.get("padding-bottom") {
            if let Some(px) = parse_px_value(p) {
                style.padding.bottom = LengthPercentage::Length(px);
            }
        }
        if let Some(p) = styled.style.get("padding-left") {
            if let Some(px) = parse_px_value(p) {
                style.padding.left = LengthPercentage::Length(px);
            }
        }
        if let Some(p) = styled.style.get("padding-right") {
            if let Some(px) = parse_px_value(p) {
                style.padding.right = LengthPercentage::Length(px);
            }
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

/// Try to parse a pixel value from a string like `"100px"` or `"100"`.
fn parse_px_value(value: &str) -> Option<f32> {
    let trimmed = value.trim().trim_end_matches("px");
    trimmed.parse::<f32>().ok()
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
    fn parse_px_values() {
        assert_eq!(parse_px_value("100px"), Some(100.0));
        assert_eq!(parse_px_value("50"), Some(50.0));
        assert_eq!(parse_px_value(" 25px "), Some(25.0));
        assert_eq!(parse_px_value("auto"), None);
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
