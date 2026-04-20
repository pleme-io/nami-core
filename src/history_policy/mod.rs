//! `(defhistory-policy)` — browsing-history retention + privacy rules.
//!
//! Absorbs Chrome/Firefox/Safari history settings, Brave private
//! windows, Arc "private tabs", DuckDuckGo Fire Button. Declarative
//! control over which pages enter history, how long they live, and
//! what metadata is captured.
//!
//! ```lisp
//! (defhistory-policy :name          "strict"
//!                    :host          "*"
//!                    :retention     :session
//!                    :capture       (title url favicon visit-time)
//!                    :private-hosts ("*://*.bank.com/*" "*://*.health.gov/*")
//!                    :excluded-schemes ("chrome://" "about:"))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How long history entries survive.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Retention {
    /// No history recorded.
    Off,
    /// Cleared at browser close.
    Session,
    /// Kept for `retention_days` days.
    #[default]
    Days,
    /// Kept forever.
    Forever,
    /// Kept only if the user bookmarks / tags the page.
    OnlyIfBookmarked,
}

/// Metadata captured per entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureField {
    Title,
    Url,
    Favicon,
    /// Unix-seconds of last visit.
    VisitTime,
    /// Visit counter.
    VisitCount,
    /// Referrer URL.
    Referrer,
    /// Full-text snippet for local search.
    PageSnippet,
    /// Screenshot thumbnail.
    Screenshot,
    /// Source of the navigation — omnibox/link/reload/…
    NavigationType,
    /// Time spent on page.
    DwellTime,
}

