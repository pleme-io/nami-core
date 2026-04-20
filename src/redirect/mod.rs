//! `(defredirect)` — privacy-frontend URL rewriting.
//!
//! Absorbs LibRedirect, Privacy Redirect, and the ecosystem of
//! "privacy frontends": YouTube → Invidious / Piped, Twitter → Nitter,
//! Reddit → Libreddit / Redlib, Wikipedia → Wikiless, etc. Each spec
//! names a source host glob and a list of mirror origins; the engine
//! rewrites matching URLs to a chosen mirror, preserving the path +
//! query so deep links survive.
//!
//! ```lisp
//! (defredirect :name     "youtube"
//!              :from     "*://*.youtube.com/*"
//!              :mirrors  ("https://invidio.us" "https://yewtu.be")
//!              :strategy :round-robin)
//!
//! (defredirect :name     "twitter"
//!              :from     "*://twitter.com/*"
//!              :mirrors  ("https://nitter.net" "https://nitter.42l.fr")
//!              :strategy :random)
//!
//! (defredirect :name     "reddit"
//!              :from     "*://*.reddit.com/*"
//!              :mirrors  ("https://redlib.example.com")
//!              :strategy :priority)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Strategy for picking a mirror when multiple are declared.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RotationStrategy {
    /// Always pick the first mirror. Fall through on failure is a
    /// fetch-layer concern.
    Priority,
    /// Round-robin — consumer tracks an index alongside the spec.
    RoundRobin,
    /// Random pick per navigate (uses caller-provided RNG).
    Random,
}

impl Default for RotationStrategy {
    fn default() -> Self {
        Self::Priority
    }
}

/// One privacy-frontend redirect rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defredirect"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RedirectSpec {
    pub name: String,
    /// Source host glob. WebExtensions-style; reuses the shared matcher.
    pub from: String,
    /// Mirror origins (no trailing slash). First = highest priority.
    pub mirrors: Vec<String>,
    #[serde(default)]
    pub strategy: RotationStrategy,
    /// Runtime toggle.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl RedirectSpec {
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.from.is_empty() {
            return false;
        }
        // Bare "*.host" suffix match.
        if self.from.starts_with('*')
            && self.from.len() > 1
            && self.from.as_bytes()[1] == b'.'
            && !self.from[1..].contains(|c: char| c == '*' || c == '/' || c == ':')
        {
            return host.ends_with(&self.from[1..]);
        }
        crate::extension::glob_match_host(&self.from, host)
    }

    /// Rewrite `input_url` using the `index`-th mirror (modulo count).
    /// Preserves path + query. Returns None when the host doesn't
    /// match or no mirrors are declared.
    #[must_use]
    pub fn rewrite(&self, input_url: &str, index: usize) -> Option<String> {
        if !self.enabled || self.mirrors.is_empty() {
            return None;
        }
        let parsed = url::Url::parse(input_url).ok()?;
        if !self.matches_host(parsed.host_str().unwrap_or("")) {
            return None;
        }
        let target = self.mirrors[index % self.mirrors.len()].trim_end_matches('/');
        let path_and_query = {
            let mut s = parsed.path().to_owned();
            if let Some(q) = parsed.query() {
                s.push('?');
                s.push_str(q);
            }
            if let Some(f) = parsed.fragment() {
                s.push('#');
                s.push_str(f);
            }
            s
        };
        Some(format!("{target}{path_and_query}"))
    }
}

/// Registry keyed by name. Rotation counters live outside the
/// registry so the struct stays cheaply cloneable.
#[derive(Debug, Clone, Default)]
pub struct RedirectRegistry {
    specs: Vec<RedirectSpec>,
}

