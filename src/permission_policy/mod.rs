//! `(defpermission-policy)` — per-host permission decision tree.
//!
//! Absorbs Chrome/Firefox/Safari permission UIs, Brave Shields
//! permissions, Arc site settings. Every Permissions-API surface
//! plus the sensor-like ones (clipboard, USB, serial) gets a Decision
//! per host, with sensible defaults and explicit allow/block/prompt.
//!
//! ```lisp
//! (defpermission-policy :name         "strict"
//!                       :host         "*"
//!                       :camera       :block
//!                       :microphone   :prompt
//!                       :geolocation  :block
//!                       :notifications :block
//!                       :clipboard-read :prompt-ephemeral
//!                       :midi         :block
//!                       :usb          :block
//!                       :require-user-gesture (camera microphone))
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Permission kinds — union of the Permissions API + hardware-ish
/// surfaces every browser now gates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Camera,
    Microphone,
    Geolocation,
    Notifications,
    PersistentStorage,
    ClipboardRead,
    ClipboardWrite,
    Midi,
    BackgroundSync,
    IdleDetection,
    ScreenWake,
    Usb,
    Serial,
    Hid,
    Bluetooth,
    NfcRead,
    AmbientLightSensor,
    /// window.open popups.
    Popups,
    /// `Notification.requestPermission()` + Push API.
    Push,
    /// Site requested autoplay with sound.
    AutoplayWithSound,
    /// `navigator.wakeLock.request('screen')`.
    DisplayCapture,
    /// Protected content playback (Widevine / FairPlay).
    ProtectedMedia,
    /// Payment Handler registration.
    PaymentHandler,
    Custom,
}

/// What to do when a page requests this permission.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    /// Grant silently.
    Allow,
    /// Reject silently.
    Block,
    /// Ask the user and remember the answer.
    #[default]
    Prompt,
    /// Ask the user but don't remember — next time asks again.
    PromptEphemeral,
    /// Only consider the call if it follows a recent user gesture
    /// (click/keydown). Otherwise reject silently.
    RequireUserGesture,
    /// Reject but show a shield/badge so the user can open
    /// permissions UI.
    BlockWithBadge,
}

/// A single host's decision map.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defpermission-policy"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPolicySpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub camera: Decision,
    #[serde(default)]
    pub microphone: Decision,
    #[serde(default = "default_block_sensitive")]
    pub geolocation: Decision,
    #[serde(default)]
    pub notifications: Decision,
    #[serde(default)]
    pub persistent_storage: Decision,
    #[serde(default)]
    pub clipboard_read: Decision,
    #[serde(default = "default_allow")]
    pub clipboard_write: Decision,
    #[serde(default = "default_block_sensitive")]
    pub midi: Decision,
    #[serde(default)]
    pub background_sync: Decision,
    #[serde(default)]
    pub idle_detection: Decision,
    #[serde(default)]
    pub screen_wake: Decision,
    #[serde(default = "default_block_sensitive")]
    pub usb: Decision,
    #[serde(default = "default_block_sensitive")]
    pub serial: Decision,
    #[serde(default = "default_block_sensitive")]
    pub hid: Decision,
    #[serde(default = "default_block_sensitive")]
    pub bluetooth: Decision,
    #[serde(default = "default_block_sensitive")]
    pub nfc_read: Decision,
    #[serde(default)]
    pub ambient_light_sensor: Decision,
    #[serde(default)]
    pub popups: Decision,
    #[serde(default)]
    pub push: Decision,
    #[serde(default = "default_allow")]
    pub autoplay_with_sound: Decision,
    #[serde(default)]
    pub display_capture: Decision,
    #[serde(default = "default_allow")]
    pub protected_media: Decision,
    #[serde(default)]
    pub payment_handler: Decision,
    /// Permissions that ALWAYS require a fresh user-gesture even if
    /// the base decision is `Allow`.
    #[serde(default)]
    pub require_user_gesture: Vec<Permission>,
    /// Host allow-list — bypass all `Block` decisions.
    #[serde(default)]
    pub allow_hosts: Vec<String>,
    /// Host block-list — force `Block` even if the decision says Allow.
    #[serde(default)]
    pub block_hosts: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_block_sensitive() -> Decision {
    Decision::Block
}
fn default_allow() -> Decision {
    Decision::Allow
}
fn default_enabled() -> bool {
    true
}

