//! `(deftab-macro)` — atomic multi-tab transformations.
//!
//! **Novel** — browsers have "close other tabs" / "close tabs to the
//! right" / "mute all" as scattered one-off menu items. None give
//! authors a NAMED, declarative, atomic primitive that says "do X
//! to every tab matching Y". This DSL does. It composes with
//! (deftab-group), (deftab-hibernate), and (defnavigation-intent).
//!
//! ```lisp
//! (deftab-macro :name    "archive-news"
//!               :trigger :command
//!               :match   (host-glob "*://*.news.com/*"
//!                         created-before-days 7
//!                         not-pinned)
//!               :action  :move-to-group
//!               :target-group "archive"
//!               :atomic   #t
//!               :dry-run  #f
//!               :max-tabs 50)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How the macro is invoked.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// Invoked by name from the command palette / (defcommand).
    #[default]
    Command,
    /// Bound to a global hotkey (chord lives elsewhere, in (defbind)).
    Hotkey,
    /// Omnibox shortcut (user types the macro name).
    Omnibox,
    /// Fire automatically when the matcher first becomes true.
    AutoMatch,
    /// Fire on a fixed timer interval.
    Periodic,
}

/// Which tabs a macro touches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Matcher {
    /// Host-glob filter. Empty = any host.
    #[serde(default)]
    pub host_globs: Vec<String>,
    /// Title substring filter (case-insensitive).
    #[serde(default)]
    pub title_contains: Option<String>,
    /// URL-path regex (pre-compiled at match time).
    #[serde(default)]
    pub url_regex: Option<String>,
    /// Only pinned tabs.
    #[serde(default)]
    pub pinned_only: bool,
    /// Exclude pinned tabs.
    #[serde(default)]
    pub not_pinned: bool,
    /// Only tabs in a specific (deftab-group) by name.
    #[serde(default)]
    pub in_group: Option<String>,
    /// Only tabs older than this (days since last user visit).
    #[serde(default)]
    pub created_before_days: Option<u32>,
    /// Only tabs with audio currently playing.
    #[serde(default)]
    pub playing_audio: bool,
    /// Only tabs without audio playing.
    #[serde(default)]
    pub silent: bool,
    /// Exclude the currently-focused tab.
    #[serde(default)]
    pub exclude_active: bool,
}

/// What the macro does to the matched set.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Close every matched tab.
    #[default]
    Close,
    /// Reload every matched tab.
    Reload,
    /// Duplicate every matched tab.
    Duplicate,
    /// Move into `target_group` (a (deftab-group) name).
    MoveToGroup,
    /// Pin every matched tab.
    Pin,
    /// Unpin every matched tab.
    Unpin,
    /// Mute every matched tab.
    Mute,
    /// Unmute.
    Unmute,
    /// Hibernate (hand to (deftab-hibernate)'s policy).
    Hibernate,
    /// Discard (drop DOM, keep tab strip entry).
    Discard,
    /// Rewrite every URL through a template (`{host}`, `{path}`, `{query}` tokens).
    RewriteUrl,
    /// Evaluate a JS snippet in each tab via (defjs-runtime).
    EvalScript,
    /// Bookmark every matched tab.
    BookmarkAll,
    /// Open a single overview page listing the matched URLs.
    Overview,
}

/// Action-specific payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActionPayload {
    /// (deftab-group) name for MoveToGroup.
    #[serde(default)]
    pub target_group: Option<String>,
    /// URL template for RewriteUrl (tokens: `{host}`, `{path}`, `{query}`, `{full}`).
    #[serde(default)]
    pub url_template: Option<String>,
    /// JS source for EvalScript.
    #[serde(default)]
    pub script: Option<String>,
    /// (defjs-runtime) profile name when EvalScript is used. Empty =
    /// system default.
    #[serde(default)]
    pub runtime: Option<String>,
    /// Named bookmark folder for BookmarkAll.
    #[serde(default)]
    pub bookmark_folder: Option<String>,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deftab-macro"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabMacroSpec {
    pub name: String,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default)]
    pub match_rule: Matcher,
    #[serde(default)]
    pub action: Action,
    #[serde(default)]
    pub payload: ActionPayload,
    /// All-or-nothing execution — if any tab fails, none are touched.
    #[serde(default = "default_atomic")]
    pub atomic: bool,
    /// Dry-run mode — log intended changes but don't apply.
    #[serde(default)]
    pub dry_run: bool,
    /// Max tabs processed per invocation (safety ceiling).
    #[serde(default = "default_max_tabs")]
    pub max_tabs: u32,
    /// Interval between Periodic firings (seconds).
    #[serde(default)]
    pub interval_seconds: u32,
    /// Show a confirmation toast before destructive actions
    /// (Close/Discard/Hibernate/RewriteUrl).
    #[serde(default = "default_confirm")]
    pub confirm_destructive: bool,
    /// Hotkey chord when trigger = Hotkey (e.g. "Cmd+Shift+A").
    #[serde(default)]
    pub hotkey: Option<String>,
    /// Omnibox alias when trigger = Omnibox.
    #[serde(default)]
    pub omnibox_alias: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_atomic() -> bool {
    true
}
fn default_max_tabs() -> u32 {
    100
}
fn default_confirm() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