impl RedirectRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: RedirectSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = RedirectSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

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
    pub fn specs(&self) -> &[RedirectSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&RedirectSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// First enabled rule whose `from` matches `host`.
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&RedirectSpec> {
        self.specs.iter().find(|s| s.enabled && s.matches_host(host))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<RedirectSpec>, String> {
    tatara_lisp::compile_typed::<RedirectSpec>(src)
        .map_err(|e| format!("failed to compile defredirect forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<RedirectSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, from: &str, mirrors: &[&str]) -> RedirectSpec {
        RedirectSpec {
            name: name.into(),
            from: from.into(),
            mirrors: mirrors.iter().map(|s| (*s).into()).collect(),
            strategy: RotationStrategy::Priority,
            enabled: true,
            description: None,
        }
    }

    #[test]
    fn matches_host_glob() {
        let s = sample("yt", "*://*.youtube.com/*", &["https://invidio.us"]);
        assert!(s.matches_host("www.youtube.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn matches_host_bare_star_suffix() {
        let s = sample("onion", "*.onion", &["https://tor-frontend.example"]);
        assert!(s.matches_host("abc.onion"));
        assert!(!s.matches_host("example.com"));
    }

    #[test]
    fn rewrite_preserves_path_and_query() {
        let s = sample("yt", "*://*.youtube.com/*", &["https://invidio.us"]);
        let got = s
            .rewrite("https://www.youtube.com/watch?v=abc123", 0)
            .unwrap();
        assert_eq!(got, "https://invidio.us/watch?v=abc123");
    }

    #[test]
    fn rewrite_preserves_fragment() {
        let s = sample("tw", "*://twitter.com/*", &["https://nitter.net"]);
        let got = s
            .rewrite("https://twitter.com/user/status/1#photo", 0)
            .unwrap();
        assert_eq!(got, "https://nitter.net/user/status/1#photo");
    }

    #[test]
    fn rewrite_returns_none_on_host_mismatch() {
        let s = sample("yt", "*://*.youtube.com/*", &["https://invidio.us"]);
        assert!(s.rewrite("https://example.com/", 0).is_none());
    }

    #[test]
    fn rewrite_returns_none_when_disabled() {
        let mut s = sample("yt", "*://*.youtube.com/*", &["https://invidio.us"]);
        s.enabled = false;
        assert!(s.rewrite("https://www.youtube.com/", 0).is_none());
    }

    #[test]
    fn rewrite_returns_none_with_no_mirrors() {
        let s = sample("yt", "*://*.youtube.com/*", &[]);
        assert!(s.rewrite("https://www.youtube.com/", 0).is_none());
    }

    #[test]
    fn rewrite_rotates_by_index_modulo() {
        let s = sample(
            "yt",
            "*://*.youtube.com/*",
            &["https://a.example", "https://b.example"],
        );
        assert_eq!(
            s.rewrite("https://www.youtube.com/x", 0).unwrap(),
            "https://a.example/x"
        );
        assert_eq!(
            s.rewrite("https://www.youtube.com/x", 1).unwrap(),
            "https://b.example/x"
        );
        // Wraps.
        assert_eq!(
            s.rewrite("https://www.youtube.com/x", 2).unwrap(),
            "https://a.example/x"
        );
    }

    #[test]
    fn rewrite_strips_trailing_slash_from_mirror() {
        let s = sample(
            "yt",
            "*://*.youtube.com/*",
            &["https://invidio.us/"],
        );
        let got = s.rewrite("https://www.youtube.com/x", 0).unwrap();
        assert_eq!(got, "https://invidio.us/x");
    }

    #[test]
    fn registry_dedupes_by_name_and_resolves_by_host() {
        let mut reg = RedirectRegistry::new();
        reg.insert(sample(
            "yt",
            "*://*.youtube.com/*",
            &["https://invidio.us"],
        ));
        reg.insert(sample("tw", "*://twitter.com/*", &["https://nitter.net"]));
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.resolve("www.youtube.com").unwrap().name, "yt");
        assert!(reg.resolve("example.com").is_none());
    }

    #[test]
    fn resolve_excludes_disabled() {
        let mut reg = RedirectRegistry::new();
        reg.insert(sample(
            "yt",
            "*://*.youtube.com/*",
            &["https://invidio.us"],
        ));
        reg.set_enabled("yt", false);
        assert!(reg.resolve("www.youtube.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_redirect_form() {
        let src = r#"
            (defredirect :name     "youtube"
                         :from     "*://*.youtube.com/*"
                         :mirrors  ("https://invidio.us" "https://yewtu.be")
                         :strategy "round-robin")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "youtube");
        assert_eq!(s.mirrors.len(), 2);
        assert_eq!(s.strategy, RotationStrategy::RoundRobin);
    }
}
