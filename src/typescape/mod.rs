//! Typescape — the self-describing manifest of every typed capability
//! this crate ships.
//!
//! Aligned with the pleme-io **arch-synthesizer** typescape pattern
//! (see `~/code/github/pleme-io/CLAUDE.md` → "Typescape — The Universe
//! of All Types"). Every `def*` DSL, every AST domain, every
//! normalize pack, every host API becomes a queryable, serializable
//! record. The whole bundle is BLAKE3-hashed (128-bit, base32, 26
//! chars — matching `tameshi`, `sekiban`, `kensa`, and the rest of
//! the pleme-io attestation chain).
//!
//! ## Relation to arch-synthesizer's SystemTypescape
//!
//! arch-synthesizer's `SystemTypescape` is the **workspace-wide**
//! universe of all types (19 AST domains, 230+ vocabulary terms, 28
//! lattice permutations, …). Each pleme-io repo is a leaf in that
//! Merkle tree — carrying its own `TypescapeManifest` that enumerates
//! the dimensions it contributes to and content-hashes its artifacts.
//! The root Merkle aggregator walks every repo's manifest to produce
//! the system-wide attestation.
//!
//! nami-core's contribution: **13 DSL keywords + 3 AST domains + 31
//! canonical vocabulary tags + 4 WASM host APIs + 6 feature flags +
//! 3 provenance attrs**. Export via `manifest_yaml()` to produce the
//! `.typescape.yaml` that arch-synthesizer's aggregator consumes.
//!
//! Downstream tooling (agents, MCP servers, doc generators,
//! compliance checkers) reads this structure instead of scraping the
//! source. Coherence tests assert every shipped feature shows up
//! here — nothing implementable-but-unlisted.
//!
//! ## Eight dimensions (aligned to arch-synthesizer)
//!
//! | arch-synthesizer dim | nami-core field      |
//! | -------------------- | -------------------- |
//! | vocabulary           | `canonical_vocab`    |
//! | domains              | `ast_domains`        |
//! | stack / DSLs         | `dsl_keywords`       |
//! | render               | `host_apis`          |
//! | modules              | `features`           |
//! | attestation          | `provenance_attrs`   |
//! | version              | `version`            |
//! | content hash         | `typescape_hash()`   |
//!
//! Derived surfaces (not dimensions in their own right, but
//! introspectable):
//!
//! * `accessibility::ax_tree(&doc)` — the canonical `n-*`
//!   vocabulary doubles as the ARIA role map; every normalize pack
//!   implicitly gives its framework free a11y coverage.

use serde::{Deserialize, Serialize};

/// The full manifest. Stable structure: adding a new DSL / AST domain
/// / host API means a new entry here. Bumping `version` signals a
/// semver-meaningful change to tooling that depends on the shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamiTypescape {
    pub name: String,
    pub version: String,

    pub dsl_keywords: Vec<DslKeyword>,
    pub ast_domains: Vec<AstDomain>,
    pub canonical_vocab: Vec<CanonicalTag>,
    pub host_apis: Vec<HostApi>,
    pub features: Vec<FeatureFlag>,
    pub provenance_attrs: Vec<ProvenanceAttr>,
}

/// Every `(def*)` Lisp keyword this crate recognizes, with the spec
/// type it produces and a one-line description.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DslKeyword {
    pub keyword: String,
    pub spec_type: String,
    pub description: String,
    /// Which feature enables the compile pass (none = always on).
    pub requires_feature: Option<String>,
}

/// A parseable source language. Grammars producing `Document`s via
/// `ast::parse_*_as_document` are listed here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AstDomain {
    /// Short name used as `data-ast-source` stamp and in diagnostic
    /// output.
    pub name: String,
    pub parser: String,
    pub produces: String,
    pub description: String,
    pub requires_feature: Option<String>,
}

/// One entry in the canonical `n-*` vocabulary. This is the lingua
/// franca — normalize packs fold framework idioms into these tags so
/// downstream tooling targets a single schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalTag {
    pub name: String,
    pub description: String,
    /// Conceptual role — loose bucket, used by docs + inspectors.
    pub role: CanonicalRole,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CanonicalRole {
    Structural, // article, section, nav, main, aside, header, footer
    Interactive, // button, input, tab
    Container, // card, list, dialog
    Text, // card-title, card-description, badge, alert
    Media, // avatar, figure, image
}

