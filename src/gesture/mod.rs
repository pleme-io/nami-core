//! `(defgesture)` — mouse-gesture bindings.
//!
//! Absorbs Vivaldi, Opera, and UC Browser mouse gestures into the
//! substrate. A gesture is a compact stroke string — space-separated
//! cardinal directions + `click` tokens — that maps to a command name.
//! Shares the `(defcommand)` dispatch table, so one command can fire
//! from key, menu, MCP tool, HTTP, or gesture indistinguishably.
//!
//! Stroke syntax:
//!   `U` `D` `L` `R`           — up / down / left / right
//!   `UR`, `DL`, etc.          — diagonals
//!   `click` / `right-click`   — button events
//!   `wheel-up` / `wheel-down` — vertical scroll ticks
//!
//! ```lisp
//! (defgesture :stroke "L"         :command "back")
//! (defgesture :stroke "R"         :command "forward")
//! (defgesture :stroke "U L"       :command "new-tab")
//! (defgesture :stroke "D"         :command "reload")
//! (defgesture :stroke "D R"       :command "close-tab")
//! (defgesture :stroke "R L"       :command "undo-close-tab")
//! ```
//!
//! Canonicalization collapses whitespace and uppercases cardinal
//! tokens; `"l  r"` and `"L R"` collide in the registry.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Single gesture rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defgesture"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GestureSpec {
    /// Stroke string — cardinal tokens (U/D/L/R + diagonals) or
    /// `click`/`right-click`/`wheel-up`/`wheel-down`, space-separated.
    pub stroke: String,
    /// Target command name, looked up in the `(defcommand)` registry.
    pub command: String,
    /// Optional predicate-name gate (future: "only when tab has video").
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl GestureSpec {
    #[must_use]
    pub fn canonical_stroke(&self) -> String {
        canonicalize_stroke(&self.stroke)
    }
}

/// Registry keyed by canonicalized stroke.
#[derive(Debug, Clone, Default)]
pub struct GestureRegistry {
    specs: Vec<GestureSpec>,
}

impl GestureRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: GestureSpec) {
        let canon = spec.canonical_stroke();
        self.specs.retain(|s| canonicalize_stroke(&s.stroke) != canon);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = GestureSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    #[must_use]
    pub fn specs(&self) -> &[GestureSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, stroke: &str) -> Option<&GestureSpec> {
        let canon = canonicalize_stroke(stroke);
        self.specs
            .iter()
            .find(|s| canonicalize_stroke(&s.stroke) == canon)
    }

    /// Canonical stroke strings currently bound. Useful for the
    /// inspector + conflict detection.
    #[must_use]
    pub fn strokes(&self) -> Vec<String> {
        self.specs
            .iter()
            .map(|s| canonicalize_stroke(&s.stroke))
            .collect()
    }
}

/// Canonicalize a stroke — collapse whitespace, uppercase cardinal
/// tokens, preserve casing for button/wheel tokens (they stay
/// lowercase).
#[must_use]
pub fn canonicalize_stroke(input: &str) -> String {
    input
        .split_whitespace()
        .map(canonicalize_token)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn canonicalize_token(t: &str) -> String {
    let lower = t.to_ascii_lowercase();
    match lower.as_str() {
        "u" | "d" | "l" | "r" | "ul" | "ur" | "dl" | "dr" | "lu" | "ru" | "ld" | "rd" => {
            lower.to_ascii_uppercase()
        }
        "click" | "right-click" | "middle-click" | "wheel-up" | "wheel-down" => lower,
        _ => lower,
    }
}

/// Incremental stroke tokenizer — call on each mouse-move sample
/// (dx, dy in pixels) and get back the cardinal token if the vector
/// exceeds a threshold, or None while still accumulating.
///
/// Reset between gestures by constructing a new [`StrokeBuilder`].
#[derive(Debug, Clone, Default)]
pub struct StrokeBuilder {
    dx: f32,
    dy: f32,
    /// Pixel threshold before a direction registers — prevents
    /// jitter from producing spurious tokens.
    threshold: f32,
    tokens: Vec<String>,
}

impl StrokeBuilder {
    #[must_use]
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold: threshold.max(1.0),
            ..Self::default()
        }
    }

    /// Accumulate a movement sample. Returns `Some(token)` exactly
    /// when a fresh direction token fires.
    pub fn sample(&mut self, dx: f32, dy: f32) -> Option<String> {
        self.dx += dx;
        self.dy += dy;
        let mag_sq = self.dx * self.dx + self.dy * self.dy;
        if mag_sq < self.threshold * self.threshold {
            return None;
        }
        let token = classify_direction(self.dx, self.dy);
        self.dx = 0.0;
        self.dy = 0.0;
        // Collapse consecutive duplicates — one long swipe shouldn't
        // emit "R R R".
        if self.tokens.last().is_some_and(|last| last == &token) {
            return None;
        }
        self.tokens.push(token.clone());
        Some(token)
    }

    /// Record a button event — appends verbatim after canonicalization.
    pub fn button(&mut self, token: &str) {
        let canon = canonicalize_token(token);
        if !canon.is_empty() {
            self.tokens.push(canon);
        }
    }

    /// Full canonical stroke accumulated so far.
    #[must_use]
    pub fn stroke(&self) -> String {
        self.tokens.join(" ")
    }
}

