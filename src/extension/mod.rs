//! `(defextension)` — declarative browser-extension bundles.
//!
//! Absorbs Chrome MV3 + Firefox WebExtensions + Safari App Extensions
//! into one substrate pattern: a bundle is an `ExtensionSpec` that
//! names the other def* forms belonging to it, plus metadata,
//! permissions, and host-match patterns.
//!
//! Mapping (Chrome/Firefox ↔ ours):
//! - `manifest.json` top-level       → this `ExtensionSpec`
//! - content scripts                 → `(defdom-transform)`, `(defnormalize)`, `(defcssinject)`
//! - background scripts              → `(defeffect)`, `(defagent)`
//! - `browser_action` / commands     → `(defcommand)` + `(defbind)` (planned)
//! - `options_ui`                    → `(defcomponent)` + `(defstate)`
//! - `storage.local`                 → `(defstorage :name "<ext>")`
//! - `declarativeNetRequest`         → `(defblocker)`
//! - `i18n.getMessage`               → `(defmessages)` (planned)
//!
//! ```lisp
//! (defextension :name         "dark-reader"
//!               :version      "1.0.0"
//!               :author       "Jane"
//!               :description  "Per-site dark-mode CSS."
//!               :homepage-url "https://example.com"
//!               :permissions  (storage active-tab)
//!               :host-permissions ("*://*.example.com/*"
//!                                  "*://*.github.com/*")
//!               :rules        ("dark-reader/css"
//!                              "dark-reader/toggle-command")
//!               :enabled      #t)
//! ```
//!
//! **Gating semantics:** a rule listed in `:rules` is owned by this
//! extension. If the extension is disabled, the substrate skips every
//! owned rule regardless of its own trigger. If a rule's host doesn't
//! match the current URL's host (per `:host-permissions`), it also
//! skips — same model Chrome uses.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// WebExtensions-style permission grants. Each maps to capabilities
/// the substrate enforces when the extension's rules fire.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /// Scoped `chrome.storage.local` — backed by `(defstorage)`.
    Storage,
    /// Read/write cookies via the substrate cookie jar.
    Cookies,
    /// Read tab list, title, URL. Write = activation-only.
    Tabs,
    /// Read the currently-focused tab contents (one-shot, user-initiated).
    ActiveTab,
    /// Read browsing history.
    History,
    /// Read/write bookmarks.
    Bookmarks,
    /// Observe / modify outbound requests — `(defblocker)` domain list.
    WebRequest,
    /// Declarative network request rules (a superset of WebRequest).
    DeclarativeNetRequest,
    /// User notifications via tsuuchi.
    Notifications,
    /// Right-click context menu entries.
    ContextMenus,
    /// Omnibox suggestion provider.
    Omnibox,
    /// `chrome.scripting.executeScript` → inline-lisp evaluation.
    Scripting,
    /// File downloads.
    Downloads,
    /// Clipboard read/write via hasami.
    Clipboard,
}

impl Permission {
    /// Chrome/Firefox-style permission string, for manifest.json export.
    #[must_use]
    pub fn as_manifest_str(&self) -> &'static str {
        match self {
            Self::Storage => "storage",
            Self::Cookies => "cookies",
            Self::Tabs => "tabs",
            Self::ActiveTab => "activeTab",
            Self::History => "history",
            Self::Bookmarks => "bookmarks",
            Self::WebRequest => "webRequest",
            Self::DeclarativeNetRequest => "declarativeNetRequest",
            Self::Notifications => "notifications",
            Self::ContextMenus => "contextMenus",
            Self::Omnibox => "omnibox",
            Self::Scripting => "scripting",
            Self::Downloads => "downloads",
            Self::Clipboard => "clipboard",
        }
    }
}

/// One extension's full metadata + rule ownership.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defextension"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSpec {
    /// Stable identifier. Doubles as (defstorage) namespace prefix.
    pub name: String,
    /// Semver.
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage_url: Option<String>,
    /// Icon path (relative to the bundle root).
    #[serde(default)]
    pub icon: Option<String>,
    /// Granted capabilities. `storage` + `active-tab` are the
    /// conservative defaults; anything more needs explicit grant.
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// Host-match glob patterns. `*` and `?` wildcards; `://` preserved.
    /// Empty list → no host-restricted rules (but permissions still gate).
    #[serde(default)]
    pub host_permissions: Vec<String>,
    /// Names of other def* forms this extension owns. The substrate
    /// skips these rules when the extension is disabled.
    #[serde(default)]
    pub rules: Vec<String>,
    /// Install-time enabled state. Users can toggle at runtime via the
    /// registry. `true` by default (matches Chrome install flow).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl ExtensionSpec {
    /// Does the host URL satisfy this extension's host-permissions?
    /// Empty `host_permissions` means "all hosts" (same as `<all_urls>`).
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host_permissions.is_empty() {
            return true;
        }
        self.host_permissions
            .iter()
            .any(|pat| glob_match_host(pat, host))
    }

    /// True if `perm` is granted.
    #[must_use]
    pub fn has_permission(&self, perm: Permission) -> bool {
        self.permissions.contains(&perm)
    }

    /// True if `rule_name` is owned by this extension.
    #[must_use]
    pub fn owns_rule(&self, rule_name: &str) -> bool {
        self.rules.iter().any(|r| r == rule_name)
    }
}

