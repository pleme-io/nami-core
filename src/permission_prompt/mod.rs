//! `(defpermission-prompt)` — declarative prompt UX.
//!
//! Paired with [`crate::permission_policy`]. Policy decides WHAT
//! (allow/block/prompt); prompt declares HOW the prompt looks and
//! behaves — text, icon, remember duration, auto-deny timeout, focus
//! steal policy. Absorbs Chrome quiet UI, Firefox permission panel,
//! Safari pop-over, Brave shield.
//!
//! ```lisp
//! (defpermission-prompt :name         "quiet"
//!                       :host         "*"
//!                       :style        :quiet-badge
//!                       :remember-days 30
//!                       :deny-after-seconds 20
//!                       :focus-steal  :none
//!                       :group-related #t
//!                       :text-template "{origin} wants to use your {permission}")
//! ```

use serde::{Deserialize, Serialize};

pub use crate::permission_policy::Permission;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How intrusive the prompt is.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PromptStyle {
    /// Modal dialog — takes focus, blocks page (Chrome pre-M90).
    Modal,
    /// Floating anchored bubble (Chrome/Firefox current default).
    #[default]
    Anchored,
    /// Small omnibox badge — user must click to see full prompt
    /// (Chrome "quiet UI" for notifications).
    QuietBadge,
    /// Toast bottom-corner (Brave-ish).
    Toast,
    /// Full-page interstitial (for high-risk permissions).
    Interstitial,
    /// No UI — defer to policy (auto-deny if not handled).
    None,
}

/// Focus-steal behavior.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FocusSteal {
    /// Prompt steals focus immediately.
    Immediate,
    /// Prompt does not steal focus — waits for user to click badge.
    #[default]
    None,
    /// Steals focus only when the tab is already foregrounded.
    Foreground,
}