/// Whether to surface entries in suggestions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    /// Surface everywhere — omnibox, history panel, sync.
    #[default]
    Full,
    /// Keep in local db but never surface in suggestions.
    LocalOnly,
    /// Store only; no suggestions, no local search surface.
    SuppressSuggestions,
    /// Not even stored (matches `Retention::Off` effect but scoped).
    None,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defhistory-policy"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPolicySpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub retention: Retention,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_capture")]
    pub capture: Vec<CaptureField>,
    #[serde(default)]
    pub visibility: Visibility,
    /// Hosts for which history is NEVER recorded (banks, health,
    /// porn, whatever the user defines as sensitive).
    #[serde(default)]
    pub private_hosts: Vec<String>,
    /// URL schemes excluded entirely from history.
    #[serde(default = "default_excluded_schemes")]
    pub excluded_schemes: Vec<String>,
    /// Don't record single-character typed navigations (avoids
    /// leaking intra-omnibox typing into history).
    #[serde(default = "default_filter_short_urls")]
    pub filter_short_typed_urls: bool,
    /// Minimum seconds on page before the entry is persisted
    /// (filter out mis-clicks).
    #[serde(default)]
    pub min_dwell_seconds: u32,
    /// Also delete matching Cache + cookies + storage when a history
    /// entry expires.
    #[serde(default)]
    pub cascade_delete_storage: bool,
    /// Include in cross-device sync ((defsync) :signal history)?
    #[serde(default = "default_sync")]
    pub sync: bool,
    /// Include in local full-text search index?
    #[serde(default = "default_search_indexed")]
    pub search_indexed: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_retention_days() -> u32 {
    90
}
fn default_capture() -> Vec<CaptureField> {
    vec![
        CaptureField::Title,
        CaptureField::Url,
        CaptureField::Favicon,
        CaptureField::VisitTime,
        CaptureField::VisitCount,
    ]
}
fn default_excluded_schemes() -> Vec<String> {
    vec![
        "chrome://".into(),
        "about:".into(),
        "view-source:".into(),
        "data:".into(),
    ]
}
fn default_filter_short_urls() -> bool {
    true
}
fn default_sync() -> bool {
    true
}
fn default_search_indexed() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl HistoryPolicySpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            retention: Retention::Days,
            retention_days: 90,
            capture: default_capture(),
            visibility: Visibility::Full,
            private_hosts: vec![],
            excluded_schemes: default_excluded_schemes(),
            filter_short_typed_urls: true,
            min_dwell_seconds: 0,
            cascade_delete_storage: false,
            sync: true,
            search_indexed: true,
            enabled: true,
            description: Some(
                "Default history — Days retention (90), full metadata, no private hosts.".into(),
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

    #[must_use]
    pub fn is_private_host(&self, host: &str) -> bool {
        self.private_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    #[must_use]
    pub fn is_excluded_scheme(&self, url: &str) -> bool {
        self.excluded_schemes.iter().any(|s| url.starts_with(s))
    }

    #[must_use]
    pub fn captures(&self, field: CaptureField) -> bool {
        self.capture.contains(&field)
    }

    /// Should this (host, url, dwell) be recorded?
    #[must_use]
    pub fn should_record(&self, host: &str, url: &str, dwell_seconds: u32) -> bool {
        if !self.enabled {
            return false;
        }
        if matches!(self.retention, Retention::Off) || matches!(self.visibility, Visibility::None)
        {
            return false;
        }
        if self.is_excluded_scheme(url) {
            return false;
        }
        if self.is_private_host(host) {
            return false;
        }
        if dwell_seconds < self.min_dwell_seconds {
            return false;
        }
        true
    }

    /// Effective retention seconds — 0 means session, u64::MAX means
    /// forever, else days * 86400. `OnlyIfBookmarked` returns u64::MAX
    /// because the expiry is bookmark-presence-gated not time-gated.
    #[must_use]
    pub fn retention_seconds(&self) -> u64 {
        match self.retention {
            Retention::Off | Retention::Session => 0,
            Retention::Days => u64::from(self.retention_days) * 86_400,
            Retention::Forever | Retention::OnlyIfBookmarked => u64::MAX,
        }
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct HistoryPolicyRegistry {
    specs: Vec<HistoryPolicySpec>,
}

impl HistoryPolicyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: HistoryPolicySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = HistoryPolicySpec>) {
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
    pub fn specs(&self) -> &[HistoryPolicySpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&HistoryPolicySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<HistoryPolicySpec>, String> {
    tatara_lisp::compile_typed::<HistoryPolicySpec>(src)
        .map_err(|e| format!("failed to compile defhistory-policy forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<HistoryPolicySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_captures_standard_fields() {
        let s = HistoryPolicySpec::default_profile();
        assert!(s.captures(CaptureField::Title));
        assert!(s.captures(CaptureField::Url));
        assert!(s.captures(CaptureField::VisitCount));
        assert!(!s.captures(CaptureField::Screenshot));
    }

    #[test]
    fn should_record_happy_path() {
        let s = HistoryPolicySpec::default_profile();
        assert!(s.should_record("example.com", "https://example.com/page", 10));
    }

    #[test]
    fn should_not_record_private_hosts() {
        let s = HistoryPolicySpec {
            private_hosts: vec!["*://*.bank.com/*".into()],
            ..HistoryPolicySpec::default_profile()
        };
        assert!(!s.should_record("my.bank.com", "https://my.bank.com/a", 30));
    }

    #[test]
    fn should_not_record_excluded_schemes() {
        let s = HistoryPolicySpec::default_profile();
        assert!(!s.should_record("", "about:blank", 30));
        assert!(!s.should_record("ex.com", "data:text/html,<p>", 30));
        assert!(!s.should_record("ex.com", "chrome://settings", 30));
    }

    #[test]
    fn should_not_record_when_retention_off() {
        let s = HistoryPolicySpec {
            retention: Retention::Off,
            ..HistoryPolicySpec::default_profile()
        };
        assert!(!s.should_record("example.com", "https://example.com", 30));
    }

    #[test]
    fn should_not_record_when_visibility_none() {
        let s = HistoryPolicySpec {
            visibility: Visibility::None,
            ..HistoryPolicySpec::default_profile()
        };
        assert!(!s.should_record("example.com", "https://example.com", 30));
    }

    #[test]
    fn should_record_respects_min_dwell() {
        let s = HistoryPolicySpec {
            min_dwell_seconds: 5,
            ..HistoryPolicySpec::default_profile()
        };
        assert!(!s.should_record("example.com", "https://example.com", 2));
        assert!(s.should_record("example.com", "https://example.com", 5));
    }

    #[test]
    fn should_not_record_when_disabled() {
        let s = HistoryPolicySpec {
            enabled: false,
            ..HistoryPolicySpec::default_profile()
        };
        assert!(!s.should_record("example.com", "https://example.com", 30));
    }

    #[test]
    fn retention_seconds_days_math() {
        let s = HistoryPolicySpec {
            retention: Retention::Days,
            retention_days: 30,
            ..HistoryPolicySpec::default_profile()
        };
        assert_eq!(s.retention_seconds(), 30 * 86_400);
    }

    #[test]
    fn retention_seconds_off_is_zero() {
        let s = HistoryPolicySpec {
            retention: Retention::Off,
            ..HistoryPolicySpec::default_profile()
        };
        assert_eq!(s.retention_seconds(), 0);
    }

    #[test]
    fn retention_seconds_forever_is_max() {
        let s = HistoryPolicySpec {
            retention: Retention::Forever,
            ..HistoryPolicySpec::default_profile()
        };
        assert_eq!(s.retention_seconds(), u64::MAX);
    }

    #[test]
    fn retention_seconds_only_if_bookmarked_is_max() {
        let s = HistoryPolicySpec {
            retention: Retention::OnlyIfBookmarked,
            ..HistoryPolicySpec::default_profile()
        };
        assert_eq!(s.retention_seconds(), u64::MAX);
    }

    #[test]
    fn retention_roundtrips_through_serde() {
        for r in [
            Retention::Off,
            Retention::Session,
            Retention::Days,
            Retention::Forever,
            Retention::OnlyIfBookmarked,
        ] {
            let s = HistoryPolicySpec {
                retention: r,
                ..HistoryPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: HistoryPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.retention, r);
        }
    }

    #[test]
    fn capture_field_roundtrips_through_serde() {
        let s = HistoryPolicySpec {
            capture: vec![
                CaptureField::Title,
                CaptureField::Url,
                CaptureField::Favicon,
                CaptureField::VisitTime,
                CaptureField::VisitCount,
                CaptureField::Referrer,
                CaptureField::PageSnippet,
                CaptureField::Screenshot,
                CaptureField::NavigationType,
                CaptureField::DwellTime,
            ],
            ..HistoryPolicySpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: HistoryPolicySpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.capture.len(), 10);
    }

    #[test]
    fn visibility_roundtrips_through_serde() {
        for v in [
            Visibility::Full,
            Visibility::LocalOnly,
            Visibility::SuppressSuggestions,
            Visibility::None,
        ] {
            let s = HistoryPolicySpec {
                visibility: v,
                ..HistoryPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: HistoryPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.visibility, v);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = HistoryPolicyRegistry::new();
        reg.insert(HistoryPolicySpec::default_profile());
        reg.insert(HistoryPolicySpec {
            name: "news-suppress".into(),
            host: "*://*.nytimes.com/*".into(),
            visibility: Visibility::SuppressSuggestions,
            ..HistoryPolicySpec::default_profile()
        });
        let ny = reg.resolve("www.nytimes.com").unwrap();
        assert_eq!(ny.visibility, Visibility::SuppressSuggestions);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_history_policy_form() {
        let src = r#"
            (defhistory-policy :name "strict"
                               :host "*"
                               :retention "days"
                               :retention-days 30
                               :capture ("title" "url" "favicon" "visit-time")
                               :visibility "local-only"
                               :private-hosts ("*://*.bank.com/*")
                               :excluded-schemes ("chrome://" "about:")
                               :filter-short-typed-urls #t
                               :min-dwell-seconds 3
                               :cascade-delete-storage #t
                               :sync #t
                               :search-indexed #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.retention, Retention::Days);
        assert_eq!(s.retention_days, 30);
        assert_eq!(s.visibility, Visibility::LocalOnly);
        assert_eq!(s.private_hosts.len(), 1);
        assert!(s.cascade_delete_storage);
    }
}