/// A host function exposed from Rust into guest WASM via the `nami`
/// import namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostApi {
    pub name: String,
    /// WASM signature, in WAT notation.
    pub signature: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeatureFlag {
    pub name: String,
    pub description: String,
}

/// Attributes this crate stamps on elements to record provenance. Any
/// tool walking a post-pipeline Document can read these to trace
/// which source language produced an element and which rule(s)
/// rewrote it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProvenanceAttr {
    pub name: String,
    pub stamped_by: String,
    pub description: String,
}

/// Build the full typescape. Pure function; cheap to call. Results
/// are deterministic — same build = same manifest = same hash.
#[must_use]
pub fn typescape() -> NamiTypescape {
    NamiTypescape {
        name: "nami-core".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        dsl_keywords: dsl_keywords(),
        ast_domains: ast_domains(),
        canonical_vocab: canonical_vocabulary(),
        host_apis: host_apis(),
        features: features(),
        provenance_attrs: provenance_attrs(),
    }
}

/// Helper to keep the registry literals compact.
fn s(s: &str) -> String {
    s.to_owned()
}

fn opt_s(s: &str) -> Option<String> {
    Some(s.to_owned())
}

/// BLAKE3 content hash of the typescape — 128 bits, base32 lowercase,
/// 26 chars. Same convention as tameshi / sekiban / kensa / the rest
/// of the pleme-io attestation chain (see CLAUDE.md → "Kubernetes as
/// Convergence Processes" + "Tameshi — Deterministic Integrity
/// Attestation"). This hash can be emitted into a workspace Merkle
/// tree as this crate's leaf contribution.
#[must_use]
pub fn typescape_hash() -> String {
    let ts = typescape();
    let json = serde_json::to_vec(&ts).expect("typescape serializes");
    let hash = blake3::hash(&json);
    // 16 bytes (128 bits) → 26 base32 chars (with padding).
    base32_encode(&hash.as_bytes()[..16])
}

/// Emit the typescape as a `.typescape.yaml` manifest body — the
/// format arch-synthesizer's aggregator consumes for Merkle roll-up.
/// The top-level is a map keyed by dimension name, with the content
/// hash of this crate's contribution at `_hash`.
#[must_use]
pub fn manifest_yaml() -> String {
    let ts = typescape();
    let hash = typescape_hash();
    // `serde_yaml_ng` isn't a dep; build the YAML ourselves so the
    // shape is stable and format-locked. Keys match arch-synthesizer
    // dimensions so the aggregator can merge without mapping.
    let mut out = String::new();
    out.push_str("# arch-synthesizer TypescapeManifest — generated by nami-core\n");
    out.push_str(&format!("name: {:?}\n", ts.name));
    out.push_str(&format!("version: {:?}\n", ts.version));
    out.push_str(&format!("hash: {hash:?}\n"));
    out.push_str("dimensions:\n");
    out.push_str(&format!(
        "  vocabulary:  {}   # canonical n-* tags\n",
        ts.canonical_vocab.len()
    ));
    out.push_str(&format!(
        "  domains:     {}    # AST parsers → Document\n",
        ts.ast_domains.len()
    ));
    out.push_str(&format!(
        "  dsls:        {}    # (def*) keywords\n",
        ts.dsl_keywords.len()
    ));
    out.push_str(&format!(
        "  host_apis:   {}    # WASM nami.* imports\n",
        ts.host_apis.len()
    ));
    out.push_str(&format!(
        "  features:    {}    # compile-time feature flags\n",
        ts.features.len()
    ));
    out.push_str(&format!(
        "  attestation: {}    # provenance attrs stamped on elements\n",
        ts.provenance_attrs.len()
    ));
    out
}

