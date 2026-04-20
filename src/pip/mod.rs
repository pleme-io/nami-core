//! `(defpip)` — declarative picture-in-picture rules.
//!
//! Absorbs Safari/Chrome/Firefox PiP into the substrate. Each rule
//! scopes to a host + declares which video elements are promotable,
//! a default window corner, and whether PiP activates automatically
//! on scroll-off.
//!
//! ```lisp
//! (defpip :name        "default"
//!         :host        "*"
//!         :selectors   ("video")
//!         :position    :bottom-right
//!         :auto-activate #f)
//!
//! (defpip :name          "youtube"
//!         :host          "*://*.youtube.com/*"
//!         :selectors     (".html5-main-video" "video[src]")
//!         :position      :top-right
//!         :auto-activate #t
//!         :always-on-top #t)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// On-screen corner anchor for the PiP window.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PipPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Caller remembers the last position across sessions.
    Remembered,
}

impl Default for PipPosition {
    fn default() -> Self {
        Self::BottomRight
    }
}

/// One declarative PiP rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defpip"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PipSpec {
    pub name: String,
    /// Host glob. `"*"` = everywhere.
    #[serde(default = "default_host")]
    pub host: String,
    /// CSS selectors whose matches are promotable to PiP.
    /// Bare `"video"` is the universal default.
    #[serde(default = "default_selectors")]
    pub selectors: Vec<String>,
    /// Window corner at open time.
    #[serde(default)]
    pub position: PipPosition,
    /// When true, auto-enter PiP as soon as the user scrolls the
    /// video out of the viewport. Chrome + Safari default behavior.
    #[serde(default)]
    pub auto_activate: bool,
    /// Request the windowing system keep the PiP layer above other
    /// app windows (platform-dependent).
    #[serde(default = "default_always_on_top")]
    pub always_on_top: bool,
    /// Minimum viewport-relative size (fraction of the viewport's
    /// shorter axis). Default 0.25 — smaller than Chrome's, larger
    /// than iOS's.
    #[serde(default = "default_min_size")]
    pub min_size: f32,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_selectors() -> Vec<String> {
    vec!["video".into()]
}
fn default_always_on_top() -> bool {
    true
}
fn default_min_size() -> f32 {
    0.25
}

impl PipSpec {
    /// Sensible default — any `<video>`, bottom-right, on-top, no auto.
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            selectors: default_selectors(),
            position: PipPosition::BottomRight,
            auto_activate: false,
            always_on_top: true,
            min_size: default_min_size(),
            description: Some("Default PiP behavior for any <video>.".into()),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host.is_empty() || self.host == "*" {
            return true;
        }
        crate::extension::glob_match_host(&self.host, host)
    }
}

/// Registry — host-specific wins over wildcard.
#[derive(Debug, Clone, Default)]
pub struct PipRegistry {
    specs: Vec<PipSpec>,
}

impl PipRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PipSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PipSpec>) {
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
    pub fn specs(&self) -> &[PipSpec] {
        &self.specs
    }

    /// Most-specific host match wins (non-`"*"` beats wildcard).
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&PipSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PipSpec>, String> {
    tatara_lisp::compile_typed::<PipSpec>(src)
        .map_err(|e| format!("failed to compile defpip forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PipSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_matches_every_host() {
        let s = PipSpec::default_profile();
        assert!(s.matches_host("example.com"));
        assert!(s.matches_host(""));
    }

    #[test]
    fn glob_matches_subdomain() {
        let s = PipSpec {
            host: "*://*.youtube.com/*".into(),
            ..PipSpec::default_profile()
        };
        assert!(s.matches_host("www.youtube.com"));
        assert!(!s.matches_host("youtube.com.evil.com"));
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = PipRegistry::new();
        reg.insert(PipSpec::default_profile());
        reg.insert(PipSpec {
            auto_activate: true,
            ..PipSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert!(reg.specs()[0].auto_activate);
    }

    #[test]
    fn resolve_prefers_specific_host() {
        let mut reg = PipRegistry::new();
        reg.insert(PipSpec::default_profile());
        reg.insert(PipSpec {
            name: "yt".into(),
            host: "*://*.youtube.com/*".into(),
            auto_activate: true,
            position: PipPosition::TopRight,
            ..PipSpec::default_profile()
        });
        let yt = reg.resolve("www.youtube.com").unwrap();
        assert_eq!(yt.name, "yt");
        assert!(yt.auto_activate);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn default_selectors_contains_video() {
        let s = PipSpec::default_profile();
        assert!(s.selectors.iter().any(|x| x == "video"));
    }

    #[test]
    fn default_position_is_bottom_right() {
        assert_eq!(PipSpec::default_profile().position, PipPosition::BottomRight);
    }

    #[test]
    fn remembered_position_roundtrips_through_serde() {
        let s = PipSpec {
            position: PipPosition::Remembered,
            ..PipSpec::default_profile()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: PipSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.position, PipPosition::Remembered);
    }

    #[test]
    fn min_size_default_is_025() {
        let s = PipSpec::default_profile();
        assert!((s.min_size - 0.25).abs() < f32::EPSILON);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_pip_form() {
        let src = r#"
            (defpip :name      "youtube"
                    :host      "*://*.youtube.com/*"
                    :selectors (".html5-main-video" "video[src]")
                    :position  "top-right"
                    :auto-activate #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "youtube");
        assert_eq!(s.selectors.len(), 2);
        assert_eq!(s.position, PipPosition::TopRight);
        assert!(s.auto_activate);
    }
}
