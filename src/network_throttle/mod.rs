//! `(defnetwork-throttle)` — DevTools network throttling.
//!
//! Absorbs Chrome DevTools Network conditions, Firefox Developer
//! Tools throttling, Safari Web Inspector network throttling, and
//! Chrome's 3G/4G presets. One Lisp form declares a throttling
//! profile — bandwidth up/down, latency, packet loss, offline flag
//! — scoped to a host or applied globally.
//!
//! ```lisp
//! (defnetwork-throttle :name        "slow-3g"
//!                      :host        "*"
//!                      :preset      :slow-3g
//!                      :download-kbps 500
//!                      :upload-kbps   500
//!                      :latency-ms    400
//!                      :packet-loss-pct 0.0
//!                      :offline       #f)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Chrome DevTools preset flavors.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    /// Unthrottled — the knobs below are ignored.
    #[default]
    Unthrottled,
    /// Offline — every request fails.
    Offline,
    /// Slow 3G (Chrome DevTools preset: 500↓/500↑ kbps, 400 ms).
    Slow3G,
    /// Fast 3G (Chrome DevTools preset: 1.6↓/750↑ Mbps, 150 ms).
    Fast3G,
    /// Regular 4G — ~4↓/3↑ Mbps, 20 ms.
    Regular4G,
    /// Good 4G — ~8↓/5↑ Mbps, 20 ms.
    Good4G,
    /// Wi-Fi — ~30↓/15↑ Mbps, 2 ms.
    Wifi,
    /// Cable — ~5↓/1↑ Mbps, 28 ms.
    Cable,
    /// DSL — ~1.5↓/0.38↑ Mbps, 50 ms.
    Dsl,
    /// Dial-up — 50↓/30↑ kbps, 500 ms.
    DialUp,
    /// Author-provided custom values in the spec fields.
    Custom,
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defnetwork-throttle"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkThrottleSpec {
    pub name: String,
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    #[serde(default)]
    pub preset: Preset,
    /// Download cap in kbps — ignored unless `preset = Custom`.
    #[serde(default)]
    pub download_kbps: u32,
    /// Upload cap in kbps — ignored unless `preset = Custom`.
    #[serde(default)]
    pub upload_kbps: u32,
    /// Round-trip latency in ms — ignored unless `preset = Custom`.
    #[serde(default)]
    pub latency_ms: u32,
    /// Packet loss percentage in `[0.0, 100.0]`. Always honored even
    /// with presets.
    #[serde(default)]
    pub packet_loss_pct: f32,
    /// Jitter in ms. Always honored.
    #[serde(default)]
    pub jitter_ms: u32,
    /// Offline override — wins over everything else when true.
    #[serde(default)]
    pub offline: bool,
    /// Per-request probability that the request times out — useful
    /// for chaos testing. `[0.0, 100.0]`.
    #[serde(default)]
    pub timeout_pct: f32,
    /// Hosts exempt from throttling (e.g. metrics endpoints).
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

/// Effective (download_kbps, upload_kbps, latency_ms) tuple after
/// preset expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effective {
    pub download_kbps: u32,
    pub upload_kbps: u32,
    pub latency_ms: u32,
}