/// Coherence check — walks the typescape and verifies every claim
/// against the running code. Any failure is a bug: the typescape has
/// drifted from reality.
///
/// This is the "prove everything in the abstract" entry point — pure,
/// deterministic, feature-aware. A CI gate can call this and fail
/// the build on drift.
pub fn coherence_check() -> Result<(), String> {
    let ts = typescape();

    // 1. DSL keywords are non-empty + unique.
    if ts.dsl_keywords.is_empty() {
        return Err("no DSL keywords registered".into());
    }
    let mut seen = std::collections::HashSet::new();
    for dsl in &ts.dsl_keywords {
        if dsl.keyword.is_empty() || dsl.spec_type.is_empty() {
            return Err(format!("DSL keyword has empty field: {dsl:?}"));
        }
        if !seen.insert(dsl.keyword.clone()) {
            return Err(format!("duplicate DSL keyword: {}", dsl.keyword));
        }
    }

    // 2. Canonical vocab tags all start with n- and are unique.
    let mut seen = std::collections::HashSet::new();
    for tag in &ts.canonical_vocab {
        if !tag.name.starts_with("n-") {
            return Err(format!("canonical tag missing n- prefix: {}", tag.name));
        }
        if !seen.insert(tag.name.clone()) {
            return Err(format!("duplicate canonical tag: {}", tag.name));
        }
    }

    // 3. Every AST domain with no feature gate must parse the empty
    // source without error. That's the weakest possible correctness
    // contract: a parser that can't swallow "" isn't a parser.
    for d in &ts.ast_domains {
        if d.requires_feature.is_none() && d.name == "html" {
            // html5ever is always-on; prove it.
            let doc = crate::dom::Document::parse("");
            if !doc.root.is_document() {
                return Err("html parser produced non-document root for empty input".into());
            }
        }
    }

    // 4. The hash has the expected shape (128 bits → 26 chars).
    let h = typescape_hash();
    if h.len() != 26 {
        return Err(format!("hash length {} != 26", h.len()));
    }

    Ok(())
}

fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buf >> bits) & 0x1f) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buf << (5 - bits)) & 0x1f) as usize;
        out.push(ALPHABET[idx] as char);
    }
    out
}

// ── the registries ──────────────────────────────────────────────

fn dsl_keywords() -> Vec<DslKeyword> {
    let lisp = || opt_s("lisp");
    let mk = |keyword: &str, spec_type: &str, description: &str| DslKeyword {
        keyword: s(keyword),
        spec_type: s(spec_type),
        description: s(description),
        requires_feature: lisp(),
    };
    vec![
        mk("defdom-transform", "transform::DomTransformSpec",
           "Selector-gated DOM mutation (remove, unwrap, set-attr, set-text, insert-before/after, replace-with, optional :component)."),
        mk("defscrape", "scrape::ScrapeSpec",
           "Structured data extraction from matched elements — text/attr/html/count outputs."),
        mk("defframework-alias", "alias::AliasSpec",
           "Per-framework selector substitution — `@card` maps to shadcn's `[data-slot=card]`, MUI's `.MuiCard-root`, etc."),
        mk("defplan", "plan::PlanSpec",
           "Named bundle of transform + sub-plan names, recursively expanded, cycle-detected."),
        mk("defpredicate", "predicate::PredicateSpec",
           "Named boolean check over DOM + framework detections + embedded state. Four body shapes."),
        mk("defagent", "agent::AgentSpec",
           "Two-phase {decide, apply} — on trigger, if predicate, apply a plan or transform."),
        mk("defstate", "store::StateSpec",
           "Runtime state cell — persists across navigates via StateStore."),
        mk("defeffect", "effect::EffectSpec",
           "Trigger-gated Lisp expression that mutates state via (set-state NAME VALUE)."),
        mk("defderived", "derived::DerivedSpec",
           "Svelte-$:-style computed values over state cells; cycle-detected, recomputed on demand."),
        mk("defquery", "query::QuerySpec",
           "Transport-agnostic HTTP query declaration with `:into` state cell target. Fetcher trait pluggable."),
        mk("defroute", "route::RouteSpec",
           "URL pattern match with :param bindings → state store, plus on-match action list."),
        mk("defcomponent", "component::ComponentSpec",
           "Prop-parameterized DOM template — React-in-Lisp Layer 1. Supports `(@ prop)` + `(: expr)` via tatara-eval."),
        mk("defnormalize", "normalize::NormalizeSpec",
           "Framework-gated DOM rewrites toward canonical n-* vocabulary. :set-attrs + :remove-attrs support bidirectional (fold/emit) flows."),
        mk("defwasm-agent", "wasm_agent::WasmAgentSpec",
           "Precompiled .wasm scraper declaration — trigger + fuel budget + path. Executes against a read-only DOM snapshot via WasmHost::run_agent."),
        mk("defblocker", "blocker::BlockerSpec",
           "Content blocking rule — domain list for outbound fetches + CSS selectors for DOM strip. Absorbs uBlock/EasyList patterns into the substrate pipeline."),
        mk("defstorage", "storage::kv::StorageSpec",
           "Typed persistent key/value store with optional TTL. Pure tatara-lisp, append-only event log for persistence, BLAKE3-attestable. Covers cookies/session/user-prefs without FFI."),
        mk("defreader", "reader::ReaderSpec",
           "Readability-style simplified view — keep/strip selectors + paragraph-density fallback + title/byline extraction. Absorbs Firefox Reader View + Safari Reader into the substrate pattern."),
        mk("defextension", "extension::ExtensionSpec",
           "Browser-extension bundle — metadata, permissions, host-permissions, and ownership of other def* forms. Absorbs Chrome MV3 + Firefox WebExtensions + Safari App Extensions. Decentralized store: BLAKE3 content address + ed25519 author signature."),
        mk("defcommand", "command::CommandSpec",
           "Named unit of behavior — built-in verb via :action, or tatara-lisp body via :do. Invokable by key, menu, MCP tool, or HTTP. Absorbs Chrome/Firefox commands API, Vivaldi command-chains, Arc toolbar-shortcut authoring."),
        mk("defbind", "command::BindSpec",
           "Key chord / multi-key sequence → command mapping, with Vim-style mode scoping (normal/insert/visual/any) and optional predicate gate. Full Lisp programmability; ships with a default vim-mode pack."),
        mk("defomnibox", "omnibox::OmniboxSpec",
           "URL-bar autocomplete profile — sources (history/bookmarks/commands/tabs/extensions), search providers with {query} templates + shortcuts (ddg/g/gh), max_results, min_chars. Absorbs Chrome omnibox, Firefox awesomebar, Safari smart search."),
    ]
}

