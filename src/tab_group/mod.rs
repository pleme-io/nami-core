//! `(deftab-group)` — declarative tab groups.
//!
//! Absorbs Chrome Tab Groups, Firefox containers, Vivaldi Tab Stacks,
//! Arc Spaces / Split tabs, Edge Collections, Safari Tab Groups.
//! Each profile declares a group by name + color + match rule; tabs
//! matching the rule auto-enter the group.
//!
//! ```lisp
//! (deftab-group :name       "work"
//!               :color      "blue"
//!               :hosts      ("*://*.github.com/*" "*://*.linear.app/*")
//!               :collapsed  #t
//!               :pinned     #f
//!               :icon       "briefcase"
//!               :isolation  :per-profile)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Tab-group color (Chrome uses a fixed palette; we absorb that
/// surface verbatim plus a `Custom` escape hatch).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GroupColor {
    Grey,
    #[default]
    Blue,
    Red,
    Yellow,
    Green,
    Pink,
    Purple,
    Cyan,
    Orange,
    Custom,
}

/// How the group is isolated from other groups.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GroupIsolation {
    /// Shared cookies/storage with all other tabs.
    #[default]
    None,
    /// Dedicated cookie jar + storage per profile (like Firefox
    /// Containers).
    PerProfile,
    /// Dedicated ephemeral cookie jar per window.
    PerWindow,
    /// Fully ephemeral — cleared on group close.
    Ephemeral,
}

/// Tab-group profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deftab-group"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabGroupSpec {
    pub name: String,
    #[serde(default)]
    pub color: GroupColor,
    /// Optional custom hex color when `color = :custom`.
    #[serde(default)]
    pub custom_color: Option<String>,
    /// Host-glob patterns. Any tab whose URL host matches any pattern
    /// is a candidate member of this group.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Collapsed state in the tab strip.
    #[serde(default)]
    pub collapsed: bool,
    /// Whether member tabs are pinned (small, left-of-strip).
    #[serde(default)]
    pub pinned: bool,
    /// Icon glyph name (freeform — the desktop chrome picks).
    #[serde(default)]
    pub icon: Option<String>,
    /// Isolation scope for cookies/storage.
    #[serde(default)]
    pub isolation: GroupIsolation,
    /// Maximum tabs allowed in the group; extras open in the default
    /// window instead. 0 = unlimited.
    #[serde(default)]
    pub max_tabs: u32,
    /// Auto-close the group when the last tab closes (vs keeping the
    /// empty-group header visible).
    #[serde(default = "default_close_when_empty")]
    pub close_when_empty: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_close_when_empty() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl TabGroupSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            color: GroupColor::Blue,
            custom_color: None,
            hosts: vec![],
            collapsed: false,
            pinned: false,
            icon: None,
            isolation: GroupIsolation::None,
            max_tabs: 0,
            close_when_empty: true,
            enabled: true,
            description: Some("Default tab group — blue, no isolation, no auto-matching.".into()),
        }
    }

    /// Would `host` be auto-placed in this group?
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        self.hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Does the group have an open slot?
    #[must_use]
    pub fn can_accept(&self, current_tabs: u32) -> bool {
        self.max_tabs == 0 || current_tabs < self.max_tabs
    }

    /// Resolve final color — returns `custom_color` when
    /// `color == Custom`, else the canonical palette hex.
    #[must_use]
    pub fn resolved_color(&self) -> String {
        match self.color {
            GroupColor::Custom => self
                .custom_color
                .clone()
                .unwrap_or_else(|| "#3b82f6".into()),
            _ => palette_hex(self.color).into(),
        }
    }
}

fn palette_hex(c: GroupColor) -> &'static str {
    match c {
        GroupColor::Grey => "#9aa0a6",
        GroupColor::Blue => "#1a73e8",
        GroupColor::Red => "#d93025",
        GroupColor::Yellow => "#fbbc04",
        GroupColor::Green => "#188038",
        GroupColor::Pink => "#d01884",
        GroupColor::Purple => "#a142f4",
        GroupColor::Cyan => "#007b83",
        GroupColor::Orange => "#fa7b17",
        GroupColor::Custom => "#3b82f6",
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct TabGroupRegistry {
    specs: Vec<TabGroupSpec>,
}

impl TabGroupRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: TabGroupSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = TabGroupSpec>) {
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
    pub fn specs(&self) -> &[TabGroupSpec] {
        &self.specs
    }

