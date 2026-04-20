//! `(defnavigation-intent)` — where new navigations open.
//!
//! Absorbs Chrome/Firefox/Safari link-handling prefs, Vivaldi
//! "intelligent tabs", Arc new-tab-vs-current logic, Firefox
//! "open links in tabs rather than windows". Each rule declares, for
//! a host-scoped set of link sources (click, middle-click, keyboard
//! shortcut, script-initiated), where the target should open and
//! whether to focus it.
//!
//! ```lisp
//! (defnavigation-intent :name         "foreground-on-click"
//!                       :host         "*"
//!                       :link-click   :new-tab-foreground
//!                       :middle-click :new-tab-background
//!                       :cmd-click    :new-tab-background
//!                       :script-open  :new-window
//!                       :same-site    :current-tab
//!                       :popup        :block)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Where a navigation target opens.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum OpenDisposition {
    #[default]
    CurrentTab,
    NewTabForeground,
    NewTabBackground,
    NewWindow,
    /// Open in a separate process + ephemeral session.
    IncognitoWindow,
    /// Open within the current (deftab-group).
    SameTabGroup,
    /// Pop the URL into the reader pane without leaving the tab.
    InlineReader,
    /// Copy the URL to clipboard instead of navigating.
    CopyLink,
    /// Reject the navigation.
    Block,
}

