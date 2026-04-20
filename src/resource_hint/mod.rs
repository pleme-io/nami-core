//! `(defresource-hint)` — declarative resource hints.
//!
//! Absorbs `<link rel="preload|preconnect|prefetch|dns-prefetch|
//! modulepreload">`, Chrome Priority Hints (`fetchpriority`), and
//! Early Hints (HTTP 103). Each profile declares which hints to
//! inject automatically for a host, so authors don't have to hand-
//! edit HTML to get CDN preconnects right.
//!
//! ```lisp
//! (defresource-hint :name           "cdn-preconnect"
//!                   :host           "*://*.example.com/*"
//!                   :kind           :preconnect
//!                   :url            "https://cdn.example.com"
//!                   :crossorigin    #t
//!                   :fetch-priority :high)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Hint type.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum HintKind {
    /// Open TCP + TLS early — `rel="preconnect"`.
    #[default]
    Preconnect,
    /// Resolve DNS early — `rel="dns-prefetch"`.
    DnsPrefetch,
    /// Fetch a resource with high priority for imminent use —
    /// `rel="preload"`.
    Preload,
    /// Low-priority fetch for a later navigation — `rel="prefetch"`.
    Prefetch,
    /// Same as Preload but for ES modules — `rel="modulepreload"`.
    ModulePreload,
    /// HTTP 103 Early Hints — server-side only; still declared here
    /// so the manifest records intent.
    EarlyHints,
}

/// Resource type hint — maps to `as="…"` on a `<link rel="preload">`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AsKind {
    #[default]
    Fetch,
    Script,
    Style,
    Image,
    Font,
    Audio,
    Video,
    Track,
    Document,
    Worker,
    Object,
    Embed,
    /// None = don't set `as=` (fine for preconnect/dns-prefetch).
    None,
}

/// Chrome Priority Hints.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FetchPriority {
    #[default]
    Auto,
    Low,
    High,
}

/// When to inject the hint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum InjectTiming {
    /// Insert at parse time (before first paint).
    #[default]
    OnParse,
    /// Wait for the main resource to start loading.
    OnMainLoad,
    /// On user idle (useful for prefetch).
    OnIdle,
    /// After user interaction (click, keydown).
    OnInteraction,
}

/// Hint profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defresource-hint"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceHintSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub kind: HintKind,
    /// Target URL (absolute or path). Empty = use `host` verbatim
    /// (makes sense only for DnsPrefetch/Preconnect).
    pub url: String,
    #[serde(default)]
    pub as_kind: AsKind,
    /// MIME type hint — e.g. `"font/woff2"`.
    #[serde(default)]
    pub mime_type: Option<String>,
    /// CORS credentials mode — matters for fonts.
    #[serde(default)]
    pub crossorigin: bool,
    /// Integrity hash (SRI). Empty = skip.
    #[serde(default)]
    pub integrity: Option<String>,
    #[serde(default)]
    pub fetch_priority: FetchPriority,
    /// Media query for conditional hints (e.g. `"(min-width: 768px)"`).
    #[serde(default)]
    pub media: Option<String>,
    /// Viewport predicates — only inject on desktop/tablet/mobile.
    #[serde(default)]
    pub viewports: Vec<String>,
    #[serde(default)]
    pub timing: InjectTiming,
    /// Priority tiebreak — higher wins in the inject ordering.
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_enabled() -> bool {
    true
}

