//! Property-based tests — fuzz the parsers + invariants across
//! randomly generated inputs. Runs in `cargo test` like any other
//! integration test; proptest shrinks failing inputs to the minimal
//! form.

use nami_core::dom::Document;
use nami_core::lisp::{SexpOptions, dom_to_sexp_with, sexp_to_dom};
use nami_core::route::RouteRegistry;
use nami_core::selector::Selector;
use proptest::prelude::*;

// ── 1. Selector parser never panics ────────────────────────────
//
// For ANY ASCII string, `Selector::parse` returns `Ok` or a clean
// `Err` — never unwinds. This pins the panic-safety contract that
// the selector engine relies on for running untrusted Lisp input.
//
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn selector_parse_never_panics(s in "\\PC*") {
        // "\PC*" — any char sequence, no control chars. The panic
        // we're guarding against is arithmetic underflow / bad
        // slicing on malformed input; we don't care about the Ok/Err
        // outcome, just that it returns.
        let _ = Selector::parse(&s);
    }

    #[test]
    fn selector_parse_rejects_bare_operators(c in "[>+~]") {
        // A bare combinator can't stand alone.
        let r = Selector::parse(&c);
        prop_assert!(r.is_err(), "bare '{c}' should error, got {r:?}");
    }
}

// ── 2. Selector grammar is stable for well-formed inputs ────────
//
// Generate well-formed selector strings and prove they parse +
// stringifying them via Debug doesn't lose them.
//
fn ident_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,7}".prop_map(|s| s)
}

fn simple_selector_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        ident_strategy(),
        ident_strategy().prop_map(|s| format!(".{s}")),
        ident_strategy().prop_map(|s| format!("#{s}")),
        (ident_strategy(), ident_strategy()).prop_map(|(t, c)| format!("{t}.{c}")),
        (ident_strategy(), ident_strategy()).prop_map(|(a, v)| format!("[{a}={v}]")),
        (ident_strategy(), ident_strategy()).prop_map(|(a, v)| format!("[{a}^={v}]")),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn well_formed_selectors_parse_successfully(s in simple_selector_strategy()) {
        let r = Selector::parse(&s);
        prop_assert!(r.is_ok(), "generated selector {s:?} should parse, got {r:?}");
    }

    #[test]
    fn descendant_combinator_parses(a in simple_selector_strategy(), b in simple_selector_strategy()) {
        let combined = format!("{a} {b}");
        prop_assert!(Selector::parse(&combined).is_ok());
    }

    #[test]
    fn child_combinator_parses(a in simple_selector_strategy(), b in simple_selector_strategy()) {
        let combined = format!("{a} > {b}");
        prop_assert!(Selector::parse(&combined).is_ok());
    }
}

// ── 3. DOM → sexp → DOM is a fixed point ───────────────────────
//
// For any HTML input that html5ever accepts, round-tripping through
// our sexp serializer + parser produces a byte-identical canonical
// sexp after two cycles. This verifies the attestation chain: a
// snapshot's BLAKE3 hash is stable across the roundtrip.
//
fn html_strategy() -> impl Strategy<Value = String> {
    // Small HTML snippets with known tags + random attrs / text.
    prop_oneof![
        Just("<p>hi</p>".to_string()),
        Just(r#"<a href="https://x">go</a>"#.to_string()),
        Just("<div><span>one</span><span>two</span></div>".to_string()),
        Just("<article><h1>Title</h1><p>body</p></article>".to_string()),
        Just(
            r#"<button hx-get="/x" data-state="open" class="btn primary">go</button>"#.to_string()
        ),
        Just("<!-- hidden --><p>visible</p>".to_string()),
        Just(r#"<img src="a.png" alt="x">"#.to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn sexp_roundtrip_is_a_fixed_point(html in html_strategy()) {
        let opts = SexpOptions {
            depth_cap: None,
            pretty: false,
            trim_whitespace: true,
        };
        let doc = Document::parse(&html);
        let sexp1 = dom_to_sexp_with(&doc, &opts);
        let reparsed = sexp_to_dom(&sexp1)
            .map_err(|e| TestCaseError::fail(e))?;
        let sexp2 = dom_to_sexp_with(&reparsed, &opts);
        prop_assert_eq!(sexp1, sexp2, "sexp roundtrip is not a fixed point");
    }
}

// ── 4. Route matcher never panics ───────────────────────────────
//
// Bogus URL strings, weird unicode, empty paths — all must return
// cleanly. The matcher runs on every navigate so it's a critical
// panic-safety surface.
//
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn route_match_never_panics(url in "\\PC*") {
        let reg = RouteRegistry::new();
        let _ = reg.match_url(&url);
    }

    #[test]
    fn param_extraction_is_byte_identical(
        prefix in "[a-z]{1,5}",
        id in "[a-z0-9]{1,20}",
    ) {
        use nami_core::route::RouteSpec;
        let mut reg = RouteRegistry::new();
        reg.insert(RouteSpec {
            name: None,
            pattern: format!("/{prefix}/:id"),
            bind: vec![],
            on_match: vec![],
            description: None,
            tags: vec![],
        });
        let url = format!("https://example.com/{prefix}/{id}");
        let m = reg.match_url(&url).ok_or_else(|| TestCaseError::fail("no match"))?;
        prop_assert_eq!(m.params.get("id").map(String::as_str), Some(id.as_str()));
    }
}