impl PermissionPolicySpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            camera: Decision::Prompt,
            microphone: Decision::Prompt,
            geolocation: Decision::Block,
            notifications: Decision::Prompt,
            persistent_storage: Decision::Prompt,
            clipboard_read: Decision::Prompt,
            clipboard_write: Decision::Allow,
            midi: Decision::Block,
            background_sync: Decision::Prompt,
            idle_detection: Decision::Block,
            screen_wake: Decision::Prompt,
            usb: Decision::Block,
            serial: Decision::Block,
            hid: Decision::Block,
            bluetooth: Decision::Block,
            nfc_read: Decision::Block,
            ambient_light_sensor: Decision::Block,
            popups: Decision::Block,
            push: Decision::Prompt,
            autoplay_with_sound: Decision::Allow,
            display_capture: Decision::Prompt,
            protected_media: Decision::Allow,
            payment_handler: Decision::Prompt,
            require_user_gesture: vec![
                Permission::Camera,
                Permission::Microphone,
                Permission::DisplayCapture,
            ],
            allow_hosts: vec![],
            block_hosts: vec![],
            enabled: true,
            description: Some(
                "Default permission policy — Prompt on camera/mic/notifications, Block on USB/Serial/MIDI, Allow on clipboard-write.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn is_allowed(&self, host: &str) -> bool {
        self.allow_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    #[must_use]
    pub fn is_blocked(&self, host: &str) -> bool {
        self.block_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Look up the decision for `permission` on `host`. Applies
    /// allow_hosts / block_hosts overrides — block wins.
    #[must_use]
    pub fn decide(&self, permission: Permission, host: &str) -> Decision {
        if self.is_blocked(host) {
            return Decision::Block;
        }
        let base = self.lookup(permission);
        if self.is_allowed(host) && base == Decision::Block {
            return Decision::Allow;
        }
        base
    }

    /// Does `permission` require a user-gesture on top of the base
    /// decision?
    #[must_use]
    pub fn requires_user_gesture(&self, permission: Permission) -> bool {
        self.require_user_gesture.contains(&permission)
    }

    fn lookup(&self, permission: Permission) -> Decision {
        use Permission::*;
        match permission {
            Camera => self.camera,
            Microphone => self.microphone,
            Geolocation => self.geolocation,
            Notifications => self.notifications,
            PersistentStorage => self.persistent_storage,
            ClipboardRead => self.clipboard_read,
            ClipboardWrite => self.clipboard_write,
            Midi => self.midi,
            BackgroundSync => self.background_sync,
            IdleDetection => self.idle_detection,
            ScreenWake => self.screen_wake,
            Usb => self.usb,
            Serial => self.serial,
            Hid => self.hid,
            Bluetooth => self.bluetooth,
            NfcRead => self.nfc_read,
            AmbientLightSensor => self.ambient_light_sensor,
            Popups => self.popups,
            Push => self.push,
            AutoplayWithSound => self.autoplay_with_sound,
            DisplayCapture => self.display_capture,
            ProtectedMedia => self.protected_media,
            PaymentHandler => self.payment_handler,
            Custom => Decision::Prompt,
        }
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct PermissionPolicyRegistry {
    specs: Vec<PermissionPolicySpec>,
}

impl PermissionPolicyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: PermissionPolicySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = PermissionPolicySpec>) {
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
    pub fn specs(&self) -> &[PermissionPolicySpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&PermissionPolicySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<PermissionPolicySpec>, String> {
    tatara_lisp::compile_typed::<PermissionPolicySpec>(src)
        .map_err(|e| format!("failed to compile defpermission-policy forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<PermissionPolicySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_blocks_sensitive_devices() {
        let s = PermissionPolicySpec::default_profile();
        assert_eq!(s.decide(Permission::Usb, "example.com"), Decision::Block);
        assert_eq!(s.decide(Permission::Serial, "example.com"), Decision::Block);
        assert_eq!(s.decide(Permission::Midi, "example.com"), Decision::Block);
        assert_eq!(s.decide(Permission::Geolocation, "example.com"), Decision::Block);
    }

    #[test]
    fn default_profile_prompts_common_media_permissions() {
        let s = PermissionPolicySpec::default_profile();
        assert_eq!(s.decide(Permission::Camera, "example.com"), Decision::Prompt);
        assert_eq!(s.decide(Permission::Microphone, "example.com"), Decision::Prompt);
        assert_eq!(s.decide(Permission::Notifications, "example.com"), Decision::Prompt);
    }

    #[test]
    fn default_profile_allows_clipboard_write() {
        let s = PermissionPolicySpec::default_profile();
        assert_eq!(s.decide(Permission::ClipboardWrite, "example.com"), Decision::Allow);
    }

    #[test]
    fn decide_block_hosts_force_block_even_on_allow_permission() {
        let s = PermissionPolicySpec {
            autoplay_with_sound: Decision::Allow,
            block_hosts: vec!["*://*.ads.com/*".into()],
            ..PermissionPolicySpec::default_profile()
        };
        assert_eq!(s.decide(Permission::AutoplayWithSound, "x.ads.com"), Decision::Block);
    }

    #[test]
    fn decide_allow_hosts_upgrade_block_to_allow() {
        let s = PermissionPolicySpec {
            usb: Decision::Block,
            allow_hosts: vec!["*://*.flashing-tool.io/*".into()],
            ..PermissionPolicySpec::default_profile()
        };
        assert_eq!(s.decide(Permission::Usb, "www.flashing-tool.io"), Decision::Allow);
        assert_eq!(s.decide(Permission::Usb, "other.com"), Decision::Block);
    }

    #[test]
    fn decide_allow_hosts_leaves_non_block_decisions_alone() {
        // allow_hosts upgrades Block→Allow, but doesn't touch Prompt
        // or PromptEphemeral — those still deserve a user decision.
        let s = PermissionPolicySpec {
            camera: Decision::Prompt,
            allow_hosts: vec!["*://*.trusted.com/*".into()],
            ..PermissionPolicySpec::default_profile()
        };
        assert_eq!(s.decide(Permission::Camera, "www.trusted.com"), Decision::Prompt);
    }

    #[test]
    fn decide_block_hosts_precede_allow_hosts() {
        let s = PermissionPolicySpec {
            camera: Decision::Allow,
            allow_hosts: vec!["*://*.ex.com/*".into()],
            block_hosts: vec!["*://ads.ex.com/*".into()],
            ..PermissionPolicySpec::default_profile()
        };
        assert_eq!(s.decide(Permission::Camera, "ads.ex.com"), Decision::Block);
        assert_eq!(s.decide(Permission::Camera, "www.ex.com"), Decision::Allow);
    }

    #[test]
    fn requires_user_gesture_covers_default_media_set() {
        let s = PermissionPolicySpec::default_profile();
        assert!(s.requires_user_gesture(Permission::Camera));
        assert!(s.requires_user_gesture(Permission::Microphone));
        assert!(s.requires_user_gesture(Permission::DisplayCapture));
        assert!(!s.requires_user_gesture(Permission::Notifications));
    }

    #[test]
    fn permission_roundtrips_through_serde() {
        for p in [
            Permission::Camera,
            Permission::Microphone,
            Permission::Geolocation,
            Permission::Notifications,
            Permission::PersistentStorage,
            Permission::ClipboardRead,
            Permission::ClipboardWrite,
            Permission::Midi,
            Permission::BackgroundSync,
            Permission::IdleDetection,
            Permission::ScreenWake,
            Permission::Usb,
            Permission::Serial,
            Permission::Hid,
            Permission::Bluetooth,
            Permission::NfcRead,
            Permission::AmbientLightSensor,
            Permission::Popups,
            Permission::Push,
            Permission::AutoplayWithSound,
            Permission::DisplayCapture,
            Permission::ProtectedMedia,
            Permission::PaymentHandler,
            Permission::Custom,
        ] {
            let s = PermissionPolicySpec {
                require_user_gesture: vec![p],
                ..PermissionPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: PermissionPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.require_user_gesture[0], p);
        }
    }

    #[test]
    fn decision_roundtrips_through_serde() {
        for d in [
            Decision::Allow,
            Decision::Block,
            Decision::Prompt,
            Decision::PromptEphemeral,
            Decision::RequireUserGesture,
            Decision::BlockWithBadge,
        ] {
            let s = PermissionPolicySpec {
                camera: d,
                ..PermissionPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: PermissionPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.camera, d);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = PermissionPolicyRegistry::new();
        reg.insert(PermissionPolicySpec::default_profile());
        reg.insert(PermissionPolicySpec {
            name: "meet".into(),
            host: "*://meet.google.com/*".into(),
            camera: Decision::Allow,
            microphone: Decision::Allow,
            ..PermissionPolicySpec::default_profile()
        });
        let meet = reg.resolve("meet.google.com").unwrap();
        assert_eq!(meet.camera, Decision::Allow);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = PermissionPolicyRegistry::new();
        reg.insert(PermissionPolicySpec {
            enabled: false,
            ..PermissionPolicySpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_permission_policy_form() {
        let src = r#"
            (defpermission-policy :name "strict"
                                  :host "*"
                                  :camera "block"
                                  :microphone "prompt"
                                  :geolocation "block"
                                  :notifications "block-with-badge"
                                  :clipboard-read "prompt-ephemeral"
                                  :midi "block"
                                  :usb "block"
                                  :require-user-gesture ("camera" "microphone"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.camera, Decision::Block);
        assert_eq!(s.microphone, Decision::Prompt);
        assert_eq!(s.notifications, Decision::BlockWithBadge);
        assert_eq!(s.clipboard_read, Decision::PromptEphemeral);
        assert!(s.requires_user_gesture(Permission::Camera));
    }
}