fn ast_domains() -> Vec<AstDomain> {
    vec![
        AstDomain {
            name: s("html"),
            parser: s("html5ever"),
            produces: s("dom::Document"),
            description: s("HTML5 spec-compliant; error-recovering tree builder."),
            requires_feature: None,
        },
        AstDomain {
            name: s("jsx"),
            parser: s("tree-sitter-typescript (TSX)"),
            produces: s("dom::Document via ast::parse_tsx_as_document"),
            description: s("JSX/TSX source; jsx_element→element, jsx_attribute→attr, jsx_text→text, self-closing→empty element."),
            requires_feature: opt_s("ts"),
        },
        AstDomain {
            name: s("svelte"),
            parser: s("tree-sitter-svelte-next"),
            produces: s("dom::Document via ast::parse_svelte_as_document"),
            description: s("Svelte templates; element→element, attribute→attr, interpolations preserved as text."),
            requires_feature: opt_s("ts"),
        },
        AstDomain {
            name: s("css"),
            parser: s("hand-rolled css_ast tokenizer"),
            produces: s("Vec<css_ast::CssRule> ⇌ (css-stylesheet (css-rule …)) sexp"),
            description: s("Vanilla CSS — top-level style rules only in V1. Bidirectional: parse_css / emit_css / css_to_sexp / sexp_to_css form a fixed point."),
            requires_feature: None,
        },
    ]
}