/// Prompt profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defpermission-prompt"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPromptSpec {
    pub name: String,
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    #[serde(default)]
    pub style: PromptStyle,
    /// How long to remember the user's answer. 0 = always ephemeral.
    #[serde(default = "default_remember_days")]
    pub remember_days: u32,
    /// Seconds before auto-deny + dismissing the prompt.
    /// 0 = wait forever.
    #[serde(default = "default_deny_after_seconds")]
    pub deny_after_seconds: u32,
    #[serde(default)]
    pub focus_steal: FocusSteal,
    /// Bundle related permission requests into one prompt (camera +
    /// microphone → single "join meeting" ask).
    #[serde(default = "default_group_related")]
    pub group_related: bool,
    /// Custom prompt body — `{origin}` / `{permission}` tokens.
    #[serde(default)]
    pub text_template: Option<String>,
    /// Custom icon URL — empty = platform default.
    #[serde(default)]
    pub icon_url: Option<String>,
    /// Which permissions this prompt applies to — empty = all.
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// Permissions to never prompt with this UX (fall through to
    /// lower-priority prompts or default).
    #[serde(default)]
    pub exclude_permissions: Vec<Permission>,
    /// Show the "don't ask again" checkbox.
    #[serde(default = "default_offer_permanent")]
    pub offer_permanent: bool,
    /// Priority tiebreak — higher wins when two prompts both apply.
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_remember_days() -> u32 {
    30
}
fn default_deny_after_seconds() -> u32 {
    45
}
fn default_group_related() -> bool {
    true
}
fn default_offer_permanent() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl PermissionPromptSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            style: PromptStyle::Anchored,
            remember_days: 30,
            deny_after_seconds: 45,
            focus_steal: FocusSteal::None,
            group_related: true,
            text_template: Some("{origin} wants to use your {permission}".into()),
            icon_url: None,
            permissions: vec![],
            exclude_permissions: vec![],
            offer_permanent: true,
            priority: 0,
            enabled: true,
            description: Some(
                "Default prompt — anchored bubble, 30-day memory, 45-second auto-deny.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// Does this prompt cover `permission`?
    #[must_use]
    pub fn applies_to(&self, permission: Permission) -> bool {
        if self.exclude_permissions.contains(&permission) {
            return false;
        }
        self.permissions.is_empty() || self.permissions.contains(&permission)
    }

    /// Render the prompt text for a given origin + permission.
    /// Returns None if no template is set (caller uses a default).
    #[must_use]
    pub fn render_text(&self, origin: &str, permission_label: &str) -> Option<String> {
        self.text_template.as_deref().map(|t| {
            t.replace("{origin}", origin)
                .replace("{permission}", permission_label)
        })
    }

    /// Effective remember window: 0 when style is `None` (don't
    /// remember a non-prompt), else the declared value.
    #[must_use]
    pub fn effective_remember_days(&self) -> u32 {
        if matches!(self.style, PromptStyle::None) {
            0
        } else {
            self.remember_days
        }
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct PermissionPromptRegistry {
    specs: Vec<PermissionPromptSpec>,
}

impl PermissionPromptRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PermissionPromptSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PermissionPromptSpec>) {
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
    pub fn specs(&self) -> &[PermissionPromptSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PermissionPromptSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Resolve the prompt profile to use for `permission` on `host`.
    /// Preference order: host-specific permission-scoped → host-
    /// specific wildcard → wildcard permission-scoped → wildcard.
    /// Priority tiebreak within each tier.
    #[must_use]
    pub fn resolve(&self, permission: Permission, host: &str) -> Option<&PermissionPromptSpec> {
        let mut candidates: Vec<&PermissionPromptSpec> = self
            .specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host) && s.applies_to(permission))
            .collect();

        candidates.sort_by_key(|s| {
            let host_specific = !(s.host.is_empty() || s.host == "*");
            let perm_specific = !s.permissions.is_empty();
            // Higher scores win: host-specific + permission-specific > host-specific > permission-specific > wildcard.
            let tier = u32::from(host_specific) * 2 + u32::from(perm_specific);
            // Negate for descending sort, then priority as secondary key.
            (u32::MAX - tier, -s.priority)
        });

        candidates.first().copied()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PermissionPromptSpec>, String> {
    tatara_lisp::compile_typed::<PermissionPromptSpec>(src)
        .map_err(|e| format!("failed to compile defpermission-prompt forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PermissionPromptSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_anchored_30_day_memory() {
        let s = PermissionPromptSpec::default_profile();
        assert_eq!(s.style, PromptStyle::Anchored);
        assert_eq!(s.remember_days, 30);
        assert_eq!(s.deny_after_seconds, 45);
    }

    #[test]
    fn applies_to_empty_permissions_covers_all() {
        let s = PermissionPromptSpec::default_profile();
        assert!(s.applies_to(Permission::Camera));
        assert!(s.applies_to(Permission::Usb));
    }

    #[test]
    fn applies_to_respects_exclude_list() {
        let s = PermissionPromptSpec {
            exclude_permissions: vec![Permission::Usb],
            ..PermissionPromptSpec::default_profile()
        };
        assert!(!s.applies_to(Permission::Usb));
        assert!(s.applies_to(Permission::Camera));
    }

    #[test]
    fn applies_to_limits_to_named_set_when_populated() {
        let s = PermissionPromptSpec {
            permissions: vec![Permission::Camera, Permission::Microphone],
            ..PermissionPromptSpec::default_profile()
        };
        assert!(s.applies_to(Permission::Camera));
        assert!(s.applies_to(Permission::Microphone));
        assert!(!s.applies_to(Permission::Notifications));
    }

    #[test]
    fn render_text_substitutes_origin_and_permission() {
        let s = PermissionPromptSpec::default_profile();
        assert_eq!(
            s.render_text("example.com", "camera").unwrap(),
            "example.com wants to use your camera"
        );
    }

    #[test]
    fn render_text_none_when_no_template() {
        let s = PermissionPromptSpec {
            text_template: None,
            ..PermissionPromptSpec::default_profile()
        };
        assert!(s.render_text("example.com", "camera").is_none());
    }

    #[test]
    fn effective_remember_days_is_zero_for_style_none() {
        let s = PermissionPromptSpec {
            style: PromptStyle::None,
            remember_days: 90,
            ..PermissionPromptSpec::default_profile()
        };
        assert_eq!(s.effective_remember_days(), 0);
    }

    #[test]
    fn resolve_prefers_host_and_permission_specific_prompt() {
        let mut reg = PermissionPromptRegistry::new();
        reg.insert(PermissionPromptSpec::default_profile());
        reg.insert(PermissionPromptSpec {
            name: "host-any-perm".into(),
            host: "*://meet.google.com/*".into(),
            ..PermissionPromptSpec::default_profile()
        });
        reg.insert(PermissionPromptSpec {
            name: "host-and-perm".into(),
            host: "*://meet.google.com/*".into(),
            permissions: vec![Permission::Camera],
            ..PermissionPromptSpec::default_profile()
        });
        // Both host-and-perm and host-any-perm match "meet.google.com" +
        // Camera, but host-and-perm wins because permission-specific
        // beats permission-agnostic at the same host tier.
        let resolved = reg.resolve(Permission::Camera, "meet.google.com").unwrap();
        assert_eq!(resolved.name, "host-and-perm");
    }

    #[test]
    fn resolve_falls_back_to_wildcard_host_when_no_specific() {
        let mut reg = PermissionPromptRegistry::new();
        reg.insert(PermissionPromptSpec::default_profile());
        assert_eq!(
            reg.resolve(Permission::Camera, "example.org").unwrap().name,
            "default"
        );
    }

    #[test]
    fn resolve_respects_exclude_list() {
        let mut reg = PermissionPromptRegistry::new();
        reg.insert(PermissionPromptSpec {
            name: "no-usb".into(),
            exclude_permissions: vec![Permission::Usb],
            ..PermissionPromptSpec::default_profile()
        });
        // Prompt excludes USB, and there's no other prompt for it.
        assert!(reg.resolve(Permission::Usb, "example.com").is_none());
        assert!(reg.resolve(Permission::Camera, "example.com").is_some());
    }

    #[test]
    fn resolve_priority_tiebreaks_within_tier() {
        let mut reg = PermissionPromptRegistry::new();
        reg.insert(PermissionPromptSpec {
            name: "lo".into(),
            priority: 0,
            ..PermissionPromptSpec::default_profile()
        });
        reg.insert(PermissionPromptSpec {
            name: "hi".into(),
            priority: 100,
            ..PermissionPromptSpec::default_profile()
        });
        assert_eq!(
            reg.resolve(Permission::Camera, "example.com").unwrap().name,
            "hi"
        );
    }

    #[test]
    fn style_roundtrips_through_serde() {
        for st in [
            PromptStyle::Modal,
            PromptStyle::Anchored,
            PromptStyle::QuietBadge,
            PromptStyle::Toast,
            PromptStyle::Interstitial,
            PromptStyle::None,
        ] {
            let s = PermissionPromptSpec {
                style: st,
                ..PermissionPromptSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: PermissionPromptSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.style, st);
        }
    }

    #[test]
    fn focus_steal_roundtrips_through_serde() {
        for f in [
            FocusSteal::Immediate,
            FocusSteal::None,
            FocusSteal::Foreground,
        ] {
            let s = PermissionPromptSpec {
                focus_steal: f,
                ..PermissionPromptSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: PermissionPromptSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.focus_steal, f);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_permission_prompt_form() {
        let src = r#"
            (defpermission-prompt :name "quiet"
                                  :host "*"
                                  :style "quiet-badge"
                                  :remember-days 30
                                  :deny-after-seconds 20
                                  :focus-steal "none"
                                  :group-related #t
                                  :permissions ("notifications" "push")
                                  :text-template "{origin} wants to use your {permission}")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.style, PromptStyle::QuietBadge);
        assert_eq!(s.focus_steal, FocusSteal::None);
        assert_eq!(s.deny_after_seconds, 20);
        assert_eq!(s.permissions.len(), 2);
    }
}
