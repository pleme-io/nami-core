//! `(defspace)` — Arc-style spaces (grouped tabs + per-space state).
//!
//! A *space* is a named, persistable collection of tabs plus the
//! chrome that wraps them — theme, homepage, pinned URLs, isolation
//! scope. Users switch between spaces the way other browsers switch
//! windows; unlike windows, spaces survive process death and carry
//! their own settings.
//!
//! Absorbs: Arc Spaces, Firefox Containers, Safari Profiles, Edge
//! Workspaces. Composes with existing DSLs — each space references
//! a theme name ([blackmatter::irodzuki]), an omnibox profile
//! ([`crate::omnibox`]), a bookmarks folder, and a zoom rule list.
//!
//! ```lisp
//! (defspace :name      "work"
//!           :title     "Work"
//!           :icon      "briefcase"
//!           :theme     "nord-dark"
//!           :homepage  "https://calendar.google.com"
//!           :tabs      ("https://mail.example.com" "https://docs.example.com")
//!           :pinned    ("https://mail.example.com")
//!           :bookmarks-folder "work"
//!           :isolated  #t)
//!
//! (defspace :name      "home"
//!           :title     "Home"
//!           :icon      "house"
//!           :theme     "gruvbox"
//!           :homepage  "https://news.ycombinator.com")
//! ```
//!
//! `isolated: true` signals that the space should own a separate
//! cookie jar + storage partition — wired via `(defstorage)`'s
//! namespace prefix and the `(defsecurity-policy)` cookie-partition
//! directive. This V1 encodes the intent; the enforcement arc lives
//! in namimado's fetch pipeline.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One space declaration.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defspace"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpaceSpec {
    /// Stable identifier — used as the storage-namespace prefix.
    pub name: String,
    /// Display title in the switcher UI.
    pub title: String,
    /// Optional icon name (iconic glyph set). Freeform — the chrome
    /// maps it to its glyph table.
    #[serde(default)]
    pub icon: Option<String>,
    /// Theme scheme name — resolved through irodzuki.
    #[serde(default)]
    pub theme: Option<String>,
    /// Home page opened when the space activates and has no tabs.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Seed URLs opened on first activation. Subsequent activations
    /// restore whatever tabs the user left open (see SessionStore).
    #[serde(default)]
    pub tabs: Vec<String>,
    /// URLs pinned in the sidebar (always visible, can't be closed
    /// via plain Cmd+W — require explicit unpin).
    #[serde(default)]
    pub pinned: Vec<String>,
    /// Bookmarks folder name — the bookmarks panel filters to this
    /// folder while the space is active.
    #[serde(default)]
    pub bookmarks_folder: Option<String>,
    /// Isolate storage / cookies / cache under this space's namespace.
    /// Equivalent to a Firefox Container.
    #[serde(default)]
    pub isolated: bool,
    /// Omnibox profile name — lets different spaces use different
    /// search providers + history sources.
    #[serde(default)]
    pub omnibox_profile: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Registry of declared spaces.
#[derive(Debug, Clone, Default)]
pub struct SpaceRegistry {
    specs: Vec<SpaceSpec>,
}

impl SpaceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SpaceSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SpaceSpec>) {
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
    pub fn specs(&self) -> &[SpaceSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SpaceSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.specs.iter().map(|s| s.name.clone()).collect()
    }
}

/// Mutable runtime state — which space is active.
#[derive(Debug, Clone, Default)]
pub struct SpaceState {
    active: Option<String>,
}

impl SpaceState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `name` as the active space. Caller is responsible for
    /// validating the name exists in the registry.
    pub fn activate(&mut self, name: impl Into<String>) {
        self.active = Some(name.into());
    }

    /// Clear the active space.
    pub fn deactivate(&mut self) {
        self.active = None;
    }

    #[must_use]
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SpaceSpec>, String> {
    tatara_lisp::compile_typed::<SpaceSpec>(src)
        .map_err(|e| format!("failed to compile defspace forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SpaceSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> SpaceSpec {
        SpaceSpec {
            name: name.into(),
            title: name.into(),
            icon: None,
            theme: None,
            homepage: None,
            tabs: vec![],
            pinned: vec![],
            bookmarks_folder: None,
            isolated: false,
            omnibox_profile: None,
            description: None,
        }
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = SpaceRegistry::new();
        reg.insert(sample("work"));
        reg.insert(SpaceSpec {
            theme: Some("nord".into()),
            ..sample("work")
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("work").unwrap().theme.as_deref(), Some("nord"));
    }

    #[test]
    fn registry_enumerates_names_in_insertion_order() {
        let mut reg = SpaceRegistry::new();
        reg.insert(sample("work"));
        reg.insert(sample("home"));
        reg.insert(sample("focus"));
        assert_eq!(reg.names(), vec!["work", "home", "focus"]);
    }

    #[test]
    fn space_state_activates_and_deactivates() {
        let mut s = SpaceState::new();
        assert!(s.active().is_none());
        s.activate("work");
        assert_eq!(s.active(), Some("work"));
        s.activate("home");
        assert_eq!(s.active(), Some("home"));
        s.deactivate();
        assert!(s.active().is_none());
    }

    #[test]
    fn isolated_default_is_false() {
        let s = sample("x");
        assert!(!s.isolated);
    }

    #[test]
    fn seed_tabs_and_pinned_round_trip() {
        let s = SpaceSpec {
            tabs: vec!["https://a".into(), "https://b".into()],
            pinned: vec!["https://a".into()],
            ..sample("x")
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SpaceSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tabs.len(), 2);
        assert_eq!(back.pinned.len(), 1);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_full_space_form() {
        let src = r#"
            (defspace :name      "work"
                      :title     "Work"
                      :icon      "briefcase"
                      :theme     "nord-dark"
                      :homepage  "https://calendar.google.com"
                      :tabs      ("https://mail.example.com" "https://docs.example.com")
                      :pinned    ("https://mail.example.com")
                      :bookmarks-folder "work"
                      :isolated  #t
                      :omnibox-profile "work-search")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "work");
        assert_eq!(s.title, "Work");
        assert_eq!(s.icon.as_deref(), Some("briefcase"));
        assert_eq!(s.theme.as_deref(), Some("nord-dark"));
        assert_eq!(s.homepage.as_deref(), Some("https://calendar.google.com"));
        assert_eq!(s.tabs.len(), 2);
        assert_eq!(s.pinned.len(), 1);
        assert_eq!(s.bookmarks_folder.as_deref(), Some("work"));
        assert!(s.isolated);
        assert_eq!(s.omnibox_profile.as_deref(), Some("work-search"));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_minimal_space_form() {
        let src = r#"(defspace :name "x" :title "X")"#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert!(specs[0].tabs.is_empty());
        assert!(!specs[0].isolated);
    }
}