/// Registry of installed extensions. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct ExtensionRegistry {
    specs: Vec<ExtensionSpec>,
}

impl ExtensionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Install / update an extension. Replaces any existing spec with
    /// the same `name` — version checks are the caller's job.
    pub fn insert(&mut self, spec: ExtensionSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ExtensionSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    /// Remove an extension. Returns `true` if one was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.specs.len();
        self.specs.retain(|s| s.name != name);
        self.specs.len() < before
    }

    /// Enable / disable at runtime without reinstalling.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> bool {
        for s in &mut self.specs {
            if s.name == name {
                s.enabled = enabled;
                return true;
            }
        }
        false
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
    pub fn specs(&self) -> &[ExtensionSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ExtensionSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Does any **enabled** extension own this rule, and if so, does
    /// its host filter pass for the given `host`? If no extension
    /// owns the rule, the rule runs (unowned rules are always on).
    #[must_use]
    pub fn rule_allowed(&self, rule_name: &str, host: &str) -> bool {
        // Find the owner, if any.
        let owner = self
            .specs
            .iter()
            .find(|s| s.owns_rule(rule_name));
        match owner {
            None => true, // unowned → always allowed
            Some(ext) => ext.enabled && ext.matches_host(host),
        }
    }

    /// Every rule name owned by every extension (enabled or not).
    #[must_use]
    pub fn owned_rules(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for s in &self.specs {
            for r in &s.rules {
                out.insert(r.clone());
            }
        }
        out
    }

    /// BLAKE3 content hash of the full installed set — 128 bits,
    /// 26-char base32 lowercase (pleme-io attestation convention).
    /// Stable across equivalent installations; any spec change,
    /// including enabled/disabled toggle, changes the hash.
    #[must_use]
    pub fn content_hash(&self) -> String {
        let json = serde_json::to_vec(&self.specs).unwrap_or_default();
        let h = blake3::hash(&json);
        base32_16(&h.as_bytes()[..16])
    }
}