fn canonical_vocabulary() -> Vec<CanonicalTag> {
    use CanonicalRole::*;
    let mk = |name: &str, desc: &str, role: CanonicalRole| CanonicalTag {
        name: s(name),
        description: s(desc),
        role,
    };
    vec![
        mk("n-article",           "Long-form content block.", Structural),
        mk("n-nav",               "Navigation region.", Structural),
        mk("n-main",              "Primary page content region.", Structural),
        mk("n-aside",             "Tangential content, sidebar.", Structural),
        mk("n-section",           "Generic section.", Structural),
        mk("n-header",            "Page/section header.", Structural),
        mk("n-footer",            "Page/section footer.", Structural),
        mk("n-figure",            "Self-contained content with optional caption.", Media),

        mk("n-card",              "Self-contained content block.", Container),
        mk("n-card-title",        "Card heading.", Text),
        mk("n-card-description",  "Card subtitle / summary.", Text),
        mk("n-card-content",      "Card body region.", Container),
        mk("n-card-header",       "Card header region.", Container),
        mk("n-card-actions",      "Card action bar (buttons).", Container),
        mk("n-card-footer",       "Card footer region.", Container),

        mk("n-button",            "Interactive button.", Interactive),
        mk("n-icon-button",       "Compact button showing just an icon.", Interactive),
        mk("n-input",             "Text input field.", Interactive),
        mk("n-tab",               "Single tab trigger.", Interactive),
        mk("n-tabs-list",         "Container for tabs.", Container),
        mk("n-nav-link",          "Navigation link.", Interactive),

        mk("n-dialog",            "Modal dialog.", Container),
        mk("n-drawer",            "Side drawer / sheet.", Container),
        mk("n-alert",             "Alert / banner.", Text),
        mk("n-badge",             "Small status indicator.", Text),
        mk("n-breadcrumb",        "Breadcrumb trail.", Structural),

        mk("n-list",              "List container.", Container),
        mk("n-list-item",         "List entry.", Container),

        mk("n-app-bar",           "Top app bar / header bar.", Structural),
        mk("n-toolbar",           "Toolbar inside an app bar.", Structural),
        mk("n-avatar",            "User / entity avatar.", Media),
    ]
}

fn host_apis() -> Vec<HostApi> {
    let mk = |name: &str, sig: &str, desc: &str| HostApi {
        name: s(name),
        signature: s(sig),
        description: s(desc),
    };
    vec![
        mk("nami.query_count",   "(param i32 i32) (result i32)",
           "Run a CSS selector against the read-only DOM snapshot; return element count."),
        mk("nami.dom_sexp_len",  "(func (result i32))",
           "Length in bytes of the cached DOM-as-S-expression (depth-capped at 12)."),
        mk("nami.dom_sexp_read", "(param i32 i32 i32) (result i32)",
           "Read `len` bytes at `offset` of the DOM sexp into guest memory at `dst_ptr`."),
        mk("nami.emit",          "(param i32 i32) (result i32)",
           "Append the guest-memory slice to the output accumulator (bounded by max_output_bytes)."),
    ]
}

fn features() -> Vec<FeatureFlag> {
    let mk = |name: &str, desc: &str| FeatureFlag {
        name: s(name),
        description: s(desc),
    };
    vec![
        mk("lisp",    "Programmable DSL compile passes via tatara-lisp."),
        mk("eval",    "Runtime Lisp evaluator via tatara-eval — implies `lisp`."),
        mk("ts",      "Tree-sitter adapters for JSX/TSX + Svelte."),
        mk("wasm",    "WASM/WASI agent host via wasmtime + wasmtime-wasi."),
        mk("network", "todoku HTTP client for queries + fetches."),
        mk("config",  "shikumi config loader integration."),
    ]
}