impl NetworkThrottleSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "unthrottled".into(),
            host: "*".into(),
            preset: Preset::Unthrottled,
            download_kbps: 0,
            upload_kbps: 0,
            latency_ms: 0,
            packet_loss_pct: 0.0,
            jitter_ms: 0,
            offline: false,
            timeout_pct: 0.0,
            exempt_hosts: vec![],
            enabled: true,
            description: Some("Default — no throttling applied.".into()),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn is_exempt(&self, host: &str) -> bool {
        self.exempt_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Expand the preset into concrete (down, up, latency) numbers.
    /// `Offline` maps to (0,0,0); `Unthrottled` too — the `offline`
    /// flag is checked separately.
    #[must_use]
    pub fn effective(&self) -> Effective {
        match self.preset {
            Preset::Unthrottled | Preset::Offline => Effective {
                download_kbps: 0,
                upload_kbps: 0,
                latency_ms: 0,
            },
            Preset::Slow3G => Effective {
                download_kbps: 500,
                upload_kbps: 500,
                latency_ms: 400,
            },
            Preset::Fast3G => Effective {
                download_kbps: 1_600,
                upload_kbps: 750,
                latency_ms: 150,
            },
            Preset::Regular4G => Effective {
                download_kbps: 4_000,
                upload_kbps: 3_000,
                latency_ms: 20,
            },
            Preset::Good4G => Effective {
                download_kbps: 8_000,
                upload_kbps: 5_000,
                latency_ms: 20,
            },
            Preset::Wifi => Effective {
                download_kbps: 30_000,
                upload_kbps: 15_000,
                latency_ms: 2,
            },
            Preset::Cable => Effective {
                download_kbps: 5_000,
                upload_kbps: 1_000,
                latency_ms: 28,
            },
            Preset::Dsl => Effective {
                download_kbps: 1_500,
                upload_kbps: 380,
                latency_ms: 50,
            },
            Preset::DialUp => Effective {
                download_kbps: 50,
                upload_kbps: 30,
                latency_ms: 500,
            },
            Preset::Custom => Effective {
                download_kbps: self.download_kbps,
                upload_kbps: self.upload_kbps,
                latency_ms: self.latency_ms,
            },
        }
    }

    /// Should a request to `host` go through at all?
    /// Offline flag + Preset::Offline both block.
    #[must_use]
    pub fn admits(&self, host: &str) -> bool {
        if !self.enabled {
            return true;
        }
        if self.is_exempt(host) {
            return true;
        }
        !(self.offline || matches!(self.preset, Preset::Offline))
    }

    /// Clamped packet-loss in `[0.0, 100.0]`.
    #[must_use]
    pub fn clamped_packet_loss(&self) -> f32 {
        self.packet_loss_pct.clamp(0.0, 100.0)
    }

    /// Clamped timeout probability in `[0.0, 100.0]`.
    #[must_use]
    pub fn clamped_timeout(&self) -> f32 {
        self.timeout_pct.clamp(0.0, 100.0)
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct NetworkThrottleRegistry {
    specs: Vec<NetworkThrottleSpec>,
}

impl NetworkThrottleRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: NetworkThrottleSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = NetworkThrottleSpec>) {
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
    pub fn specs(&self) -> &[NetworkThrottleSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&NetworkThrottleSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<NetworkThrottleSpec>, String> {
    tatara_lisp::compile_typed::<NetworkThrottleSpec>(src)
        .map_err(|e| format!("failed to compile defnetwork-throttle forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<NetworkThrottleSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_unthrottled() {
        let s = NetworkThrottleSpec::default_profile();
        assert_eq!(s.preset, Preset::Unthrottled);
        let e = s.effective();
        assert_eq!(e.download_kbps, 0);
        assert_eq!(e.upload_kbps, 0);
        assert_eq!(e.latency_ms, 0);
    }

    #[test]
    fn effective_matches_chrome_slow_3g() {
        let s = NetworkThrottleSpec {
            preset: Preset::Slow3G,
            ..NetworkThrottleSpec::default_profile()
        };
        let e = s.effective();
        assert_eq!(e.download_kbps, 500);
        assert_eq!(e.upload_kbps, 500);
        assert_eq!(e.latency_ms, 400);
    }

    #[test]
    fn effective_matches_chrome_fast_3g() {
        let s = NetworkThrottleSpec {
            preset: Preset::Fast3G,
            ..NetworkThrottleSpec::default_profile()
        };
        let e = s.effective();
        assert_eq!(e.download_kbps, 1_600);
        assert_eq!(e.upload_kbps, 750);
        assert_eq!(e.latency_ms, 150);
    }

    #[test]
    fn effective_dialup_is_punishing() {
        let s = NetworkThrottleSpec {
            preset: Preset::DialUp,
            ..NetworkThrottleSpec::default_profile()
        };
        let e = s.effective();
        assert_eq!(e.download_kbps, 50);
        assert_eq!(e.latency_ms, 500);
    }

    #[test]
    fn effective_custom_returns_author_fields() {
        let s = NetworkThrottleSpec {
            preset: Preset::Custom,
            download_kbps: 2_500,
            upload_kbps: 1_000,
            latency_ms: 80,
            ..NetworkThrottleSpec::default_profile()
        };
        let e = s.effective();
        assert_eq!(e.download_kbps, 2_500);
        assert_eq!(e.upload_kbps, 1_000);
        assert_eq!(e.latency_ms, 80);
    }

    #[test]
    fn effective_unthrottled_ignores_author_values() {
        let s = NetworkThrottleSpec {
            preset: Preset::Unthrottled,
            download_kbps: 999,
            ..NetworkThrottleSpec::default_profile()
        };
        assert_eq!(s.effective().download_kbps, 0);
    }

    #[test]
    fn admits_offline_flag_blocks_requests() {
        let s = NetworkThrottleSpec {
            offline: true,
            ..NetworkThrottleSpec::default_profile()
        };
        assert!(!s.admits("example.com"));
    }

    #[test]
    fn admits_offline_preset_blocks_requests() {
        let s = NetworkThrottleSpec {
            preset: Preset::Offline,
            ..NetworkThrottleSpec::default_profile()
        };
        assert!(!s.admits("example.com"));
    }

    #[test]
    fn admits_exempt_hosts_bypass_offline() {
        let s = NetworkThrottleSpec {
            preset: Preset::Offline,
            exempt_hosts: vec!["*://*.metrics.com/*".into()],
            ..NetworkThrottleSpec::default_profile()
        };
        assert!(!s.admits("example.com"));
        assert!(s.admits("tracker.metrics.com"));
    }

    #[test]
    fn admits_disabled_profile_always_true() {
        let s = NetworkThrottleSpec {
            enabled: false,
            offline: true,
            ..NetworkThrottleSpec::default_profile()
        };
        assert!(s.admits("example.com"));
    }

    #[test]
    fn clamped_packet_loss_bounds() {
        let s = NetworkThrottleSpec {
            packet_loss_pct: 250.0,
            ..NetworkThrottleSpec::default_profile()
        };
        assert!((s.clamped_packet_loss() - 100.0).abs() < 1e-5);

        let neg = NetworkThrottleSpec {
            packet_loss_pct: -1.0,
            ..NetworkThrottleSpec::default_profile()
        };
        assert!(neg.clamped_packet_loss().abs() < 1e-5);
    }

    #[test]
    fn clamped_timeout_bounds() {
        let s = NetworkThrottleSpec {
            timeout_pct: 50.0,
            ..NetworkThrottleSpec::default_profile()
        };
        assert!((s.clamped_timeout() - 50.0).abs() < 1e-5);
        let over = NetworkThrottleSpec {
            timeout_pct: 1_000.0,
            ..NetworkThrottleSpec::default_profile()
        };
        assert!((over.clamped_timeout() - 100.0).abs() < 1e-5);
    }

    #[test]
    fn preset_roundtrips_through_serde() {
        for p in [
            Preset::Unthrottled,
            Preset::Offline,
            Preset::Slow3G,
            Preset::Fast3G,
            Preset::Regular4G,
            Preset::Good4G,
            Preset::Wifi,
            Preset::Cable,
            Preset::Dsl,
            Preset::DialUp,
            Preset::Custom,
        ] {
            let s = NetworkThrottleSpec {
                preset: p,
                ..NetworkThrottleSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: NetworkThrottleSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.preset, p);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = NetworkThrottleRegistry::new();
        reg.insert(NetworkThrottleSpec::default_profile());
        reg.insert(NetworkThrottleSpec {
            name: "news-slow".into(),
            host: "*://*.news.com/*".into(),
            preset: Preset::Slow3G,
            ..NetworkThrottleSpec::default_profile()
        });
        assert_eq!(
            reg.resolve("www.news.com").unwrap().preset,
            Preset::Slow3G
        );
        assert_eq!(
            reg.resolve("example.org").unwrap().preset,
            Preset::Unthrottled
        );
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = NetworkThrottleRegistry::new();
        reg.insert(NetworkThrottleSpec {
            enabled: false,
            ..NetworkThrottleSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_network_throttle_form() {
        let src = r#"
            (defnetwork-throttle :name "chaos"
                                 :host "*"
                                 :preset "custom"
                                 :download-kbps 2500
                                 :upload-kbps 800
                                 :latency-ms 80
                                 :packet-loss-pct 2.5
                                 :jitter-ms 25
                                 :timeout-pct 0.5
                                 :exempt-hosts ("*://*.metrics.example.com/*"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.preset, Preset::Custom);
        assert_eq!(s.download_kbps, 2_500);
        assert!((s.packet_loss_pct - 2.5).abs() < 1e-5);
        assert_eq!(s.exempt_hosts.len(), 1);
    }
}
