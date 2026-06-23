//! Rendering-pipeline property tests — "no input crashes the renderer".
//!
//! Invariants over ALL inputs for the CSS → layout → paint pipeline:
//!
//! - [`Color::parse`] never panics (returns `None` or an in-range color).
//! - [`Length::parse`] + [`Length::resolve`] never panic; `resolve` is
//!   finite-or-`None`.
//! - [`CompoundSelector::parse`] + `matches` are total (never panic).
//! - THE FULL PIPELINE (`Document::parse` → cascade → layout →
//!   `build_display_list`) never panics on arbitrary HTML + CSS — the
//!   "no input crashes the renderer" guarantee (UNREPRESENTABILITY: a
//!   crash on hostile input is a representable bad state we forbid).
//!
//! Case counts are bounded so the suite stays fast.

use nami_core::css::cascade::{StyleResolver, StyleSheet};
use nami_core::css::selector::CompoundSelector;
use nami_core::css::values::{Color, Length, LengthContext};
use nami_core::dom::{Document, ElementData};
use nami_core::layout::{LayoutEngine, Size};
use nami_core::paint::build_display_list;
use proptest::prelude::*;

// ── 1. Color::parse total + in-range ────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `Color::parse` never panics on ANY non-control string, and every
    /// `Some` it returns has all four channels in `0.0..=1.0`.
    #[test]
    fn color_parse_never_panics_and_is_in_range(s in "\\PC{0,40}") {
        if let Some(c) = Color::parse(&s) {
            for ch in [c.r, c.g, c.b, c.a] {
                prop_assert!(
                    (0.0..=1.0).contains(&ch),
                    "Color::parse({s:?}) yielded out-of-range channel {ch}"
                );
            }
        }
    }

    /// `Color::parse` is deterministic — same input, same output.
    #[test]
    fn color_parse_is_deterministic(s in "\\PC{0,40}") {
        prop_assert_eq!(Color::parse(&s), Color::parse(&s));
    }
}

// ── 2. Length::parse + resolve total + finite ───────────────────────

fn arb_length_context() -> impl Strategy<Value = LengthContext> {
    (1.0f32..200.0, 1.0f32..200.0, 1.0f32..4000.0, 1.0f32..4000.0, 0.0f32..4000.0).prop_map(
        |(font, root, vw, vh, basis)| LengthContext {
            font_size: font,
            root_font_size: root,
            viewport_w: vw,
            viewport_h: vh,
            percent_basis: basis,
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `Length::parse` never panics; a `Some` resolved against any finite
    /// context is either `None` (Auto) or a finite f32 — never NaN/Inf.
    #[test]
    fn length_parse_and_resolve_are_total_and_finite(
        s in "\\PC{0,24}",
        ctx in arb_length_context(),
    ) {
        if let Some(len) = Length::parse(&s) {
            if let Some(px) = len.resolve(&ctx) {
                prop_assert!(px.is_finite(), "{s:?} → {len:?} resolved to non-finite {px}");
            }
        }
    }

    /// A parsed-then-resolved well-formed pixel length is the identity.
    #[test]
    fn px_length_resolves_to_itself(n in -10000.0f32..10000.0, ctx in arb_length_context()) {
        let s = format!("{n}px");
        if let Some(len) = Length::parse(&s) {
            prop_assert_eq!(len.resolve(&ctx), Some(n));
        }
    }
}

// ── 3. CompoundSelector::parse + matches total ──────────────────────

fn arb_element() -> impl Strategy<Value = ElementData> {
    (
        "[a-z]{1,8}",
        prop::option::of("[a-z]{1,8}"),
        prop::option::of("[a-z ]{0,20}"),
    )
        .prop_map(|(tag, id, class)| {
            let mut attrs = Vec::new();
            if let Some(id) = id {
                attrs.push(("id".to_string(), id));
            }
            if let Some(class) = class {
                attrs.push(("class".to_string(), class));
            }
            ElementData::with_attributes(tag, attrs)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `CompoundSelector::parse` never panics on ANY string.
    #[test]
    fn compound_parse_never_panics(s in "\\PC{0,40}") {
        let _ = CompoundSelector::parse(&s);
    }

    /// `matches` is a total predicate: parse any selector, match any
    /// element — never panics.
    #[test]
    fn compound_matches_is_total(sel in "\\PC{0,40}", el in arb_element()) {
        let s = CompoundSelector::parse(&sel);
        let _ = s.matches(&el);
    }

    /// The universal selector matches every element.
    #[test]
    fn universal_matches_all(el in arb_element()) {
        let s = CompoundSelector::parse("*");
        prop_assert!(s.matches(&el));
    }
}

// ── 4. THE FULL PIPELINE never panics ───────────────────────────────

/// Arbitrary HTML — known snippets plus random noise. The renderer must
/// survive both.
fn arb_html() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("<div><p>Hello</p></div>".to_string()),
        Just("<html><body><h1>x</h1><p>y</p></body></html>".to_string()),
        Just("<div class=\"a b\"><span id=\"z\">t</span></div>".to_string()),
        Just("<img src=\"x.png\"><a href=\"/\">l</a>".to_string()),
        Just("<table><tr><td>c</td></tr></table>".to_string()),
        Just("<!-- c --><section><aside>s</aside></section>".to_string()),
        Just(String::new()),
        "\\PC{0,80}",
    ]
}

/// Arbitrary CSS — known rules plus random noise.
fn arb_css() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("div{color:red;width:50px;height:50px}".to_string()),
        Just(".a{background:#3050ff} #z{color:#fff}".to_string()),
        Just("*{display:block} p{font-size:2em}".to_string()),
        Just("div{width:50vw;height:25vh;margin:10px;padding:5px}".to_string()),
        Just("span{display:none} a{color:currentColor}".to_string()),
        Just(String::new()),
        "\\PC{0,80}",
    ]
}

proptest! {
    // Bounded — the full pipeline (parse + cascade + taffy layout +
    // display list) is the heaviest per-case work; keep it tight.
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// No HTML+CSS input crashes the renderer. Document::parse →
    /// StyleResolver::resolve → LayoutEngine::compute →
    /// build_display_list runs to completion for every input.
    #[test]
    fn full_pipeline_never_panics(
        html in arb_html(),
        css in arb_css(),
        w in 1.0f32..4000.0,
        h in 1.0f32..4000.0,
    ) {
        let doc = Document::parse(&html);
        let mut resolver = StyleResolver::new();
        if let Ok(sheet) = StyleSheet::parse(&css) {
            resolver.add_sheet(sheet);
        }
        let styled = resolver.resolve(&doc);
        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&styled, Size::new(w, h));
        let _ = build_display_list(&layout, &styled, &doc);
    }

    /// The pipeline is deterministic — same inputs yield the same
    /// display list (a load-bearing property for golden tests).
    #[test]
    fn full_pipeline_is_deterministic(
        html in arb_html(),
        css in arb_css(),
    ) {
        let run = |html: &str, css: &str| {
            let doc = Document::parse(html);
            let mut resolver = StyleResolver::new();
            if let Ok(sheet) = StyleSheet::parse(css) {
                resolver.add_sheet(sheet);
            }
            let styled = resolver.resolve(&doc);
            let mut engine = LayoutEngine::new();
            let layout = engine.compute(&styled, Size::new(800.0, 600.0));
            build_display_list(&layout, &styled, &doc)
        };
        prop_assert_eq!(run(&html, &css), run(&html, &css));
    }
}