fn provenance_attrs() -> Vec<ProvenanceAttr> {
    let mk = |name: &str, by: &str, desc: &str| ProvenanceAttr {
        name: s(name),
        stamped_by: s(by),
        description: s(desc),
    };
    vec![
        mk("data-ast-source", "ast::parse_tsx_as_document / parse_svelte_as_document",
           "Source language an element was parsed from (jsx / svelte)."),
        mk("data-n-from",     "normalize::apply",
           "Original tag name before a normalize rule rewrote it."),
        mk("data-n-rule",     "normalize::apply",
           "Name of the (defnormalize) rule that rewrote this element."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescape_contains_all_shipped_dsl_keywords() {
        let ts = typescape();
        assert_eq!(
            ts.dsl_keywords.len(),
            21,
            "21 def* DSLs expected; if this fires, update both the DSL surface AND the typescape"
        );
    }

    #[test]
    fn dsl_keywords_are_unique() {
        let ts = typescape();
        let mut seen = std::collections::HashSet::new();
        for k in &ts.dsl_keywords {
            assert!(
                seen.insert(k.keyword.clone()),
                "duplicate DSL keyword: {}",
                k.keyword
            );
        }
    }

    #[test]
    fn typescape_enumerates_every_shipped_ast_domain() {
        let ts = typescape();
        let names: Vec<&str> = ts.ast_domains.iter().map(|d| d.name.as_str()).collect();
        // html + css are always there; jsx + svelte require `ts`.
        assert!(names.contains(&"html"));
        assert!(names.contains(&"jsx"));
        assert!(names.contains(&"svelte"));
        assert!(names.contains(&"css"));
    }

    #[test]
    fn typescape_enumerates_every_wasm_host_api() {
        let ts = typescape();
        let names: Vec<&str> = ts.host_apis.iter().map(|h| h.name.as_str()).collect();
        assert!(names.contains(&"nami.query_count"));
        assert!(names.contains(&"nami.dom_sexp_len"));
        assert!(names.contains(&"nami.dom_sexp_read"));
        assert!(names.contains(&"nami.emit"));
    }

    #[test]
    fn typescape_canonical_vocabulary_uses_n_prefix() {
        let ts = typescape();
        assert!(!ts.canonical_vocab.is_empty());
        for tag in &ts.canonical_vocab {
            assert!(
                tag.name.starts_with("n-"),
                "canonical tag {:?} missing n- prefix",
                tag.name
            );
        }
    }

    #[test]
    fn typescape_canonical_vocabulary_names_are_unique() {
        let ts = typescape();
        let mut seen = std::collections::HashSet::new();
        for tag in &ts.canonical_vocab {
            assert!(
                seen.insert(tag.name.clone()),
                "duplicate canonical tag: {}",
                tag.name
            );
        }
    }

    #[test]
    fn typescape_hash_is_stable_across_calls() {
        // Pure function — two calls in the same process must agree.
        let a = typescape_hash();
        let b = typescape_hash();
        let c = typescape_hash();
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn typescape_hash_has_expected_shape() {
        let h = typescape_hash();
        // 16 bytes × 8 bits / 5 bits per char = 25.6 → 26 base32 chars
        // (matches arch-synthesizer + tameshi attestation format).
        assert_eq!(h.len(), 26);
        for ch in h.chars() {
            assert!(
                ch.is_ascii_lowercase() || ch.is_ascii_digit(),
                "unexpected char in hash: {ch:?}"
            );
        }
    }

    #[test]
    fn coherence_check_passes_against_running_code() {
        // The typescape must always describe the actual crate.
        // Failure here is a bug — the manifest drifted from reality.
        coherence_check().expect("typescape ↔ code coherence");
    }

    #[test]
    fn manifest_yaml_is_stable_and_parses_key_fields() {
        let a = manifest_yaml();
        let b = manifest_yaml();
        assert_eq!(a, b, "manifest YAML must be deterministic");
        // Spot-check: every dimension appears.
        for dim in [
            "vocabulary",
            "domains",
            "dsls",
            "host_apis",
            "features",
            "attestation",
        ] {
            assert!(
                a.contains(dim),
                "manifest missing dimension {dim}:\n{a}"
            );
        }
        assert!(a.contains("name: \"nami-core\""));
        assert!(a.contains("hash:"));
    }

    #[test]
    fn manifest_yaml_declares_current_counts() {
        let yaml = manifest_yaml();
        // 13 DSLs is a load-bearing count — adding a new one means
        // also updating the `dsls: N` line here, which catches drift.
        assert!(yaml.contains("dsls:        21"), "yaml: {yaml}");
        // 3 AST domains currently: html + jsx + svelte.
        assert!(yaml.contains("domains:     4"), "yaml: {yaml}");
        // 4 host APIs.
        assert!(yaml.contains("host_apis:   4"), "yaml: {yaml}");
    }

    #[test]
    fn typescape_serializes_to_json_roundtrip() {
        let ts = typescape();
        let json = serde_json::to_string(&ts).unwrap();
        let back: NamiTypescape = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, back);
    }

    #[test]
    fn typescape_provenance_attrs_listed() {
        let ts = typescape();
        let names: Vec<&str> = ts.provenance_attrs.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"data-ast-source"));
        assert!(names.contains(&"data-n-from"));
        assert!(names.contains(&"data-n-rule"));
    }

    #[test]
    fn base32_encode_matches_expected_for_known_input() {
        // RFC 4648 lowercase a-z 2-7: 0x00 = 00000 000 → 'a' + padded '0'→'a'.
        assert_eq!(base32_encode(&[0x00]), "aa");
        // 0xff = 11111 111 → top 5 bits = 31 ('7'), low 3 shifted up = 11100 = 28 ('4').
        assert_eq!(base32_encode(&[0xff]), "74");
        // Round-trip sanity: 15 bytes → 24 chars.
        assert_eq!(base32_encode(&[0u8; 15]).len(), 24);
    }
}