impl ResourceHintSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "fonts-preconnect".into(),
            host: "*".into(),
            kind: HintKind::Preconnect,
            url: "https://fonts.gstatic.com".into(),
            as_kind: AsKind::None,
            mime_type: None,
            crossorigin: true,
            integrity: None,
            fetch_priority: FetchPriority::Auto,
            media: None,
            viewports: vec![],
            timing: InjectTiming::OnParse,
            priority: 0,
            enabled: true,
            description: Some(
                "Default hint — preconnect fonts.gstatic.com at parse time.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host.is_empty() || self.host == "*" {
            return true;
        }
        crate::extension::glob_match_host(&self.host, host)
    }

    /// Render the `<link rel="...">` tag that expresses this hint.
    /// Returns None for Kind::EarlyHints (server-side only).
    #[must_use]
    pub fn render_link(&self) -> Option<String> {
        if matches!(self.kind, HintKind::EarlyHints) {
            return None;
        }
        let rel = match self.kind {
            HintKind::Preconnect => "preconnect",
            HintKind::DnsPrefetch => "dns-prefetch",
            HintKind::Preload => "preload",
            HintKind::Prefetch => "prefetch",
            HintKind::ModulePreload => "modulepreload",
            HintKind::EarlyHints => unreachable!(),
        };
        let mut out = format!(r#"<link rel="{rel}" href="{}""#, escape_attr(&self.url));
        if !matches!(self.as_kind, AsKind::None) {
            let as_str = match self.as_kind {
                AsKind::Fetch => "fetch",
                AsKind::Script => "script",
                AsKind::Style => "style",
                AsKind::Image => "image",
                AsKind::Font => "font",
                AsKind::Audio => "audio",
                AsKind::Video => "video",
                AsKind::Track => "track",
                AsKind::Document => "document",
                AsKind::Worker => "worker",
                AsKind::Object => "object",
                AsKind::Embed => "embed",
                AsKind::None => unreachable!(),
            };
            out.push_str(&format!(r#" as="{as_str}""#));
        }
        if let Some(mt) = &self.mime_type {
            out.push_str(&format!(r#" type="{}""#, escape_attr(mt)));
        }
        if self.crossorigin {
            out.push_str(r#" crossorigin"#);
        }
        if let Some(i) = &self.integrity {
            out.push_str(&format!(r#" integrity="{}""#, escape_attr(i)));
        }
        if let Some(m) = &self.media {
            out.push_str(&format!(r#" media="{}""#, escape_attr(m)));
        }
        let fp = match self.fetch_priority {
            FetchPriority::Auto => None,
            FetchPriority::Low => Some("low"),
            FetchPriority::High => Some("high"),
        };
        if let Some(fp) = fp {
            out.push_str(&format!(r#" fetchpriority="{fp}""#));
        }
        out.push('>');
        Some(out)
    }
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ResourceHintRegistry {
    specs: Vec<ResourceHintSpec>,
}

impl ResourceHintRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ResourceHintSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ResourceHintSpec>) {
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
    pub fn specs(&self) -> &[ResourceHintSpec] {
        &self.specs
    }

    /// All enabled hints applicable to `host`, sorted priority-desc.
    #[must_use]
    pub fn hints_for(&self, host: &str) -> Vec<&ResourceHintSpec> {
        let mut matches: Vec<_> = self
            .specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host))
            .collect();
        matches.sort_by(|a, b| b.priority.cmp(&a.priority));
        matches
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ResourceHintSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ResourceHintSpec>, String> {
    tatara_lisp::compile_typed::<ResourceHintSpec>(src)
        .map_err(|e| format!("failed to compile defresource-hint forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ResourceHintSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_preconnects_google_fonts() {
        let s = ResourceHintSpec::default_profile();
        assert_eq!(s.kind, HintKind::Preconnect);
        assert!(s.crossorigin);
        assert_eq!(s.timing, InjectTiming::OnParse);
    }

    #[test]
    fn render_preconnect_link() {
        let s = ResourceHintSpec::default_profile();
        let link = s.render_link().unwrap();
        assert!(link.contains(r#"rel="preconnect""#));
        assert!(link.contains(r#"href="https://fonts.gstatic.com""#));
        assert!(link.contains("crossorigin"));
    }

    #[test]
    fn render_preload_font_with_type_and_integrity() {
        let s = ResourceHintSpec {
            kind: HintKind::Preload,
            url: "/fonts/inter.woff2".into(),
            as_kind: AsKind::Font,
            mime_type: Some("font/woff2".into()),
            crossorigin: true,
            integrity: Some("sha384-abc".into()),
            fetch_priority: FetchPriority::High,
            ..ResourceHintSpec::default_profile()
        };
        let link = s.render_link().unwrap();
        assert!(link.contains(r#"rel="preload""#));
        assert!(link.contains(r#"as="font""#));
        assert!(link.contains(r#"type="font/woff2""#));
        assert!(link.contains("crossorigin"));
        assert!(link.contains(r#"integrity="sha384-abc""#));
        assert!(link.contains(r#"fetchpriority="high""#));
    }

    #[test]
    fn render_escapes_quotes_in_url() {
        let s = ResourceHintSpec {
            url: r#"https://ex.com/path"q=1"#.into(),
            ..ResourceHintSpec::default_profile()
        };
        let link = s.render_link().unwrap();
        assert!(link.contains("&quot;q=1"));
    }

    #[test]
    fn render_early_hints_returns_none() {
        let s = ResourceHintSpec {
            kind: HintKind::EarlyHints,
            ..ResourceHintSpec::default_profile()
        };
        assert!(s.render_link().is_none());
    }

    #[test]
    fn render_skips_crossorigin_when_false() {
        let s = ResourceHintSpec {
            crossorigin: false,
            ..ResourceHintSpec::default_profile()
        };
        let link = s.render_link().unwrap();
        assert!(!link.contains("crossorigin"));
    }

    #[test]
    fn render_includes_media_and_fetch_priority_low() {
        let s = ResourceHintSpec {
            kind: HintKind::Prefetch,
            url: "/foo".into(),
            media: Some("(min-width: 768px)".into()),
            fetch_priority: FetchPriority::Low,
            ..ResourceHintSpec::default_profile()
        };
        let link = s.render_link().unwrap();
        assert!(link.contains(r#"media="(min-width: 768px)""#));
        assert!(link.contains(r#"fetchpriority="low""#));
    }

    #[test]
    fn hints_for_orders_by_priority_desc() {
        let mut reg = ResourceHintRegistry::new();
        reg.insert(ResourceHintSpec {
            name: "lo".into(),
            priority: 0,
            ..ResourceHintSpec::default_profile()
        });
        reg.insert(ResourceHintSpec {
            name: "hi".into(),
            priority: 100,
            ..ResourceHintSpec::default_profile()
        });
        let list = reg.hints_for("example.com");
        assert_eq!(list[0].name, "hi");
        assert_eq!(list[1].name, "lo");
    }

    #[test]
    fn hints_for_respects_host_glob_and_enabled() {
        let mut reg = ResourceHintRegistry::new();
        reg.insert(ResourceHintSpec {
            name: "gh".into(),
            host: "*://*.github.com/*".into(),
            ..ResourceHintSpec::default_profile()
        });
        reg.insert(ResourceHintSpec {
            name: "off".into(),
            enabled: false,
            ..ResourceHintSpec::default_profile()
        });
        assert!(reg.hints_for("example.org").is_empty());
        assert_eq!(reg.hints_for("www.github.com").len(), 1);
    }

    #[test]
    fn kind_roundtrips_through_serde() {
        for k in [
            HintKind::Preconnect,
            HintKind::DnsPrefetch,
            HintKind::Preload,
            HintKind::Prefetch,
            HintKind::ModulePreload,
            HintKind::EarlyHints,
        ] {
            let s = ResourceHintSpec {
                kind: k,
                ..ResourceHintSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ResourceHintSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.kind, k);
        }
    }

    #[test]
    fn as_kind_and_fetch_priority_roundtrip_through_serde() {
        let s = ResourceHintSpec {
            kind: HintKind::Preload,
            as_kind: AsKind::Font,
            fetch_priority: FetchPriority::High,
            ..ResourceHintSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ResourceHintSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_kind, AsKind::Font);
        assert_eq!(back.fetch_priority, FetchPriority::High);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_resource_hint_form() {
        let src = r#"
            (defresource-hint :name "cdn-preconnect"
                              :host "*"
                              :kind "preconnect"
                              :url "https://cdn.example.com"
                              :crossorigin #t
                              :fetch-priority "high"
                              :timing "on-parse"
                              :priority 10)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.kind, HintKind::Preconnect);
        assert_eq!(s.fetch_priority, FetchPriority::High);
        assert!(s.crossorigin);
        assert_eq!(s.priority, 10);
    }
}