    /// Find the first enabled group whose hosts cover `host`. Groups
    /// are checked in insertion order so authors can express priority.
    #[must_use]
    pub fn group_for_host(&self, host: &str) -> Option<&TabGroupSpec> {
        self.specs
            .iter()
            .find(|s| s.enabled && s.matches_host(host))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<TabGroupSpec>, String> {
    tatara_lisp::compile_typed::<TabGroupSpec>(src)
        .map_err(|e| format!("failed to compile deftab-group forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<TabGroupSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_blue_no_hosts() {
        let s = TabGroupSpec::default_profile();
        assert_eq!(s.color, GroupColor::Blue);
        assert!(s.hosts.is_empty());
    }

    #[test]
    fn matches_host_uses_webextension_glob() {
        let s = TabGroupSpec {
            hosts: vec!["*://*.github.com/*".into(), "*://*.linear.app/*".into()],
            ..TabGroupSpec::default_profile()
        };
        assert!(s.matches_host("www.github.com"));
        assert!(s.matches_host("app.linear.app"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn resolved_color_returns_palette() {
        let s = TabGroupSpec {
            color: GroupColor::Red,
            ..TabGroupSpec::default_profile()
        };
        assert_eq!(s.resolved_color(), "#d93025");
    }

    #[test]
    fn resolved_color_uses_custom_when_set() {
        let s = TabGroupSpec {
            color: GroupColor::Custom,
            custom_color: Some("#abcdef".into()),
            ..TabGroupSpec::default_profile()
        };
        assert_eq!(s.resolved_color(), "#abcdef");
    }

    #[test]
    fn custom_color_with_no_hex_falls_back() {
        let s = TabGroupSpec {
            color: GroupColor::Custom,
            custom_color: None,
            ..TabGroupSpec::default_profile()
        };
        assert_eq!(s.resolved_color(), "#3b82f6");
    }

    #[test]
    fn can_accept_respects_cap() {
        let capped = TabGroupSpec {
            max_tabs: 3,
            ..TabGroupSpec::default_profile()
        };
        assert!(capped.can_accept(0));
        assert!(capped.can_accept(2));
        assert!(!capped.can_accept(3));

        let unlimited = TabGroupSpec {
            max_tabs: 0,
            ..TabGroupSpec::default_profile()
        };
        assert!(unlimited.can_accept(999));
    }

    #[test]
    fn isolation_roundtrips_through_serde() {
        for iso in [
            GroupIsolation::None,
            GroupIsolation::PerProfile,
            GroupIsolation::PerWindow,
            GroupIsolation::Ephemeral,
        ] {
            let s = TabGroupSpec {
                isolation: iso,
                ..TabGroupSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TabGroupSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.isolation, iso);
        }
    }

    #[test]
    fn color_roundtrips_through_serde() {
        for c in [
            GroupColor::Grey,
            GroupColor::Blue,
            GroupColor::Red,
            GroupColor::Yellow,
            GroupColor::Green,
            GroupColor::Pink,
            GroupColor::Purple,
            GroupColor::Cyan,
            GroupColor::Orange,
            GroupColor::Custom,
        ] {
            let s = TabGroupSpec {
                color: c,
                ..TabGroupSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TabGroupSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.color, c);
        }
    }

    #[test]
    fn registry_group_for_host_returns_first_match() {
        let mut reg = TabGroupRegistry::new();
        reg.insert(TabGroupSpec {
            name: "work".into(),
            hosts: vec!["*://*.github.com/*".into()],
            ..TabGroupSpec::default_profile()
        });
        reg.insert(TabGroupSpec {
            name: "dev".into(),
            hosts: vec!["*://*.github.com/*".into()],
            ..TabGroupSpec::default_profile()
        });
        // First-in wins (author priority).
        assert_eq!(reg.group_for_host("www.github.com").unwrap().name, "work");
    }

    #[test]
    fn disabled_groups_never_match() {
        let mut reg = TabGroupRegistry::new();
        reg.insert(TabGroupSpec {
            enabled: false,
            hosts: vec!["*://*.github.com/*".into()],
            ..TabGroupSpec::default_profile()
        });
        assert!(reg.group_for_host("www.github.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_tab_group_form() {
        let src = r#"
            (deftab-group :name "work"
                          :color "blue"
                          :hosts ("*://*.github.com/*" "*://*.linear.app/*")
                          :collapsed #t
                          :pinned #f
                          :icon "briefcase"
                          :isolation "per-profile"
                          :max-tabs 50)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "work");
        assert_eq!(s.color, GroupColor::Blue);
        assert_eq!(s.isolation, GroupIsolation::PerProfile);
        assert_eq!(s.hosts.len(), 2);
        assert!(s.collapsed);
        assert_eq!(s.max_tabs, 50);
    }
}
