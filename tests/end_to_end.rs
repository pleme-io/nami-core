//! End-to-end integration tests across the full substrate.
//!
//! These tests author a Lisp configuration that exercises 8+ DSLs in
//! a single plausible scenario, then drive it through the same
//! phase ordering the live browser uses:
//!
//!   1. parse HTML via nami-core's DOM
//!   2. detect frameworks + extract embedded state
//!   3. match URL against routes → bind params into state store
//!   4. run page-load effects → mutate state
//!   5. evaluate predicates + run agents → emit transform names
//!   6. apply the transforms + alias expansion
//!   7. compute derived values + verify against expected shape
//!
//! If the assertions below hold, the whole Lisp substrate composes
//! end-to-end with no glue outside what nami-core itself exposes.

#![cfg(feature = "eval")]

use nami_core::agent::{AgentRegistry, AgentSpec};
use nami_core::alias::{AliasRegistry, AliasSpec};
use nami_core::derived::{DerivedRegistry, DerivedSpec};
use nami_core::dom::Document;
use nami_core::effect::{EffectRegistry, EffectSpec};
use nami_core::framework;
use nami_core::plan::{PlanRegistry, PlanSpec};
use nami_core::predicate::{EvalContext, PredicateRegistry, PredicateSpec};
use nami_core::route::{RouteRegistry, RouteSpec};
use nami_core::state;
use nami_core::store::{StateSpec, StateStore};
use nami_core::transform::DomTransformSpec;
use serde_json::json;

const ARTICLE_HTML: &str = r##"
<html>
  <head>
    <title>Welcome</title>
    <script id="__NEXT_DATA__" type="application/json">{"page":"/blog/2026/04/welcome"}</script>
  </head>
  <body>
    <div id="__next">
      <header><nav class="navbar"></nav></header>
      <article>
        <h1>Welcome</h1>
        <p>A sufficiently long opening paragraph to meet the prose-heavy threshold.</p>
        <p>Another substantial paragraph describing the state of the art.</p>
        <p>One more to push us past the minimum.</p>
        <div class="ad">ad copy</div>
        <aside data-slot="card">sidebar card</aside>
      </article>
      <footer><p>© 2026</p></footer>
    </div>
  </body>
</html>
"##;