fn classify_direction(dx: f32, dy: f32) -> String {
    let adx = dx.abs();
    let ady = dy.abs();
    // 8-way: diagonal when both components are > 30% of max.
    let diag = adx.min(ady) > adx.max(ady) * 0.4;
    if diag {
        let h = if dx > 0.0 { 'R' } else { 'L' };
        let v = if dy > 0.0 { 'D' } else { 'U' };
        format!("{v}{h}")
    } else if adx > ady {
        (if dx > 0.0 { "R" } else { "L" }).to_owned()
    } else {
        (if dy > 0.0 { "D" } else { "U" }).to_owned()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<GestureSpec>, String> {
    tatara_lisp::compile_typed::<GestureSpec>(src)
        .map_err(|e| format!("failed to compile defgesture forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<GestureSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(stroke: &str, command: &str) -> GestureSpec {
        GestureSpec {
            stroke: stroke.into(),
            command: command.into(),
            when: None,
            description: None,
        }
    }

    #[test]
    fn canonicalize_uppercases_cardinals() {
        assert_eq!(canonicalize_stroke("l r"), "L R");
        assert_eq!(canonicalize_stroke("u  L"), "U L");
    }

    #[test]
    fn canonicalize_keeps_button_tokens_lowercase() {
        assert_eq!(canonicalize_stroke("click"), "click");
        assert_eq!(canonicalize_stroke("L RIGHT-CLICK"), "L right-click");
    }

    #[test]
    fn canonicalize_handles_diagonals() {
        assert_eq!(canonicalize_stroke("ur DL"), "UR DL");
    }

    #[test]
    fn registry_dedupes_by_canonical_stroke() {
        let mut reg = GestureRegistry::new();
        reg.insert(g("L", "back"));
        reg.insert(g("l", "back-alt")); // same canonical stroke
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].command, "back-alt");
    }

    #[test]
    fn resolve_ignores_input_case_and_whitespace() {
        let mut reg = GestureRegistry::new();
        reg.insert(g("U L", "new-tab"));
        assert_eq!(reg.resolve("u  l").unwrap().command, "new-tab");
        assert!(reg.resolve("D").is_none());
    }

    #[test]
    fn stroke_builder_emits_cardinal_tokens() {
        let mut b = StrokeBuilder::new(20.0);
        // Big right move → "R".
        assert_eq!(b.sample(30.0, 0.0).as_deref(), Some("R"));
        // Subthreshold → None.
        assert!(b.sample(5.0, 0.0).is_none());
        // Big down move → "D".
        assert_eq!(b.sample(0.0, 30.0).as_deref(), Some("D"));
        assert_eq!(b.stroke(), "R D");
    }

    #[test]
    fn stroke_builder_collapses_repeats() {
        let mut b = StrokeBuilder::new(20.0);
        b.sample(30.0, 0.0); // R
        let again = b.sample(30.0, 0.0); // should NOT re-emit
        assert!(again.is_none());
        assert_eq!(b.stroke(), "R");
    }

    #[test]
    fn stroke_builder_diagonal_classifier() {
        let mut b = StrokeBuilder::new(20.0);
        let t = b.sample(30.0, 30.0).unwrap();
        assert_eq!(t, "DR");
    }

    #[test]
    fn stroke_builder_button_appends() {
        let mut b = StrokeBuilder::new(20.0);
        b.sample(30.0, 0.0);
        b.button("right-click");
        assert_eq!(b.stroke(), "R right-click");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_gesture_form() {
        let src = r#"
            (defgesture :stroke "U L"
                        :command "new-tab"
                        :description "Swipe up-then-left to open a new tab")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].stroke, "U L");
        assert_eq!(specs[0].command, "new-tab");
    }
}