/// Runtime tab info the matcher evaluates against. Fed in by the
/// caller.
#[derive(Debug, Clone, Default)]
pub struct TabView<'a> {
    pub host: &'a str,
    pub title: &'a str,
    pub url: &'a str,
    pub pinned: bool,
    pub group: Option<&'a str>,
    pub age_days: u32,
    pub playing_audio: bool,
    pub active: bool,
}

impl TabMacroSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "close-idle".into(),
            trigger: Trigger::Command,
            match_rule: Matcher {
                not_pinned: true,
                silent: true,
                created_before_days: Some(7),
                exclude_active: true,
                ..Matcher::default()
            },
            action: Action::Close,
            payload: ActionPayload::default(),
            atomic: true,
            dry_run: false,
            max_tabs: 100,
            interval_seconds: 0,
            confirm_destructive: true,
            hotkey: None,
            omnibox_alias: None,
            enabled: false, // destructive-capable; opt-in only.
            description: Some(
                "Sample macro — close silent, idle-7-day, non-active, non-pinned tabs. DISABLED by default.".into(),
            ),
        }
    }

    /// Is this action destructive (would close / discard / rewrite
    /// something the user can't trivially undo)?
    #[must_use]
    pub fn is_destructive(&self) -> bool {
        matches!(
            self.action,
            Action::Close
                | Action::Discard
                | Action::Hibernate
                | Action::RewriteUrl
                | Action::EvalScript
        )
    }

    /// Does `tab` satisfy the matcher?
    #[must_use]
    pub fn matches(&self, tab: &TabView<'_>) -> bool {
        let m = &self.match_rule;
        if m.exclude_active && tab.active {
            return false;
        }
        if m.pinned_only && !tab.pinned {
            return false;
        }
        if m.not_pinned && tab.pinned {
            return false;
        }
        if m.playing_audio && !tab.playing_audio {
            return false;
        }
        if m.silent && tab.playing_audio {
            return false;
        }
        if let Some(group) = m.in_group.as_deref() {
            match tab.group {
                Some(g) if g == group => {}
                _ => return false,
            }
        }
        if let Some(limit) = m.created_before_days {
            if tab.age_days < limit {
                return false;
            }
        }
        if !m.host_globs.is_empty()
            && !m
                .host_globs
                .iter()
                .any(|pat| crate::extension::glob_match_host(pat, tab.host))
        {
            return false;
        }
        if let Some(needle) = &m.title_contains {
            if !tab.title.to_lowercase().contains(&needle.to_lowercase()) {
                return false;
            }
        }
        true
    }

    /// Apply `matches` across a tab list with the max-tabs cap.
    /// Returned indices are preserved in the input order.
    #[must_use]
    pub fn matched_indices(&self, tabs: &[TabView<'_>]) -> Vec<usize> {
        let mut out: Vec<usize> = tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| self.matches(t))
            .map(|(i, _)| i)
            .collect();
        if self.max_tabs != 0 && out.len() > self.max_tabs as usize {
            out.truncate(self.max_tabs as usize);
        }
        out
    }

    /// Render a URL through the `url_template` — substitutes `{host}`,
    /// `{path}`, `{query}`, `{full}` tokens.
    #[must_use]
    pub fn render_rewrite(&self, full_url: &str, host: &str, path: &str, query: &str) -> String {
        match &self.payload.url_template {
            Some(t) => t
                .replace("{host}", host)
                .replace("{path}", path)
                .replace("{query}", query)
                .replace("{full}", full_url),
            None => full_url.to_owned(),
        }
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct TabMacroRegistry {
    specs: Vec<TabMacroSpec>,
}

impl TabMacroRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: TabMacroSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = TabMacroSpec>) {
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
    pub fn specs(&self) -> &[TabMacroSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&TabMacroSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    #[must_use]
    pub fn by_trigger(&self, trigger: Trigger) -> Vec<&TabMacroSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.trigger == trigger)
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<TabMacroSpec>, String> {
    tatara_lisp::compile_typed::<TabMacroSpec>(src)
        .map_err(|e| format!("failed to compile deftab-macro forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<TabMacroSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab<'a>(host: &'a str) -> TabView<'a> {
        TabView {
            host,
            title: "title",
            url: "https://example.com/",
            pinned: false,
            group: None,
            age_days: 0,
            playing_audio: false,
            active: false,
        }
    }

    #[test]
    fn default_profile_is_destructive_but_disabled() {
        let s = TabMacroSpec::default_profile();
        assert_eq!(s.action, Action::Close);
        assert!(s.is_destructive());
        assert!(!s.enabled);
    }

    #[test]
    fn is_destructive_covers_risky_actions() {
        for a in [
            Action::Close,
            Action::Discard,
            Action::Hibernate,
            Action::RewriteUrl,
            Action::EvalScript,
        ] {
            let s = TabMacroSpec {
                action: a,
                ..TabMacroSpec::default_profile()
            };
            assert!(s.is_destructive(), "{a:?} should be destructive");
        }
    }

    #[test]
    fn is_destructive_false_for_read_only_actions() {
        for a in [
            Action::Reload,
            Action::Duplicate,
            Action::MoveToGroup,
            Action::Pin,
            Action::Unpin,
            Action::Mute,
            Action::Unmute,
            Action::BookmarkAll,
            Action::Overview,
        ] {
            let s = TabMacroSpec {
                action: a,
                ..TabMacroSpec::default_profile()
            };
            assert!(!s.is_destructive(), "{a:?} shouldn't be destructive");
        }
    }

    #[test]
    fn matcher_host_glob() {
        let s = TabMacroSpec {
            match_rule: Matcher {
                host_globs: vec!["*://*.news.com/*".into()],
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        assert!(s.matches(&tab("www.news.com")));
        assert!(!s.matches(&tab("example.com")));
    }

    #[test]
    fn matcher_pinned_only_and_not_pinned_exclusive() {
        let s = TabMacroSpec {
            match_rule: Matcher {
                pinned_only: true,
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        let mut t = tab("x.com");
        t.pinned = false;
        assert!(!s.matches(&t));
        t.pinned = true;
        assert!(s.matches(&t));

        let s2 = TabMacroSpec {
            match_rule: Matcher {
                not_pinned: true,
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        t.pinned = false;
        assert!(s2.matches(&t));
        t.pinned = true;
        assert!(!s2.matches(&t));
    }

    #[test]
    fn matcher_exclude_active() {
        let s = TabMacroSpec {
            match_rule: Matcher {
                exclude_active: true,
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        let mut t = tab("x.com");
        t.active = true;
        assert!(!s.matches(&t));
        t.active = false;
        assert!(s.matches(&t));
    }

    #[test]
    fn matcher_silent_and_playing_audio_flags() {
        let mut t = tab("x.com");
        let silent = TabMacroSpec {
            match_rule: Matcher {
                silent: true,
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        t.playing_audio = true;
        assert!(!silent.matches(&t));
        t.playing_audio = false;
        assert!(silent.matches(&t));

        let loud = TabMacroSpec {
            match_rule: Matcher {
                playing_audio: true,
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        t.playing_audio = false;
        assert!(!loud.matches(&t));
        t.playing_audio = true;
        assert!(loud.matches(&t));
    }

    #[test]
    fn matcher_in_group_scopes() {
        let s = TabMacroSpec {
            match_rule: Matcher {
                in_group: Some("work".into()),
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        let mut t = tab("x.com");
        t.group = Some("work");
        assert!(s.matches(&t));
        t.group = Some("personal");
        assert!(!s.matches(&t));
        t.group = None;
        assert!(!s.matches(&t));
    }

    #[test]
    fn matcher_age_days_floor() {
        let s = TabMacroSpec {
            match_rule: Matcher {
                created_before_days: Some(7),
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        let mut t = tab("x.com");
        t.age_days = 3;
        assert!(!s.matches(&t));
        t.age_days = 7;
        assert!(s.matches(&t));
        t.age_days = 30;
        assert!(s.matches(&t));
    }

    #[test]
    fn matcher_title_contains_case_insensitive() {
        let s = TabMacroSpec {
            match_rule: Matcher {
                title_contains: Some("NEWS".into()),
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        let mut t = tab("x.com");
        t.title = "Morning News Digest";
        assert!(s.matches(&t));
        t.title = "Stock Prices";
        assert!(!s.matches(&t));
    }

    #[test]
    fn matched_indices_respects_max_tabs() {
        let s = TabMacroSpec {
            max_tabs: 2,
            match_rule: Matcher::default(),
            ..TabMacroSpec::default_profile()
        };
        let tabs = (0..5).map(|_| tab("x.com")).collect::<Vec<_>>();
        let ix = s.matched_indices(&tabs);
        assert_eq!(ix, vec![0, 1]);
    }

    #[test]
    fn matched_indices_preserves_input_order() {
        let s = TabMacroSpec {
            match_rule: Matcher {
                host_globs: vec!["*://*.news.com/*".into()],
                ..Matcher::default()
            },
            ..TabMacroSpec::default_profile()
        };
        let tabs = vec![
            tab("x.com"),
            tab("www.news.com"),
            tab("y.com"),
            tab("b.news.com"),
        ];
        assert_eq!(s.matched_indices(&tabs), vec![1, 3]);
    }

    #[test]
    fn matched_indices_unlimited_when_max_tabs_zero() {
        let s = TabMacroSpec {
            max_tabs: 0,
            match_rule: Matcher::default(),
            ..TabMacroSpec::default_profile()
        };
        let tabs = (0..500).map(|_| tab("x.com")).collect::<Vec<_>>();
        assert_eq!(s.matched_indices(&tabs).len(), 500);
    }

    #[test]
    fn render_rewrite_substitutes_tokens() {
        let s = TabMacroSpec {
            action: Action::RewriteUrl,
            payload: ActionPayload {
                url_template: Some("https://archive.org/{host}/{path}?q={query}".into()),
                ..ActionPayload::default()
            },
            ..TabMacroSpec::default_profile()
        };
        assert_eq!(
            s.render_rewrite(
                "https://ex.com/news?x=1",
                "ex.com",
                "/news",
                "x=1"
            ),
            "https://archive.org/ex.com//news?q=x=1"
        );
    }

    #[test]
    fn render_rewrite_no_template_returns_original() {
        let s = TabMacroSpec::default_profile();
        assert_eq!(
            s.render_rewrite("https://ex.com/", "ex.com", "/", ""),
            "https://ex.com/"
        );
    }

    #[test]
    fn trigger_roundtrips_through_serde() {
        for t in [
            Trigger::Command,
            Trigger::Hotkey,
            Trigger::Omnibox,
            Trigger::AutoMatch,
            Trigger::Periodic,
        ] {
            let s = TabMacroSpec {
                trigger: t,
                ..TabMacroSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TabMacroSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.trigger, t);
        }
    }

    #[test]
    fn action_roundtrips_through_serde() {
        for a in [
            Action::Close,
            Action::Reload,
            Action::Duplicate,
            Action::MoveToGroup,
            Action::Pin,
            Action::Unpin,
            Action::Mute,
            Action::Unmute,
            Action::Hibernate,
            Action::Discard,
            Action::RewriteUrl,
            Action::EvalScript,
            Action::BookmarkAll,
            Action::Overview,
        ] {
            let s = TabMacroSpec {
                action: a,
                ..TabMacroSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: TabMacroSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.action, a);
        }
    }

    #[test]
    fn registry_by_trigger_skips_disabled() {
        let mut reg = TabMacroRegistry::new();
        reg.insert(TabMacroSpec {
            name: "a".into(),
            trigger: Trigger::Hotkey,
            enabled: true,
            ..TabMacroSpec::default_profile()
        });
        reg.insert(TabMacroSpec {
            name: "b".into(),
            trigger: Trigger::Hotkey,
            enabled: false,
            ..TabMacroSpec::default_profile()
        });
        let list = reg.by_trigger(Trigger::Hotkey);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "a");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_tab_macro_form() {
        let src = r#"
            (deftab-macro :name "archive-news"
                          :trigger "command"
                          :action "move-to-group"
                          :atomic #t
                          :dry-run #f
                          :max-tabs 50
                          :confirm-destructive #f
                          :enabled #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.action, Action::MoveToGroup);
        assert_eq!(s.max_tabs, 50);
        assert!(s.enabled);
        assert!(!s.confirm_destructive);
    }
}