#[test]
fn full_substrate_composes_end_to_end() {
    // ── 1. Parse + detect + extract embedded state ────────────────
    let mut doc = Document::parse(ARTICLE_HTML);
    let detections = framework::detect(&doc);
    let page_state = state::extract(&doc);
    // Frameworks we expect to detect: Next.js (via __NEXT_DATA__) +
    // shadcn/radix (via data-slot) + bootstrap (via .navbar).
    let fw_names: Vec<_> = detections.iter().map(|d| d.framework.name()).collect();
    assert!(
        fw_names.iter().any(|n| *n == "next.js"),
        "expected Next.js: {fw_names:?}"
    );
    assert!(
        fw_names.iter().any(|n| *n == "shadcn/radix"),
        "expected shadcn: {fw_names:?}"
    );
    // At least one embedded JSON blob (__NEXT_DATA__).
    assert!(!page_state.is_empty(), "expected embedded state");

    // ── 2. Route the URL. Pattern "/blog/:year/:month/:slug" should
    //      extract {year, month, slug} from a canonical blog URL. ──
    let mut routes = RouteRegistry::new();
    routes.insert(RouteSpec {
        name: Some("blog-post".into()),
        pattern: "/blog/:year/:month/:slug".into(),
        bind: vec![
            ("post-year".into(), "year".into()),
            ("post-month".into(), "month".into()),
            ("post-slug".into(), "slug".into()),
        ],
        on_match: vec!["reader-mode".into()],
        description: None,
        tags: vec!["blog".into()],
    });

    let url = "https://site.test/blog/2026/04/welcome";
    let rm = routes.match_url(url).expect("route should match");
    assert_eq!(rm.route, "blog-post");
    assert_eq!(rm.params.get("year").map(String::as_str), Some("2026"));
    assert_eq!(rm.params.get("slug").map(String::as_str), Some("welcome"));

    // ── 3. Seed a state store + bind route params into it ─────────
    let store = StateStore::from_specs(&[
        StateSpec {
            name: "visit-count".into(),
            initial: json!(0),
            description: None,
            persistent: false,
        },
        StateSpec {
            name: "last-url".into(),
            initial: json!(""),
            description: None,
            persistent: false,
        },
    ]);
    for (cell, param) in &rm.bindings {
        if let Some(val) = rm.params.get(param) {
            store.set(cell, json!(val));
        }
    }
    assert_eq!(store.get("post-year"), Some(json!("2026")));
    assert_eq!(store.get("post-slug"), Some(json!("welcome")));

    // ── 4. Run effects: bump visit-count + stash last-url ─────────
    let mut effects = EffectRegistry::new();
    effects.insert(EffectSpec {
        name: "bump".into(),
        description: None,
        on: "page-load".into(),
        when: None,
        run: r#"(set-state "visit-count" (+ visit-count 1))"#.into(),
        tags: vec![],
    });
    let predicates_empty = PredicateRegistry::new();
    let cx = EvalContext {
        doc: &doc,
        detections: &detections,
        state: &page_state,
    };
    let (_, reports) = nami_core::effect::run_page_load(&store, &effects, &predicates_empty, &cx);
    assert!(reports.iter().all(|r| r.ok), "effects should run cleanly");
    assert_eq!(store.get("visit-count"), Some(json!(1)));

    // ── 5. Predicates + agents ────────────────────────────────────
    let mut predicates = PredicateRegistry::new();
    predicates.insert(PredicateSpec {
        name: "has-article".into(),
        description: None,
        selector: Some("article".into()),
        min: None,
        max: None,
        framework: None,
        state_kind: None,
        all: vec![],
        any: vec![],
        none: vec![],
    });
    predicates.insert(PredicateSpec {
        name: "prose-heavy".into(),
        description: None,
        selector: Some("p".into()),
        min: Some(3),
        max: None,
        framework: None,
        state_kind: None,
        all: vec![],
        any: vec![],
        none: vec![],
    });
    predicates.insert(PredicateSpec {
        name: "likely-article".into(),
        description: None,
        selector: None,
        min: None,
        max: None,
        framework: None,
        state_kind: None,
        all: vec!["has-article".into(), "prose-heavy".into()],
        any: vec![],
        none: vec![],
    });
    assert!(predicates.evaluate("likely-article", &cx).unwrap());

    // Plan: reader-mode = hide-ads (+ any others).
    let mut plans = PlanRegistry::new();
    plans.insert(PlanSpec {
        name: "reader-mode".into(),
        apply: vec!["hide-ads".into()],
        description: None,
        tags: vec![],
    });

    // Agent: when likely-article, apply reader-mode.
    let mut agents = AgentRegistry::new();
    agents.insert(AgentSpec {
        name: "auto-reader".into(),
        description: None,
        on: "page-load".into(),
        when: Some("likely-article".into()),
        apply: Some("reader-mode".into()),
        applies: vec![],
        tags: vec![],
    });

    let decisions = nami_core::agent::decide(&agents, "page-load", &predicates, &plans, &cx);
    assert_eq!(decisions.len(), 1);
    assert!(decisions[0].fired);
    assert_eq!(decisions[0].transforms, vec!["hide-ads"]);

    // ── 6. Alias resolution + transform application ───────────────
    //
    // "@ad" alias resolves to ".ad" by fallback; the shadcn detection
    // doesn't override it. Demonstrates the substitution path.
    let mut aliases = AliasRegistry::new();
    aliases.insert(AliasSpec {
        name: "@ad".into(),
        fallback: ".ad".into(),
        description: None,
        shadcn: None,
        mui: None,
        tailwind: None,
        bootstrap: None,
        react: None,
        nextjs: None,
        remix: None,
        gatsby: None,
        vue: None,
        nuxt: None,
        svelte: None,
        sveltekit: None,
        angular: None,
        astro: None,
        solid: None,
        htmx: None,
        alpine: None,
        wordpress: None,
        shopify: None,
    });
    let raw_transforms = vec![DomTransformSpec {
        name: "hide-ads".into(),
        selector: "@ad".into(),
        action: nami_core::transform::DomAction::Remove,
        arg: None,
        description: None,
    }];
    let expanded = aliases.expand_transforms(&raw_transforms, &detections);
    assert_eq!(expanded[0].selector, ".ad");

    // Apply transforms via the agent path (we pre-compiled the names).
    let reports = nami_core::agent::apply(&mut doc, &decisions, &expanded);
    assert_eq!(reports.len(), 1);
    // The ad div is gone.
    let ad_count = doc
        .root
        .descendants()
        .filter(|n| n.as_element().is_some_and(|e| e.has_class("ad")))
        .count();
    assert_eq!(ad_count, 0, "ad should be removed");

    // ── 7. Derived: compute a summary value off state ─────────────
    let mut derived = DerivedRegistry::new();
    derived.insert(DerivedSpec {
        name: "visit-label".into(),
        description: None,
        inputs: vec!["visit-count".into()],
        compute: r#"(string-append "visits: " (toString visit-count))"#.into(),
    });
    let v = derived.evaluate("visit-label", &store).unwrap();
    assert_eq!(v, json!("visits: 1"));
}

/// State persists across multiple simulated page loads.
#[test]
fn state_persists_across_multiple_navigations() {
    let store = StateStore::from_specs(&[StateSpec {
        name: "count".into(),
        initial: json!(0),
        description: None,
        persistent: false,
    }]);
    let mut effects = EffectRegistry::new();
    effects.insert(EffectSpec {
        name: "bump".into(),
        description: None,
        on: "page-load".into(),
        when: None,
        run: r#"(set-state "count" (+ count 1))"#.into(),
        tags: vec![],
    });
    let preds = PredicateRegistry::new();
    let doc = Document::parse("<html><body></body></html>");
    let cx = EvalContext {
        doc: &doc,
        detections: &[],
        state: &[],
    };

    for expected in 1..=5 {
        nami_core::effect::run_page_load(&store, &effects, &preds, &cx);
        assert_eq!(store.get("count"), Some(json!(expected)));
    }
}

/// A more-specific route beats a more-general one when registered first.
#[test]
fn route_precedence_first_wins() {
    let mut reg = RouteRegistry::new();
    reg.insert(RouteSpec {
        name: Some("me".into()),
        pattern: "/users/me".into(),
        bind: vec![],
        on_match: vec!["load-self".into()],
        description: None,
        tags: vec![],
    });
    reg.insert(RouteSpec {
        name: Some("other".into()),
        pattern: "/users/:id".into(),
        bind: vec![],
        on_match: vec!["load-other".into()],
        description: None,
        tags: vec![],
    });
    let m = reg.match_url("https://site.test/users/me").unwrap();
    assert_eq!(m.route, "me");
    let m = reg.match_url("https://site.test/users/42").unwrap();
    assert_eq!(m.route, "other");
}