/// What kind of click/action triggered the navigation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ClickSource {
    /// Plain left-click on an `<a>` or `<form>` submit.
    LinkClick,
    /// Middle-click / wheel-click.
    MiddleClick,
    /// Ctrl/Cmd + click.
    CmdClick,
    /// Ctrl/Cmd + Shift + click (Chrome: foreground new tab).
    CmdShiftClick,
    /// Script-initiated `window.open`.
    ScriptOpen,
    /// Submit of a `<form target="_blank">`.
    FormTargetBlank,
    /// Anchor with `target="_blank"` rel-opener.
    AnchorTargetBlank,
    /// Drag-and-drop onto the tab strip.
    DragDrop,
    /// Bookmark / omnibox navigation.
    Omnibox,
    /// Back / forward button.
    BackForward,
    /// Reload.
    Reload,
    /// Keyboard shortcut like Cmd+T (new blank tab).
    KeyboardShortcut,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defnavigation-intent"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NavigationIntentSpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    /// Default for plain link-clicks.
    #[serde(default = "default_link_click")]
    pub link_click: OpenDisposition,
    #[serde(default = "default_middle_click")]
    pub middle_click: OpenDisposition,
    #[serde(default = "default_middle_click")]
    pub cmd_click: OpenDisposition,
    #[serde(default = "default_cmd_shift_click")]
    pub cmd_shift_click: OpenDisposition,
    #[serde(default = "default_script_open")]
    pub script_open: OpenDisposition,
    #[serde(default = "default_middle_click")]
    pub form_target_blank: OpenDisposition,
    #[serde(default = "default_middle_click")]
    pub anchor_target_blank: OpenDisposition,
    #[serde(default = "default_middle_click")]
    pub drag_drop: OpenDisposition,
    /// Omnibox / bookmark navigation defaults.
    #[serde(default = "default_omnibox")]
    pub omnibox: OpenDisposition,
    /// Whether same-origin navigations get forced to `CurrentTab`
    /// regardless of source (reading-flow preservation).
    #[serde(default)]
    pub same_site_override: Option<OpenDisposition>,
    /// Cross-origin override — force `NewTabBackground` on all
    /// cross-origin navigations (privacy-ish: no leaks between sites).
    #[serde(default)]
    pub cross_origin_override: Option<OpenDisposition>,
    /// `window.open` popup policy.
    #[serde(default = "default_popup")]
    pub popup: OpenDisposition,
    /// Require user gesture for script-initiated navigations.
    #[serde(default = "default_require_gesture_script")]
    pub require_gesture_for_script_open: bool,
    /// Host allow-list for popups (honored even when popup=Block).
    #[serde(default)]
    pub popup_allow_hosts: Vec<String>,
    /// Force `rel="noopener"` on all target=_blank anchors.
    #[serde(default = "default_force_noopener")]
    pub force_noopener: bool,
    /// Strip Referer header on cross-origin navigations.
    #[serde(default = "default_strip_cross_origin_referrer")]
    pub strip_cross_origin_referrer: bool,
    /// Priority tiebreak.
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
fn default_link_click() -> OpenDisposition {
    OpenDisposition::CurrentTab
}
fn default_middle_click() -> OpenDisposition {
    OpenDisposition::NewTabBackground
}
fn default_cmd_shift_click() -> OpenDisposition {
    OpenDisposition::NewTabForeground
}
fn default_script_open() -> OpenDisposition {
    OpenDisposition::NewTabBackground
}
fn default_omnibox() -> OpenDisposition {
    OpenDisposition::CurrentTab
}
fn default_popup() -> OpenDisposition {
    OpenDisposition::Block
}
fn default_require_gesture_script() -> bool {
    true
}
fn default_force_noopener() -> bool {
    true
}
fn default_strip_cross_origin_referrer() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl NavigationIntentSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            link_click: OpenDisposition::CurrentTab,
            middle_click: OpenDisposition::NewTabBackground,
            cmd_click: OpenDisposition::NewTabBackground,
            cmd_shift_click: OpenDisposition::NewTabForeground,
            script_open: OpenDisposition::NewTabBackground,
            form_target_blank: OpenDisposition::NewTabBackground,
            anchor_target_blank: OpenDisposition::NewTabBackground,
            drag_drop: OpenDisposition::NewTabBackground,
            omnibox: OpenDisposition::CurrentTab,
            same_site_override: None,
            cross_origin_override: None,
            popup: OpenDisposition::Block,
            require_gesture_for_script_open: true,
            popup_allow_hosts: vec![],
            force_noopener: true,
            strip_cross_origin_referrer: true,
            priority: 0,
            enabled: true,
            description: Some(
                "Default navigation intent — CurrentTab on click, NewTabBackground on middle/cmd/target=_blank, block popups.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    fn base_for(&self, source: ClickSource) -> OpenDisposition {
        use ClickSource::*;
        match source {
            LinkClick => self.link_click,
            MiddleClick => self.middle_click,
            CmdClick => self.cmd_click,
            CmdShiftClick => self.cmd_shift_click,
            ScriptOpen => self.script_open,
            FormTargetBlank => self.form_target_blank,
            AnchorTargetBlank => self.anchor_target_blank,
            DragDrop => self.drag_drop,
            Omnibox => self.omnibox,
            BackForward | Reload => OpenDisposition::CurrentTab,
            KeyboardShortcut => OpenDisposition::NewTabForeground,
        }
    }

    /// Resolve the final disposition for a click.
    /// `same_origin`: true when the target is same-origin as the
    /// current document. `had_user_gesture`: distinguishes genuine
    /// user actions from `window.open` called during page load.
    #[must_use]
    pub fn resolve(
        &self,
        source: ClickSource,
        same_origin: bool,
        had_user_gesture: bool,
    ) -> OpenDisposition {
        if !self.enabled {
            return self.base_for(source);
        }

        // Popup is a distinct top-level reject path.
        if matches!(source, ClickSource::ScriptOpen)
            && self.require_gesture_for_script_open
            && !had_user_gesture
        {
            return OpenDisposition::Block;
        }

        let base = self.base_for(source);
        if same_origin {
            if let Some(ov) = self.same_site_override {
                return ov;
            }
        } else if let Some(ov) = self.cross_origin_override {
            return ov;
        }
        base
    }

    /// Is `host` on the popup allow-list (bypasses popup block)?
    #[must_use]
    pub fn popup_allowed(&self, host: &str) -> bool {
        self.popup_allow_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct NavigationIntentRegistry {
    specs: Vec<NavigationIntentSpec>,
}

impl NavigationIntentRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: NavigationIntentSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = NavigationIntentSpec>) {
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
    pub fn specs(&self) -> &[NavigationIntentSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&NavigationIntentSpec> {
        let mut matches: Vec<&NavigationIntentSpec> = self
            .specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host))
            .collect();
        matches.sort_by(|a, b| {
            let ah = !(a.host.is_empty() || a.host == "*");
            let bh = !(b.host.is_empty() || b.host == "*");
            match bh.cmp(&ah) {
                std::cmp::Ordering::Equal => b.priority.cmp(&a.priority),
                other => other,
            }
        });
        matches.first().copied()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<NavigationIntentSpec>, String> {
    tatara_lisp::compile_typed::<NavigationIntentSpec>(src)
        .map_err(|e| format!("failed to compile defnavigation-intent forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<NavigationIntentSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_current_tab_on_click_bg_on_middle() {
        let s = NavigationIntentSpec::default_profile();
        assert_eq!(s.link_click, OpenDisposition::CurrentTab);
        assert_eq!(s.middle_click, OpenDisposition::NewTabBackground);
        assert_eq!(s.cmd_shift_click, OpenDisposition::NewTabForeground);
        assert_eq!(s.popup, OpenDisposition::Block);
    }

    #[test]
    fn resolve_maps_click_source_to_base_disposition() {
        let s = NavigationIntentSpec::default_profile();
        assert_eq!(
            s.resolve(ClickSource::LinkClick, true, true),
            OpenDisposition::CurrentTab
        );
        assert_eq!(
            s.resolve(ClickSource::MiddleClick, true, true),
            OpenDisposition::NewTabBackground
        );
        assert_eq!(
            s.resolve(ClickSource::CmdShiftClick, true, true),
            OpenDisposition::NewTabForeground
        );
        assert_eq!(
            s.resolve(ClickSource::Reload, true, true),
            OpenDisposition::CurrentTab
        );
    }

    #[test]
    fn resolve_blocks_script_open_without_user_gesture() {
        let s = NavigationIntentSpec::default_profile();
        assert_eq!(
            s.resolve(ClickSource::ScriptOpen, false, false),
            OpenDisposition::Block
        );
        assert_eq!(
            s.resolve(ClickSource::ScriptOpen, false, true),
            OpenDisposition::NewTabBackground
        );
    }

    #[test]
    fn resolve_respects_same_site_override() {
        let s = NavigationIntentSpec {
            link_click: OpenDisposition::NewTabBackground,
            same_site_override: Some(OpenDisposition::CurrentTab),
            ..NavigationIntentSpec::default_profile()
        };
        // Same origin → override fires.
        assert_eq!(
            s.resolve(ClickSource::LinkClick, true, true),
            OpenDisposition::CurrentTab
        );
        // Cross origin → override does NOT fire.
        assert_eq!(
            s.resolve(ClickSource::LinkClick, false, true),
            OpenDisposition::NewTabBackground
        );
    }

    #[test]
    fn resolve_respects_cross_origin_override() {
        let s = NavigationIntentSpec {
            link_click: OpenDisposition::CurrentTab,
            cross_origin_override: Some(OpenDisposition::NewTabBackground),
            ..NavigationIntentSpec::default_profile()
        };
        assert_eq!(
            s.resolve(ClickSource::LinkClick, false, true),
            OpenDisposition::NewTabBackground
        );
        assert_eq!(
            s.resolve(ClickSource::LinkClick, true, true),
            OpenDisposition::CurrentTab
        );
    }

    #[test]
    fn resolve_disabled_still_returns_base() {
        let s = NavigationIntentSpec {
            enabled: false,
            ..NavigationIntentSpec::default_profile()
        };
        // Disabled = no overrides, no gesture enforcement. The caller
        // is expected to not consult this profile at all; but the
        // decision function stays total.
        assert_eq!(
            s.resolve(ClickSource::ScriptOpen, false, false),
            OpenDisposition::NewTabBackground
        );
    }

    #[test]
    fn popup_allowed_respects_allow_list() {
        let s = NavigationIntentSpec {
            popup_allow_hosts: vec!["*://*.auth.example.com/*".into()],
            ..NavigationIntentSpec::default_profile()
        };
        assert!(s.popup_allowed("sso.auth.example.com"));
        assert!(!s.popup_allowed("ads.com"));
    }

    #[test]
    fn registry_resolve_prefers_host_specific_and_priority() {
        let mut reg = NavigationIntentRegistry::new();
        reg.insert(NavigationIntentSpec::default_profile());
        reg.insert(NavigationIntentSpec {
            name: "github".into(),
            host: "*://*.github.com/*".into(),
            priority: 5,
            link_click: OpenDisposition::NewTabBackground,
            ..NavigationIntentSpec::default_profile()
        });
        reg.insert(NavigationIntentSpec {
            name: "github-louder".into(),
            host: "*://*.github.com/*".into(),
            priority: 100,
            link_click: OpenDisposition::NewTabForeground,
            ..NavigationIntentSpec::default_profile()
        });
        let r = reg.resolve("www.github.com").unwrap();
        // Within host-specific tier, priority=100 wins.
        assert_eq!(r.name, "github-louder");
        assert_eq!(r.link_click, OpenDisposition::NewTabForeground);
    }

    #[test]
    fn registry_resolve_falls_back_to_wildcard() {
        let mut reg = NavigationIntentRegistry::new();
        reg.insert(NavigationIntentSpec::default_profile());
        assert_eq!(
            reg.resolve("example.org").unwrap().name,
            "default"
        );
    }

    #[test]
    fn disposition_roundtrips_through_serde() {
        for d in [
            OpenDisposition::CurrentTab,
            OpenDisposition::NewTabForeground,
            OpenDisposition::NewTabBackground,
            OpenDisposition::NewWindow,
            OpenDisposition::IncognitoWindow,
            OpenDisposition::SameTabGroup,
            OpenDisposition::InlineReader,
            OpenDisposition::CopyLink,
            OpenDisposition::Block,
        ] {
            let s = NavigationIntentSpec {
                link_click: d,
                ..NavigationIntentSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: NavigationIntentSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.link_click, d);
        }
    }

    #[test]
    fn click_source_roundtrips_through_serde() {
        // Not in the spec directly; we use it in resolve(). Ensure
        // serde coverage for downstream logging.
        for c in [
            ClickSource::LinkClick,
            ClickSource::MiddleClick,
            ClickSource::CmdClick,
            ClickSource::CmdShiftClick,
            ClickSource::ScriptOpen,
            ClickSource::FormTargetBlank,
            ClickSource::AnchorTargetBlank,
            ClickSource::DragDrop,
            ClickSource::Omnibox,
            ClickSource::BackForward,
            ClickSource::Reload,
            ClickSource::KeyboardShortcut,
        ] {
            let json = serde_json::to_string(&c).unwrap();
            let back: ClickSource = serde_json::from_str(&json).unwrap();
            assert_eq!(back, c);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_navigation_intent_form() {
        let src = r#"
            (defnavigation-intent :name "foreground-on-click"
                                  :host "*"
                                  :link-click "new-tab-foreground"
                                  :middle-click "new-tab-background"
                                  :cmd-click "new-tab-background"
                                  :script-open "new-window"
                                  :popup "block"
                                  :force-noopener #t
                                  :strip-cross-origin-referrer #t
                                  :require-gesture-for-script-open #t
                                  :popup-allow-hosts ("*://*.auth.example.com/*"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.link_click, OpenDisposition::NewTabForeground);
        assert_eq!(s.script_open, OpenDisposition::NewWindow);
        assert!(s.force_noopener);
        assert_eq!(s.popup_allow_hosts.len(), 1);
    }
}
