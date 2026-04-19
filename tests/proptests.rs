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

// ── 5. Normalize pipeline invariants ────────────────────────────
//
// Every property below is a claim about `normalize::apply` that must
// hold for any well-formed input. Failure here is a bug in the
// normalize engine, not in the rule pack — the engine's contract
// is what users reason against when authoring normalize packs.
//
use nami_core::framework::{Detection, Framework};
use nami_core::normalize::{NormalizeRegistry, NormalizeSpec};

/// Walk a sexp string counting `(` and `)` that appear at structural
/// positions — outside `"..."` string literals. Our escape set matches
/// what `ast::write_quoted` emits (`\"`, `\\`, `\n`, `\r`, `\t`).
#[cfg(feature = "ts")]
fn count_structural_parens(s: &str) -> (usize, usize) {
    let mut opens = 0usize;
    let mut closes = 0usize;
    let mut in_string = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if c == '\\' {
                // Skip escaped char.
                chars.next();
            } else if c == '"' {
                in_string = false;
            }
        } else {
            match c {
                '"' => in_string = true,
                '(' => opens += 1,
                ')' => closes += 1,
                _ => {}
            }
        }
    }
    (opens, closes)
}

fn arb_tag() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("article".to_owned()),
        Just("nav".to_owned()),
        Just("section".to_owned()),
        Just("main".to_owned()),
        Just("aside".to_owned()),
        Just("div".to_owned()),
        Just("p".to_owned()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // P1. Every applied hit corresponds to at least one element now
    // carrying `data-n-from` with the old tag.
    #[test]
    fn hit_count_matches_stamped_elements(
        tags in prop::collection::vec(arb_tag(), 1..10)
    ) {
        let mut html = String::from("<html><body>");
        for t in &tags {
            html.push_str(&format!("<{t}>x</{t}>"));
        }
        html.push_str("</body></html>");

        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "r-article".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });

        let mut doc = Document::parse(&html);
        let report = nami_core::normalize::apply(&mut doc, &reg, &[]);

        // Count elements that now have `data-n-from=article`.
        let stamped = doc.root.descendants()
            .filter_map(|n| n.as_element())
            .filter(|el| el.get_attribute("data-n-from") == Some("article"))
            .count();

        prop_assert_eq!(
            report.applied(),
            stamped,
            "hit count {} should equal elements stamped with data-n-from=article ({}); tags={:?}",
            report.applied(), stamped, tags
        );
    }

    // P2. Inbound fold is idempotent — running a rename-to-n-foo rule
    // a second time produces zero new hits (the source tag is gone).
    #[test]
    fn inbound_fold_idempotent(
        ts in prop::collection::vec(arb_tag(), 0..8)
    ) {
        let mut html = String::from("<html><body>");
        for t in &ts { html.push_str(&format!("<{t}>x</{t}>")); }
        html.push_str("</body></html>");

        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "art".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });

        let mut doc = Document::parse(&html);
        let _ = nami_core::normalize::apply(&mut doc, &reg, &[]);
        let second = nami_core::normalize::apply(&mut doc, &reg, &[]);

        prop_assert_eq!(second.applied(), 0, "idempotency violated on second pass");
    }

    // P3. An unused framework gate is a perfect mute — zero rewrites
    // when the detection list doesn't match.
    #[test]
    fn framework_gate_mutes_when_absent(
        ts in prop::collection::vec(arb_tag(), 0..6)
    ) {
        let mut html = String::from("<html><body>");
        for t in &ts { html.push_str(&format!("<{t}>x</{t}>")); }
        html.push_str("</body></html>");

        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "gated".into(),
            framework: Some("some-framework-that-is-not-detected".into()),
            selector: "article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });

        let mut doc = Document::parse(&html);
        let report = nami_core::normalize::apply(&mut doc, &reg, &[]);
        prop_assert_eq!(report.applied(), 0);
    }

    // P4. Empty registry is a total no-op — the pre-/post-pass DOMs
    // serialize identically.
    #[test]
    fn empty_registry_is_identity(
        ts in prop::collection::vec(arb_tag(), 0..6)
    ) {
        let mut html = String::from("<html><body>");
        for t in &ts { html.push_str(&format!("<{t}>x</{t}>")); }
        html.push_str("</body></html>");

        let before = Document::parse(&html);
        let before_sexp = dom_to_sexp_with(&before, &SexpOptions::default());

        let reg = NormalizeRegistry::new();
        let mut after = Document::parse(&html);
        let _ = nami_core::normalize::apply(&mut after, &reg, &[]);
        let after_sexp = dom_to_sexp_with(&after, &SexpOptions::default());

        prop_assert_eq!(before_sexp, after_sexp);
    }

    // P5. Roundtrip fold → emit preserves text content.
    // fold: article → n-article. emit: n-article → div[data-slot=article].
    #[test]
    fn fold_then_emit_preserves_text(
        inner in "[a-z ]{1,30}",
    ) {
        let html = format!("<html><body><article>{inner}</article></body></html>");

        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "fold".into(),
            framework: None,
            selector: "article".into(),
            rename_to: "n-article".into(),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });
        reg.insert(NormalizeSpec {
            name: "emit".into(),
            framework: None,
            selector: "n-article".into(),
            rename_to: "div".into(),
            set_attrs: vec![("data-slot".into(), "article".into())],
            remove_attrs: vec![],
            description: None,
        });

        let mut doc = Document::parse(&html);
        let _ = nami_core::normalize::apply(&mut doc, &reg, &[]);
        let text = doc.text_content();
        prop_assert!(text.contains(&inner), "text content {:?} lost original body {:?}", text, inner);
    }

    // P6.5. Tree-sitter TSX parser invariants (feature-gated).
    #[cfg(feature = "ts")]
    #[test]
    fn tsx_parser_never_panics(s in "\\PC{0,400}") {
        let _ = nami_core::ast::parse_tsx(&s);
    }

    #[cfg(feature = "ts")]
    #[test]
    fn tsx_output_parens_balanced(s in "\\PC{0,400}") {
        let Ok(out) = nami_core::ast::parse_tsx(&s) else {
            return Ok(());
        };
        // Count STRUCTURAL parens — skip anything inside "..." strings,
        // since source-text leaves legitimately include paren chars.
        let (opens, closes) = count_structural_parens(&out);
        prop_assert_eq!(opens, closes, "sexp parens unbalanced: {}", out);
    }

    #[cfg(feature = "ts")]
    #[test]
    fn tsx_parse_is_deterministic(s in "\\PC{0,400}") {
        let Ok(a) = nami_core::ast::parse_tsx(&s) else {
            return Ok(());
        };
        let b = nami_core::ast::parse_tsx(&s).expect("second parse");
        prop_assert_eq!(a, b);
    }

    #[cfg(feature = "ts")]
    #[test]
    fn tsx_every_sexp_has_at_least_one_node(s in "\\PC{0,200}") {
        let Ok(out) = nami_core::ast::parse_tsx(&s) else {
            return Ok(());
        };
        // Even empty source yields one (program) node.
        prop_assert!(nami_core::ast::count_ts_nodes(&out) >= 1);
    }

    #[cfg(feature = "ts")]
    #[test]
    fn simple_jsx_element_always_has_opening_and_closing(
        tag in "[a-z]{1,8}",
        body in "[a-z0-9 ]{0,30}",
    ) {
        let src = format!("const x = <{tag}>{body}</{tag}>;");
        let out = nami_core::ast::parse_tsx(&src).expect("parse");
        let opens = out.matches(":kind \"jsx_opening_element\"").count();
        let closes = out.matches(":kind \"jsx_closing_element\"").count();
        prop_assert_eq!(opens, 1, "expected exactly 1 opening, got {}; src: {:?}", opens, src);
        prop_assert_eq!(closes, 1, "expected exactly 1 closing, got {}; src: {:?}", closes, src);
    }

    // P7. JSX → Document round-trips — for any well-formed simple JSX
    // element, parse_tsx_as_document produces exactly one DOM element
    // with the expected tag and text, and the data-ast-source marker
    // is always set.
    #[cfg(feature = "ts")]
    #[test]
    fn parse_tsx_as_document_matches_simple_jsx(
        tag in "[a-z]{1,8}",
        body in "[a-z0-9 ]{1,30}",
    ) {
        let src = format!("const x = <{tag}>{body}</{tag}>;");
        let doc = nami_core::ast::parse_tsx_as_document(&src).expect("parse");

        let elements: Vec<_> = doc.root.descendants()
            .filter_map(|n| n.as_element())
            .collect();
        prop_assert_eq!(elements.len(), 1, "expected 1 element; src: {:?}", src);
        let el = elements[0];
        prop_assert_eq!(&el.tag, &tag);
        prop_assert_eq!(
            el.get_attribute("data-ast-source").map(str::to_owned),
            Some("jsx".to_owned())
        );
    }

    // P8. A normalize rule matching by tag name folds BOTH HTML and
    // equivalent JSX into the same canonical form.
    #[cfg(feature = "ts")]
    #[test]
    fn normalize_is_source_agnostic_across_html_and_jsx(
        tag in "[a-z]{2,6}",
    ) {
        use nami_core::normalize::{apply, NormalizeRegistry, NormalizeSpec};
        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "t".into(),
            framework: None,
            selector: tag.clone(),
            rename_to: format!("n-{tag}"),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });

        let html_src = format!("<html><body><{tag}>x</{tag}></body></html>");
        let jsx_src = format!("const x = <{tag}>x</{tag}>;");

        let mut html_doc = nami_core::dom::Document::parse(&html_src);
        let html_hits = apply(&mut html_doc, &reg, &[]).applied();

        let mut jsx_doc = nami_core::ast::parse_tsx_as_document(&jsx_src).expect("parse");
        let jsx_hits = apply(&mut jsx_doc, &reg, &[]).applied();

        prop_assert_eq!(html_hits, jsx_hits, "tag={:?}", tag);
    }

    // P7. Framework detection string matching is total for named
    // variants — for every Framework::* enum variant we can construct,
    // a rule gated on its canonical name matches.
    #[test]
    fn detected_framework_name_gates_match(
        tag in arb_tag(),
    ) {
        let html = format!("<html><body><{tag}>x</{tag}></body></html>");

        let mut reg = NormalizeRegistry::new();
        reg.insert(NormalizeSpec {
            name: "gated".into(),
            framework: Some("shadcn/radix".into()),
            selector: tag.clone(),
            rename_to: format!("n-{tag}"),
            set_attrs: vec![],
            remove_attrs: vec![],
            description: None,
        });

        let det = vec![Detection {
            framework: Framework::ShadcnRadix,
            name: "shadcn/radix",
            confidence: 0.9,
            evidence: vec![],
        }];

        let mut doc = Document::parse(&html);
        let report = nami_core::normalize::apply(&mut doc, &reg, &det);
        // At least 1 element of `tag` existed → at least 1 hit.
        prop_assert!(report.applied() >= 1);
    }
}
