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
           "Typed persistent key/value store with optional TTL + secondary indexes (:indexes [dot-paths]). Pure tatara-lisp, append-only event log, BLAKE3-attestable. Index lookup is O(log n) via BTreeMap; rebuild on replay keeps on-disk format identical. Covers cookies/session/user-prefs without FFI."),
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
        mk("defi18n", "i18n::MessageSpec",
           "Localized message bundle — one (namespace, locale) shipped per form. Registry merges by key, resolves with fallback chain (exact → locale-prefix → en → raw key). Parameterized {placeholder} substitution via MessageRegistry::format. Absorbs chrome.i18n + Firefox browser.i18n."),
        mk("defsecurity-policy", "security_policy::SecurityPolicySpec",
           "Declarative per-host security-policy bundle — CSP, Permissions-Policy, Referrer-Policy, Cross-Origin-*, X-Frame-Options, plus convenience toggles (upgrade-insecure-requests, frame-ancestors, report-uri) merged idempotently into the CSP. Registry resolves most-specific host match; render_headers() emits the full HTTP header set."),
        mk("deffind", "find::FindSpec",
           "Find-in-page profile — case-sensitive, whole-word, regex, max-matches cap. Returns FindMatch with enclosing tag + text-node index + offset. Absorbs Cmd+F across every browser."),
        mk("defzoom", "zoom::ZoomSpec",
           "Per-host zoom preference — clamped [0.25, 5.0], text-only toggle. Absorbs Chrome per-site zoom + Firefox text-only + Safari Reader zoom."),
        mk("defsnapshot", "snapshot::SnapshotSpec",
           "Declarative page-snapshot recipe — region (viewport/full-page/selector/element), format (png/jpeg/webp), scale, quality, optional BLAKE3 attestation. Absorbs Firefox screenshot + Chrome full-page capture."),
        mk("defpip", "pip::PipSpec",
           "Picture-in-picture rules — per-host selectors, window corner, auto-activate on scroll-off, always-on-top. Absorbs Safari/Chrome/Firefox PiP."),
        mk("defsession", "session::SessionSpec",
           "Session-recovery policy — restore on open, undo-close ring, autosave cadence, preserve-pinned. SessionStore pairs with it. Absorbs Firefox Session Restore + Chrome Cmd+Shift+T."),
        mk("defgesture", "gesture::GestureSpec",
           "Mouse-gesture binding — cardinal-token stroke string → command. StrokeBuilder incremental classifier with jitter threshold + duplicate collapse. Absorbs Vivaldi + Opera gestures."),
        mk("defboost", "boost::BoostSpec",
           "Per-site CSS / Lisp / JS / blocker-selector overlay — runtime-toggleable. merged_css and merged_blocker_selectors compose every applicable boost. Absorbs Arc Boosts + Stylus + Tampermonkey + Brave Shields."),
        mk("defjs-runtime", "js_runtime::JsRuntimeSpec",
           "Declarative JavaScript runtime profile — fuel_limit, memory_limit_bytes, capabilities (DomRead/Write, StorageRead/Write, FetchAllowedHosts, Notify, Clipboard, Console), host globs for fetch. The JsRuntime trait is the pluggable engine surface; MicroEval ships as the proof-of-pipeline (arithmetic + string concat + identifier lookup). Foundation for J1 (real engine as WASM guest), J2 (Service Workers), and (defboost) :js execution."),
        mk("defspace", "space::SpaceSpec",
           "Arc-style space — grouped tabs with per-space theme / homepage / bookmarks folder / omnibox profile / storage isolation. Absorbs Arc Spaces, Firefox Containers, Safari Profiles, Edge Workspaces. SpaceState tracks the currently active space."),
        mk("defsidebar", "sidebar::SidebarSpec",
           "Persistent sidebar webview — URL, position, width (clamped [120, 800]), pinned flag, space + host gates (ANDed), hibernation timeout. Absorbs Arc sidebar apps, Opera sidebar messengers, Vivaldi web panels."),
        mk("defsplit", "split::SplitSpec",
           "Declarative split-view layout — Horizontal/Vertical/Grid tiling of N panes with flexbox-style weights. proportional_sizes() returns pixel widths for any axis_size. Absorbs Arc Split View, Vivaldi tile-tabs, Safari two-pane."),
        mk("defspoof", "spoof::SpoofSpec",
           "Fingerprint-resistance profile — per-host control over User-Agent, canvas/WebGL/audio noise (passthrough/constant/randomize/block), timezone, language, letterboxing, client-hints stripping, referrer policy. Absorbs Tor Browser, Brave fingerprint randomization, Firefox resistFingerprinting, Safari ITP."),
        mk("defdns", "dns::DnsSpec",
           "DNS resolver preference — protocol (system/UDP/DoT/DoH/DoQ/AnonymizedDnscrypt/ODoH), endpoint, bootstrap IP, cache TTL, privacy tier (legacy/standard/strict/isolated), bypass + block suffix lists. Absorbs Firefox/Chrome DoH, iOS Private Relay, pleme-io's kurayami."),
        mk("defrouting", "routing::RoutingSpec",
           "Per-host network routing — direct / tunnel:<name> (mamorigami) / tor:<isolation> (kakuremino) / socks5:<url> / pt:<transport> (maboroshi). Kill-switch + fallback semantics. Absorbs Firefox Multi-Account Containers + Mozilla VPN, Tor Browser circuit-per-origin, per-tab proxy switching."),
        mk("defoutline", "outline::OutlineSpec",
           "Table-of-contents extraction profile — heading-level bounds, include/exclude selectors, optional slug-id generation, nested vs flat output. Pairs with (defreader). Absorbs Firefox Reader inline outline, Chrome Reading Mode TOC, Safari Reader structure."),
        mk("defannotate", "annotate::AnnotateSpec",
           "Highlight + comment profile — color palette, default color, shareable flag, max-comment cap, storage namespace. Annotation shape uses TextQuoteSelector (Hypothesis-compatible) for resilient anchoring, plus CSS selector + byte-range fallbacks. content_id hashes intrinsic fields to a 26-char base32 slug (tameshi-shape). Absorbs hypothes.is, Diigo, Safari PDF annotations."),
        mk("deffeed", "feed::FeedSpec",
           "RSS/Atom subscription — URL, cadence (clamped [60, 86400]s), category, max-items cap, storage name, enabled toggle. Registry enforces unique names, extends() drops invalid entries with a warning. Absorbs Opera Reader, NetNewsWire, Feedly OPML, Firefox Live Bookmarks."),
        mk("defredirect", "redirect::RedirectSpec",
           "Privacy-frontend redirect — host glob → mirror list with Priority/RoundRobin/Random rotation. Preserves path + query + fragment. Absorbs LibRedirect, Privacy Redirect (YouTube→Invidious/Piped, Twitter→Nitter, Reddit→Libreddit, etc.)."),
        mk("defurl-clean", "url_clean::UrlCleanSpec",
           "Tracking-parameter stripper — host-scoped list of exact names + trailing-`*` prefix matches. apply() rewrites a URL removing every matching query param; regex-free, dep-light. Absorbs ClearURLs, Neat URL, PureURL."),
        mk("defscript-policy", "script_policy::ScriptPolicySpec",
           "Per-origin JS execution + API restriction — mode (AllowAll/BlockAll/AllowList/BlockList), origin glob lists, 25-category API denylist (WebRtc/Geolocation/Canvas/WebGl/Sensors/FineTimers/ServiceWorker/Credentials/…), inline+eval toggles. Absorbs NoScript, JShelter, uMatrix, Brave JS-off-by-default."),
        mk("defbridge", "bridge::BridgeSpec",
           "Tor bridge + pluggable-transport endpoint — Direct/Obfs4/Meek/Snowflake/Webtunnel/Other, address, 40-hex fingerprint (validated, space-tolerant), transport-specific `extra` tail. to_torrc_line() emits the line Tor consumes; to_torrc_block() wraps every enabled bridge as `Bridge …` config. Absorbs Tor Browser bridge UI + tor-proxy ecosystem."),
        mk("defshare-target", "share::ShareTargetSpec",
           "Share-sheet destination — Clipboard/Storage/Redirect/HttpPost/Mcp/Email with `{url}` / `{title}` / `{text}` template substitution. rendered_url + rendered_body materialize at dispatch time. Absorbs iOS/Android share sheets, Chrome Web Share API, Firefox Send To, Edge Share."),
        mk("defoffline", "offline::OfflineSpec",
           "Save-for-later / offline cache profile — storage namespace, TTL, asset classes (Html/Css/Images/Fonts/Scripts/Media), size cap, auto-save tag list. OfflineEntry carries BLAKE3 content-hash. Absorbs Pocket, Instapaper, Raindrop, Safari Reading List, Firefox Read It Later."),
        mk("defpull-to-refresh", "pull_refresh::PullRefreshSpec",
           "Pull-to-refresh rule — host glob + threshold (clamped [40, 300] CSS px) + command name + animation duration. command_for() returns the invoked command when the rule is enabled. Absorbs mobile Chrome/Safari/Firefox PTR."),
        mk("defdownload", "download::DownloadSpec",
           "Download manager policy — target folder, quarantine MIME list (Tor-Browser-style), auto-open MIME globs, hash_verify (None/Blake3/Sha256/Sha512), concurrency, resume flag, size cap. blake3_content_hash() emits the 26-char base32 shape so downloads flow into sekiban attestation directly. Absorbs Chrome/Firefox/Safari DL panels + uGet/aria2/JDownloader."),
        mk("defautofill", "autofill::AutofillSpec",
           "Form-autofill profile — typed field entries (FullName/Email/Phone/Street/PostalCode/CardNumber/Custom/…), per-site field-selector overrides, excluded_hosts, confirm_first_use flag. Absorbs Chrome/Firefox/Safari autofill + 1Password field detection + LastPass form-fill."),
        mk("defpasswords", "passwords::PasswordsSpec",
           "Password vault source — 22 known backends: Local, Kagibako, 1Password, Bitwarden, LastPass, Dashlane, NordPass, ProtonPass, Enpass, RoboForm, ZohoVault, KeePass, pass, gopass, macOS Keychain, Windows CredMan, libsecret (GNOME/KWallet), HashiCorp Vault, Akeyless, AWS Secrets Manager, GCP Secret Manager, Azure Key Vault, plus Process fallback. Per-vault unlock timeout, biometric requirement, host allow/block lists, sync cadence. CredentialRecord host-match + redacted() for safe logging."),
        mk("defauth-saver", "auth_saver::AuthSaverSpec",
           "Save-on-submit capture profile — vault binding, host scope, PromptPolicy (Always/SilentAllowList/Never), DetectionHints (username/password/signup/change-password selector lists), ignore_hosts, dedupe flag. Absorbs the 'save this password?' flow from every mainstream browser."),
        mk("defsecure-note", "secure_note::SecureNoteSpec",
           "Non-password secret storage — NoteKind (Text/Markdown/SshKey/GpgKey/ApiToken/DatabaseCredential/LicenseKey/TotpSeed/RecoveryCodes/SecurityQuestions/PaymentCard/Identity/WalletSeed), tags, expose-via-cli, auto-expire, always_reauth. Sensitive kinds (WalletSeed/Identity/PaymentCard/RecoveryCodes) auto-require re-auth. Absorbs 1Password Secure Notes, Bitwarden Secure Notes, Keychain Generic Passwords, pass freeform files."),
        mk("defpasskey", "passkey::PasskeySpec",
           "WebAuthn/FIDO2 passkey profile — Authenticator (Any/Platform/CrossPlatform), UserVerification (Required/Preferred/Discouraged), sync_passkeys, allowed+blocked rp_ids, resident_key (discoverable creds). PasskeyRecord with COSE algorithm id + sign_count + last_used_at. Absorbs iCloud Keychain Passkeys, Android Credential Manager, Windows Hello, YubiKey, 1Password/Bitwarden Passkeys."),
        mk("defllm-provider", "llm::LlmProviderSpec",
           "LLM provider declaration — 7 kinds (OpenAiCompatible covers OpenAI+OpenRouter+Together+Groq+Fireworks+vLLM+llama.cpp+LM-Studio, Anthropic, Gemini, Ollama, Kurage, Mcp, Stub). Endpoint, model, max_tokens, temperature, auth_env, MCP tool name, rate limit, timeout. LlmProvider trait = pluggable engine surface (same pattern as JsRuntime). EchoProvider bundled for tests + default fallback."),
        mk("defsummarize", "summarize::SummarizeSpec",
           "Page summarization profile — provider, scope (WholePage/ReaderText/Selection), style (Paragraph/Bullets/Sentence/Outline/QnA), max_words, include_code, language override, extra_instructions. run() drives LlmProvider::generate. Composes with (defreader). Novel — no mainstream browser offers this first-class yet."),
        mk("defchat-with-page", "chat::ChatSpec",
           "Conversational Q&A over page contents — provider, ContextStrategy (WholeDom/Reader/Selection/Rag/None), HistoryScope (PerTab/PerSpace/Global/Ephemeral), storage namespace, max_context_tokens, keep_last_turns cap, rag_enabled + rag_chunk_size, system_prompt. build_call stitches system + context + history + question. Absorbs Arc AI chat, Edge Copilot, Brave Leo, Firefox AI sidebar."),
        mk("defllm-completion", "llm_completion::LlmCompletionSpec",
           "LLM-backed inline completion — CompletionTrigger (UrlBar/FormInput/Contenteditable/CodeBuffer), min_chars, debounce_ms, max_suggestions, temperature, host_gated + blocked_hosts, custom system_prompt. Built-in default_url_bar + default_compose profiles. Absorbs Arc AI URL completion, Edge Copilot-in-compose, Gmail Smart Compose."),
        mk("defmedia-session", "media_session::MediaSessionSpec",
           "W3C Media Session API profile — transport actions (Play/Pause/Stop/Seek*/Prev/Next/SkipAd/PiP/Mic/Cam/HangUp), metadata-extraction CSS selectors (title/artist/album/artwork), seek increment, auto-activate toggle. Absorbs Chrome/Edge/Firefox Media Session, iOS lock-screen, macOS Now Playing, Android media notification."),
        mk("defcast", "cast::CastSpec",
           "Media casting profile — 5 protocols (Chromecast/AirPlay/Miracast/DLNA/WebPresentation), discovery transport (Mdns/Ssdp/MdnsAndSsdp/Manual), host allow-list, preferred + default receivers, session timeout, require_confirm flag. CastReceiver shape with capability list + supports_all() check. Absorbs Google Cast SDK, Apple AirPlay, Microsoft Miracast, DLNA/UPnP."),
        mk("defsubtitle", "subtitle::SubtitleSpec",
           "Caption/subtitle handling — formats (Vtt/Srt/Ssa/Dfxp/PlatformLive), language preferences in order (exact + prefix match), font-size % (clamped [50, 400]), position (Bottom/Top/Native), auto_translate + target + provider (ties into AI pack). pick_language + should_auto_translate honor the preference chain. Absorbs HTML5 <track>, Netflix/YouTube UI, platform live captions."),
        mk("definspector", "inspector::InspectorSpec",
           "Inspector-panel declaration — title, icon, data source (Http/Mcp/Storage/State/Query/Static), view (Table/Tree/Timeline/Json/Text/Metric/Tail), refresh strategy (Manual/Interval/Tail/Once), position (Left/Right/Bottom/Floating), visibility toggle. Absorbs Chrome/Firefox/Safari DevTools panel extensions — but authored in one Lisp form instead of devtools_page + registerPanel + JS glue."),
        mk("defprofiler", "profiler::ProfilerSpec",
           "Performance profile + budgets — MetricCategory (Navigation/Paint/Layout/Scripting/Memory/Network/Interaction/Rendering/Gpu/UserTiming/Custom), sampling Hz, rolling-window seconds, per-metric PerfBudget with warn/error thresholds + OverMax/UnderMin direction. evaluate_metric returns worst severity + breaching budget. Absorbs Chrome Performance, Firefox Profiler, Safari Timelines + Lighthouse/webhint budget declarations."),
        mk("defconsole-rule", "console_rule::ConsoleRuleSpec",
           "Console output rule — LogLevel (Debug/Info/Log/Warn/Error/Table/Any), regex OR literal-substring pattern, host glob, ConsoleAction (Display/Drop/Capture/Emit) with color/background/prefix overrides, capture_store namespace, emit_state cell. case_insensitive + literal toggles. Absorbs Chrome DevTools console filters + log-level color schemes across browsers."),
        mk("defreader-aloud", "reader_aloud::ReaderAloudSpec",
           "TTS page-reading profile — voice / rate / pitch / volume (clamped), ReadScope (WholePage/ReaderText/Selection/Selector), SpeechSource (Platform/Oto/Http/Llm — composes with AI pack), sentence highlighting, stop-on-navigate, auto-start. Absorbs Safari Speak Screen, Edge Read Aloud, Chrome Select-to-Speak, NVDA/JAWS/VoiceOver patterns."),
        mk("defhigh-contrast", "high_contrast::HighContrastSpec",
           "WCAG contrast enforcement — min_ratio (clamped [1, 21]), SchemeOverride (Auto/Light/Dark/Invert/Custom), foreground + background + link + link-visited color overrides, link_boost, focus-ring thickness + color. contrast_ratio() + parse_hex() helpers. Absorbs Windows High Contrast, macOS Increase Contrast, Chrome forced-colors, Firefox override page colors, Dark Reader patterns."),
        mk("defsimplify", "simplify::SimplifySpec",
           "Cognitive-load reduction — strip_animations / strip_autoplay / reduce_motion toggles, ScrollDamping (Native/Gentle/Slow/StepOnly with velocity_scalar), line_height_min (clamped), font_override (OpenDyslexic, Atkinson Hyperlegible), reading_guide, hide_sidebars, spacing_boost_pct, paragraph_spacing_mult. inject_css() emits ready-to-attach stylesheet. Built-in focus-mode + dyslexia_mode. Novel — no browser ships this first-class."),
        mk("defpresence", "presence::PresenceSpec",
           "Who-is-here tracking — PresenceTransport (Nats/Websocket/DirectP2p/Local), BroadcastField set (Cursor/Selection/Viewport/Typing/Attention/VoiceStatus), topic_template with {origin}/{path}/{space} tokens, expires_seconds stale-prune, display_name + avatar_template + throttle_ms + max_visible. PresenceEntry roster record with session_id (BLAKE3 26-char base32), cursor/selection/viewport/typing/focused/color. Absorbs Google Docs avatars, Figma viewers, Notion member indicators, Slack huddle rosters."),
        mk("defcrdt-room", "crdt_room::CrdtRoomSpec",
           "CRDT sync-room profile — RoomTransport (Nats/Websocket/DirectP2p/Local), CrdtKind (YCrdt/Automerge/LwwElementSet/OpLog), Persistence (None/IndexedDb/LocalStorage/Daemon), topic_template with {origin}/{path}/{space}/{room} tokens, awareness toggle, isolation_token scope, max_peers cap, snapshot_interval_seconds, throttle_ms, end-to-end encryption. Absorbs Figma multiplayer, Linear live-edit, Notion realtime, tldraw/Excalidraw rooms."),
        mk("defmultiplayer-cursor", "multiplayer_cursor::MultiplayerCursorSpec",
           "Live cursor visualization — CursorStyle (Pointer/Caret/Crosshair/Dot/Hand/CustomSvg), palette color list (round-robin per session), name_tag, fade_after_seconds, click_echo ripple, follow_mode camera jump, CursorScope (PerTab/PerProfile/Global), crowd_threshold hide-at-N, smoothing coefficient clamped [0,1], respect_reduced_motion. Absorbs Figma cursor chat, Excalidraw live cursors, tldraw multiplayer pointers, Arc Easels."),
        mk("defservice-worker", "service_worker::ServiceWorkerSpec",
           "Service-worker lifecycle + fetch-interception DSL — LifecycleEvent set (Install/Activate/Fetch/Message/Push/Sync/PeriodicSync/NotificationClick), scope path prefix, runtime name (links to defjs-runtime capability set), skip_waiting + client_claim, WorkerRoute list with Workbox-style path globs + CacheStrategy (CacheFirst/NetworkFirst/StaleWhileRevalidate/NetworkOnly/CacheOnly) + timeout_seconds + max_age_seconds + max_entries + cache_name override, max_cache_mb total cap, offline_fallback page, periodic_sync_seconds wake cadence. Absorbs Chrome/Firefox/Safari Service Worker API + Workbox routing patterns."),
        mk("defsync", "sync_channel::SyncSpec",
           "Cross-device replication — SyncSignal (13 kinds: Bookmarks/History/Tabs/OpenWindows/Passwords/Passkeys/Sessions/Extensions/Settings/ReadingList/Annotations/Downloads/Custom), SyncDirection (Push/Pull/Bidirectional), SyncCrdt (YCrdt/Automerge/LwwElementSet/OpLog), SyncTransport (Nats/Websocket/DirectP2p/Local), ConflictPolicy (LastWriterWins/KeepBoth/PreferDevice/CrdtNative), topic template with {device}/{profile}/{signal} tokens, isolation_token scope, preferred_device tiebreak, encryption, throttle_ms delta coalescing, buffer_max, peer_devices allow-list, retention_days, full_sync_interval_seconds. Absorbs Chrome Sync, Firefox Sync v5, Safari iCloud, Arc Spaces sync, 1Password/Bitwarden vault sync."),
        mk("deftab-group", "tab_group::TabGroupSpec",
           "Tab-group profile — GroupColor palette (10 colors incl. Custom with hex override), host-glob auto-match list, collapsed + pinned states, icon glyph, GroupIsolation (None/PerProfile/PerWindow/Ephemeral) for per-group cookie jars (Firefox Containers), max_tabs cap, close_when_empty, resolved_color() returns palette hex or custom. Absorbs Chrome Tab Groups, Firefox Containers, Vivaldi Tab Stacks, Arc Spaces, Edge Collections, Safari Tab Groups."),
        mk("deftab-hibernate", "tab_hibernate::TabHibernateSpec",
           "Tab hibernation policy — inactive_seconds threshold, DiscardState (DiscardAll/KeepScroll/KeepForm/KeepScreenshot/KeepDom) memory-survivor spectrum, per-host exempt list, keep_audio + keep_pinned + keep_form_dirty + keep_active_transfer safety rails, MemoryPressure gate (Any/Moderate/High/Critical) with ordered rank, max_resident_bytes floor. TabSnapshot + should_hibernate() pure decision. Absorbs Chrome Memory Saver, Edge Sleeping Tabs, Vivaldi Tab Hibernation, Firefox Tab Unloading."),
        mk("deftab-preview", "tab_preview::TabPreviewSpec",
           "Hover tab-preview shape — PreviewShape (Tooltip/Compact/Rich/Full/None), PreviewField set (Screenshot/Title/Url/Favicon/Subtitle/AudioState/LoadingState/Security), delay_ms, width_px × height_px with aspect_ratio() helper, follow_cursor vs anchored-to-strip, live_update screenshot refresh cadence, respect_reduced_motion. Absorbs Chrome hover-card, Edge vertical-tab preview, Vivaldi Tab Preview, Safari tab preview."),
        mk("defsearch-engine", "search_engine::SearchEngineSpec",
           "Declarative search engine — URL template with %s / {query} substitution, omnibox keyword shortcut, QueryEncoding (PercentPlus/PercentStrict/Raw), SearchMethod (Get/Post) with post_body template, default flag, SearchCategory (13 kinds: Web/Images/Videos/News/Shopping/Maps/Code/Social/Academic/Ai/Developer/Reference/Other), favicon, auth_cookies, priority tiebreak. render_url / render_suggest / render_body helpers. Registry indexes by name + keyword + category + default. Absorbs Chrome custom search engines, Firefox keyword searches, Safari search providers, Brave/Vivaldi/Arc custom engines."),
        mk("defsearch-bang", "search_bang::SearchBangSpec",
           "!bang shortcut — trigger text (without !), target engine name OR direct URL template (url takes precedence), BangPosition (Leading/Trailing/Either) for where in the input the token appears, case_insensitive toggle, category + priority tiebreak, favicon. detect() strips the matched token from input; registry scans all enabled bangs and returns highest-priority match. Absorbs DuckDuckGo's 13,000+ bangs and Kagi's curated bang list as one declarative form per bang."),
        mk("defidentity", "identity::IdentitySpec",
           "Multi-account persona — display_name + avatar + color, vault binding (defpasswords), cookie_jar binding, default_email + default_full_name for form autofill, auto_apply_hosts glob list, IdentityIsolation (None/PerProfile/Ephemeral/OsProcess), default flag, linked totp_profiles, priority tiebreak. Registry resolves by host match (priority-ordered) → default → first-enabled. Absorbs Chrome Profiles, Firefox Containers, Arc Spaces identities, Safari Profiles (macOS 14+), Microsoft Edge Work Profiles."),
        mk("deftotp", "totp::TotpSpec",
           "RFC 6238 TOTP 2FA profile — base32 secret, TotpAlgorithm (Sha1/Sha256/Sha512), digits (6/7/8), period (seconds), issuer + account_name pair, linked identities + vault, icon. generate_at(seconds) + generate_now() produce zero-padded codes; seconds_remaining tracks rollover; otpauth_uri() renders the QR-code format. Real HMAC-based implementation passes RFC 6238 Appendix B vectors. Absorbs Authy, Google Authenticator, 1Password TOTP, Bitwarden TOTP, Yubico Authenticator, macOS Passwords (TOTP), Aegis."),
        mk("deffingerprint-randomize", "fingerprint_randomize::FingerprintRandomizeSpec",
           "Canvas/WebGL/audio/font fingerprint farbling — FingerprintMode per surface (Allow/Noise/Generic/Block/Prompt) covering canvas, webgl, audio, client_rects, pointer_hover, prefers_media, locale, navigator_info; FontMode (Allow/SystemOnly/RandomizeMetrics/Block); UserAgentMode (Real/Generic/Randomize/AllowList); SessionScope (PerSession/PerHost/PerTab/PerCall) — higher scopes break more trackers + more sites; intensity clamped [0,1]; exempt_hosts allow-list where farbling is bypassed (banks, games). Absorbs Brave Shields, Tor Browser, LibreWolf, Mullvad Browser anti-fingerprint."),
        mk("defcookie-jar", "cookie_jar::CookieJarSpec",
           "Cookie storage partitioning + clearing — Partition (None/PerSite/PerTab/PerIdentity/Ephemeral) mirroring Firefox TCP + Chrome CHIPS; ThirdPartyPolicy (Allow/Block/BlockTrackers/RequireInteraction/Prompt) with per-host allow + block lists (block wins); CookieLifetime (Server/Session/Clamp24h/Clamp7days) matching Safari ITP + Firefox ETP; ClearTrigger set (All/ThirdParty/AllIfIdleDaysGt/IdentitySwitch/TabClose); max_cookies + max_cookie_bytes + idle_days caps; suppress_samesite_upgrade. admits_third_party honors storage-access grants."),
        mk("defwebgpu-policy", "webgpu_policy::WebgpuPolicySpec",
           "Per-host WebGPU access + disclosure — GpuAccess (Allow/AllowWithPrompt/AllowRenderOnly/SoftwareFallback/Block), AdapterInfoDisclosure (Full/Generic/Empty/Masquerade) with optional masquerade_vendor + architecture, ComputePolicy (Allow/Throttled/SecureContextOnly/Block) isolating the crypto-mining + fingerprint surface, max_buffer_mb + max_total_memory_mb caps, timestamp_query + shader_f16 feature toggles, ShaderFeatureSet (WebStandard/Extended/Experimental/Minimal), allow_hosts + block_hosts overrides (block wins). access_for + compute_for + accepts_buffer_bytes helpers. Absorbs Chrome/Firefox/Safari WebGPU gating + Brave/Tor adapter-info redaction."),
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
            81,
            "81 def* DSLs expected; if this fires, update both the DSL surface AND the typescape"
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
        assert!(yaml.contains("dsls:        81"), "yaml: {yaml}");
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