fn base32_16(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// WebExtensions-compatible host glob: `*` → any segment (incl. empty),
/// `?` → any single char. Matches `host` case-insensitively.
///
/// Accepts Chrome-style patterns like `*://*.example.com/*` — we only
/// care about the host portion for matching; scheme/path segments are
/// preserved for manifest.json export.
#[must_use]
pub fn glob_match_host(pattern: &str, host: &str) -> bool {
    let host_part = extract_host_part(pattern);
    glob_match(&host_part.to_ascii_lowercase(), &host.to_ascii_lowercase())
}

fn extract_host_part(pattern: &str) -> &str {
    // Strip scheme and path portions for host matching.
    // `*://*.example.com/*` → `*.example.com`
    // `https://example.com`  → `example.com`
    let after_scheme = match pattern.find("://") {
        Some(i) => &pattern[i + 3..],
        None => pattern,
    };
    match after_scheme.find('/') {
        Some(i) => &after_scheme[..i],
        None => after_scheme,
    }
}

fn glob_match(pattern: &str, s: &str) -> bool {
    // Standard iterative glob with `*` (any run) and `?` (one char).
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t) = (usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_p = pi;
            star_t = ti;
            pi += 1;
        } else if star_p != usize::MAX {
            pi = star_p + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ExtensionSpec>, String> {
    tatara_lisp::compile_typed::<ExtensionSpec>(src)
        .map_err(|e| format!("failed to compile defextension forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ExtensionSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, rules: &[&str], hosts: &[&str]) -> ExtensionSpec {
        ExtensionSpec {
            name: name.into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            homepage_url: None,
            icon: None,
            permissions: vec![Permission::Storage],
            host_permissions: hosts.iter().map(|s| (*s).into()).collect(),
            rules: rules.iter().map(|s| (*s).into()).collect(),
            enabled: true,
        }
    }

    #[test]
    fn glob_matches_wildcard_segment() {
        assert!(glob_match_host("*://*.example.com/*", "blog.example.com"));
        assert!(glob_match_host("*://*.example.com/*", "a.b.example.com"));
        assert!(!glob_match_host("*://*.example.com/*", "evil.com"));
    }

    #[test]
    fn glob_matches_specific_host() {
        assert!(glob_match_host("https://example.com/*", "example.com"));
        assert!(!glob_match_host("https://example.com/*", "blog.example.com"));
    }

    #[test]
    fn glob_is_case_insensitive() {
        assert!(glob_match_host("*://EXAMPLE.COM/*", "example.com"));
        assert!(glob_match_host("*://example.com/*", "EXAMPLE.COM"));
    }

    #[test]
    fn empty_host_permissions_matches_all() {
        let s = sample("x", &[], &[]);
        assert!(s.matches_host("anything.com"));
        assert!(s.matches_host(""));
    }

    #[test]
    fn permission_grant_lookup() {
        let s = ExtensionSpec {
            permissions: vec![Permission::Storage, Permission::Cookies],
            ..sample("x", &[], &[])
        };
        assert!(s.has_permission(Permission::Storage));
        assert!(s.has_permission(Permission::Cookies));
        assert!(!s.has_permission(Permission::Tabs));
    }

    #[test]
    fn registry_roundtrips_install_disable_remove() {
        let mut reg = ExtensionRegistry::new();
        reg.insert(sample("a", &["a/rule"], &[]));
        reg.insert(sample("b", &["b/rule"], &[]));
        assert_eq!(reg.len(), 2);
        assert!(reg.set_enabled("a", false));
        assert!(!reg.get("a").unwrap().enabled);
        assert!(reg.remove("b"));
        assert_eq!(reg.len(), 1);
        assert!(!reg.remove("nonexistent"));
    }

    #[test]
    fn insert_with_same_name_replaces() {
        let mut reg = ExtensionRegistry::new();
        reg.insert(sample("x", &["old"], &[]));
        reg.insert(sample("x", &["new"], &[]));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("x").unwrap().rules, vec!["new"]);
    }

    #[test]
    fn rule_allowed_when_unowned() {
        let reg = ExtensionRegistry::new();
        assert!(reg.rule_allowed("some/rule", "example.com"));
    }

    #[test]
    fn rule_allowed_respects_enabled_state() {
        let mut reg = ExtensionRegistry::new();
        reg.insert(sample("dark", &["dark/css"], &[]));
        assert!(reg.rule_allowed("dark/css", "example.com"));
        reg.set_enabled("dark", false);
        assert!(!reg.rule_allowed("dark/css", "example.com"));
    }

    #[test]
    fn rule_allowed_respects_host_permissions() {
        let mut reg = ExtensionRegistry::new();
        reg.insert(sample("x", &["x/rule"], &["*://*.example.com/*"]));
        assert!(reg.rule_allowed("x/rule", "blog.example.com"));
        assert!(!reg.rule_allowed("x/rule", "evil.com"));
    }

    #[test]
    fn owned_rules_unions_across_extensions() {
        let mut reg = ExtensionRegistry::new();
        reg.insert(sample("a", &["a/1", "a/2"], &[]));
        reg.insert(sample("b", &["b/1"], &[]));
        let owned = reg.owned_rules();
        assert_eq!(owned.len(), 3);
        assert!(owned.contains("a/1"));
        assert!(owned.contains("a/2"));
        assert!(owned.contains("b/1"));
    }

    #[test]
    fn content_hash_is_128_bit_base32_and_deterministic() {
        let mut reg = ExtensionRegistry::new();
        reg.insert(sample("a", &["a/1"], &[]));
        let h1 = reg.content_hash();
        assert_eq!(h1.len(), 26);
        for ch in h1.chars() {
            assert!(ch.is_ascii_lowercase() || ch.is_ascii_digit());
        }
        // Stable:
        let h2 = reg.content_hash();
        assert_eq!(h1, h2);
    }

    #[test]
    fn content_hash_changes_on_mutation() {
        let mut reg = ExtensionRegistry::new();
        reg.insert(sample("a", &["a/1"], &[]));
        let before = reg.content_hash();
        reg.set_enabled("a", false);
        assert_ne!(before, reg.content_hash());
    }

    #[test]
    fn permission_manifest_string_maps_chrome_style() {
        assert_eq!(Permission::Storage.as_manifest_str(), "storage");
        assert_eq!(Permission::ActiveTab.as_manifest_str(), "activeTab");
        assert_eq!(
            Permission::DeclarativeNetRequest.as_manifest_str(),
            "declarativeNetRequest"
        );
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_full_extension_form() {
        let src = r#"
            (defextension :name "dark-reader"
                          :version "1.0.0"
                          :author "Jane"
                          :description "Per-site dark mode."
                          :homepage-url "https://example.com"
                          :permissions (storage active-tab)
                          :host-permissions ("*://*.example.com/*"
                                             "*://*.github.com/*")
                          :rules ("dark-reader/css")
                          :enabled #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "dark-reader");
        assert_eq!(s.version, "1.0.0");
        assert_eq!(s.author.as_deref(), Some("Jane"));
        assert_eq!(s.permissions.len(), 2);
        assert!(s.permissions.contains(&Permission::Storage));
        assert!(s.permissions.contains(&Permission::ActiveTab));
        assert_eq!(s.host_permissions.len(), 2);
        assert_eq!(s.rules, vec!["dark-reader/css"]);
        assert!(s.enabled);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_minimal_form_roundtrips() {
        // Missing optional fields are fine — spec compiles, registry
        // can install it, caller decides whether to toggle enabled.
        let src = r#"(defextension :name "x" :version "1")"#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "x");
        assert_eq!(specs[0].version, "1");
        assert!(specs[0].permissions.is_empty());
        assert!(specs[0].host_permissions.is_empty());
    }
}
