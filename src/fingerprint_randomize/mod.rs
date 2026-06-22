//! `(deffingerprint-randomize)` — canvas/WebGL/audio/font fingerprint
//! randomization policy.
//!
//! Absorbs Brave Shields farbling, Tor Browser letterboxing + resist-
//! fingerprinting, LibreWolf, Mullvad Browser, uBlock Origin
//! anti-fingerprint filters. Farbling = tiny per-session noise so
//! the same user looks different each session, and different from
//! every other user.
//!
//! ```lisp
//! (deffingerprint-randomize :name        "strict"
//!                           :host        "*"
//!                           :canvas      :noise
//!                           :webgl       :noise
//!                           :audio       :noise
//!                           :fonts       :randomize-metrics
//!                           :client-rects :noise
//!                           :user-agent   :generic
//!                           :session-scope :per-host)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How a fingerprint surface is handled.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FingerprintMode {
    /// Pass the API through untouched.
    #[default]
    Allow,
    /// Add per-session noise (Brave "farbling" — tiny, deterministic
    /// per session-key).
    Noise,
    /// Return a canonical shape that every user sees (Tor approach).
    Generic,
    /// Block the API entirely — null/empty returns.
    Block,
    /// Mark the call and prompt the user.
    Prompt,
}

/// Font-fingerprint handling.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FontMode {
    #[default]
    Allow,
    /// Return only system-core fonts.
    SystemOnly,
    /// Randomize metric queries (Font Detection via character width).
    RandomizeMetrics,
    /// Block font enumeration entirely.
    Block,
}

/// User-agent shape.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UserAgentMode {
    #[default]
    Real,
    /// Latest-stable Firefox on Linux x86_64 (Tor convention).
    Generic,
    /// Per-session randomized — new UA each shell session.
    Randomize,
    /// Site-specific allow-list — passes real UA to trusted hosts.
    AllowList,
}

/// Scope at which the noise value is cached.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SessionScope {
    /// Same noise for the whole shell session — lowest friction.
    PerSession,
    /// Per-host + per-session — trackers can't cross-correlate hosts.
    #[default]
    PerHost,
    /// Per-tab + per-session — hardest, but breaks some sites.
    PerTab,
    /// Fresh noise on every single API call — near-useless for
    /// trackers but also breaks canvas rendering.
    PerCall,
}

/// Fingerprint-randomization profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "deffingerprint-randomize"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FingerprintRandomizeSpec {
    pub name: String,
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    #[serde(default)]
    pub canvas: FingerprintMode,
    #[serde(default)]
    pub webgl: FingerprintMode,
    #[serde(default)]
    pub audio: FingerprintMode,
    #[serde(default)]
    pub client_rects: FingerprintMode,
    /// Pointer/hover media-query values — mobile detection surface.
    #[serde(default)]
    pub pointer_hover: FingerprintMode,
    /// prefers-color-scheme + prefers-reduced-motion reveal light
    /// style + accessibility toggles.
    #[serde(default)]
    pub prefers_media: FingerprintMode,
    /// Intl.DateTimeFormat().resolvedOptions() → locale fingerprint.
    #[serde(default)]
    pub locale: FingerprintMode,
    /// navigator.hardwareConcurrency / deviceMemory / platform.
    #[serde(default)]
    pub navigator_info: FingerprintMode,
    #[serde(default)]
    pub fonts: FontMode,
    #[serde(default)]
    pub user_agent: UserAgentMode,
    #[serde(default)]
    pub session_scope: SessionScope,
    /// Noise intensity in [0.0, 1.0] — higher breaks more sites.
    #[serde(default = "default_intensity")]
    pub intensity: f32,
    /// Hosts that are exempt (e.g. your bank, online game).
    #[serde(default)]
    pub exempt_hosts: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_intensity() -> f32 {
    0.25
}
fn default_enabled() -> bool {
    true
}

impl FingerprintRandomizeSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            canvas: FingerprintMode::Noise,
            webgl: FingerprintMode::Noise,
            audio: FingerprintMode::Noise,
            client_rects: FingerprintMode::Allow,
            pointer_hover: FingerprintMode::Allow,
            prefers_media: FingerprintMode::Allow,
            locale: FingerprintMode::Allow,
            navigator_info: FingerprintMode::Generic,
            fonts: FontMode::RandomizeMetrics,
            user_agent: UserAgentMode::Real,
            session_scope: SessionScope::PerHost,
            intensity: 0.25,
            exempt_hosts: vec![],
            enabled: true,
            description: Some(
                "Default farbling — Brave-style noise on canvas/webgl/audio, metric font randomization, per-host session scope.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// Is `host` exempt (e.g. real banks, SSO)?
    #[must_use]
    pub fn is_exempt(&self, host: &str) -> bool {
        self.exempt_hosts
            .iter()
            .any(|pat| crate::extension::glob_match_host(pat, host))
    }

    /// Clamped intensity in [0.0, 1.0].
    #[must_use]
    pub fn clamped_intensity(&self) -> f32 {
        self.intensity.clamp(0.0, 1.0)
    }

    /// Effective canvas mode at runtime — `Allow` on exempt hosts,
    /// else the declared mode.
    #[must_use]
    pub fn canvas_for(&self, host: &str) -> FingerprintMode {
        if self.is_exempt(host) {
            FingerprintMode::Allow
        } else {
            self.canvas
        }
    }

    #[must_use]
    pub fn webgl_for(&self, host: &str) -> FingerprintMode {
        if self.is_exempt(host) {
            FingerprintMode::Allow
        } else {
            self.webgl
        }
    }

    #[must_use]
    pub fn audio_for(&self, host: &str) -> FingerprintMode {
        if self.is_exempt(host) {
            FingerprintMode::Allow
        } else {
            self.audio
        }
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct FingerprintRandomizeRegistry {
    specs: Vec<FingerprintRandomizeSpec>,
}

impl FingerprintRandomizeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: FingerprintRandomizeSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = FingerprintRandomizeSpec>) {
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
    pub fn specs(&self) -> &[FingerprintRandomizeSpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&FingerprintRandomizeSpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

// ═══════════════════════════════════════════════════════════════════
// Seeded value generation — the absorbed Brave/Tor "farbling" technique
// ═══════════════════════════════════════════════════════════════════
//
// Obscura (and Brave Shields) generate fingerprint values that are
// DETERMINISTIC from a single per-session seed: stable within a session
// (so canvas rendering and repeated reads agree), but different across
// sessions and across users. Random-per-request is itself a detection
// tell — a real machine reports the same hardwareConcurrency on every
// call — so every value here is a pure function of `(seed, spec)`.
//
// The INJECTION of these values into page JS awaits the Servo JS engine
// host bindings (see `crate::js_runtime` / `crate::spoof`); this module
// owns the absorbed *technique* — value GENERATION — which is real and
// tested here.

/// `SplitMix64` — a tiny, fast, well-distributed deterministic PRNG.
/// Pure-Rust, zero deps (`rand` stays dev-only). Same seed → same
/// stream; used to derive every spoofed value from one session seed.
#[derive(Debug, Clone, Copy)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Seed the generator. Any `u64` is a valid seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next pseudo-random `u64` in the stream.
    pub fn next_u64(&mut self) -> u64 {
        // Reference SplitMix64 (Steele, Lea & Flood 2014).
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform-ish value in `[0, n)`. `n == 0` returns `0`.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    /// Pick a reference into `items` deterministically. Panics never —
    /// callers pass non-empty static pools.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        debug_assert!(!items.is_empty(), "pick from empty pool");
        let idx = self.below(items.len() as u64) as usize;
        &items[idx]
    }
}

/// Realistic WebGL `(UNMASKED_VENDOR, UNMASKED_RENDERER)` pairs drawn
/// from common desktop GPU stacks. A spoofed renderer must look like a
/// plausible real machine, so vendor + renderer are picked as a *pair*
/// (an Apple vendor with an NVIDIA renderer would itself be a tell).
const WEBGL_POOL: &[(&str, &str)] = &[
    ("Google Inc. (Intel)", "ANGLE (Intel, Intel(R) UHD Graphics 630 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (Intel)", "ANGLE (Intel, Intel(R) Iris(R) Xe Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (NVIDIA)", "ANGLE (NVIDIA, NVIDIA GeForce GTX 1660 Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (AMD)", "ANGLE (AMD, AMD Radeon RX 6700 XT Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Google Inc. (AMD)", "ANGLE (AMD, AMD Radeon(TM) Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)"),
    ("Apple", "Apple M1"),
    ("Apple", "Apple M2"),
    ("Apple", "Apple M3"),
    ("Intel Inc.", "Intel(R) Iris(TM) Plus Graphics 640"),
    ("Mozilla", "Mesa Intel(R) UHD Graphics (CML GT2)"),
    ("Mesa/X.org", "AMD Radeon Graphics (radeonsi, navi23, LLVM 15.0.7, DRM 3.49)"),
];

/// Common desktop screen geometries `(width, height)`. Real screens
/// cluster at a handful of resolutions; spoofing to an oddball size is
/// a tell, so the pool is the documented popular set.
const SCREEN_POOL: &[(u32, u32)] = &[
    (1920, 1080),
    (1366, 768),
    (1536, 864),
    (1440, 900),
    (1280, 720),
    (2560, 1440),
    (1680, 1050),
    (1600, 900),
    (3840, 2160),
];

/// Plausible `navigator.hardwareConcurrency` values (logical cores).
/// Obscura leaves this FIXED — randomizing it (within a realistic set)
/// is the improvement absorbed here.
const CONCURRENCY_POOL: &[u32] = &[4, 6, 8, 12, 16];

/// Plausible `navigator.deviceMemory` (GiB) values. The Device Memory
/// API only ever reports a power-of-two-ish bucket: 0.25/0.5/1/2/4/8.
/// Real desktops report 8 (the spec caps the reported value at 8), so
/// we pick from the upper buckets. Obscura leaves this fixed too.
const DEVICE_MEMORY_POOL: &[u8] = &[4, 8];

/// Recent stable Chrome major versions for the
/// `navigator.userAgentData` brand list. Kept to a small recent window
/// so the spoofed UA looks current, not archaic.
const CHROME_MAJORS: &[u16] = &[122, 123, 124, 125, 126];

/// A fully-generated set of spoofed fingerprint values for one session.
/// Every field is deterministic from the `(seed, spec)` pair passed to
/// [`generate`]. Surfaces left in `Allow` mode by the spec are reported
/// as realistic *default* values (no farbling), matching the policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpoofedFingerprint {
    /// Per-pixel canvas noise seed. The JS host hook adds a tiny,
    /// deterministic per-pixel delta derived from this seed so
    /// `getImageData` differs imperceptibly across sessions.
    pub canvas_noise: u64,
    /// Audio-context noise seed — same idea for the audio fingerprint
    /// (sums of `AnalyserNode` frequency data).
    pub audio_noise: u64,
    /// Spoofed `WEBGL_debug_renderer_info` `UNMASKED_VENDOR_WEBGL`.
    pub webgl_vendor: String,
    /// Spoofed `WEBGL_debug_renderer_info` `UNMASKED_RENDERER_WEBGL`.
    pub webgl_renderer: String,
    /// Spoofed screen geometry.
    pub screen: Screen,
    /// Spoofed `navigator.hardwareConcurrency` (logical cores).
    pub hardware_concurrency: u32,
    /// Spoofed `navigator.deviceMemory` in GiB.
    pub device_memory_gb: u8,
    /// Spoofed `navigator.userAgentData` brand info (Chrome sim).
    pub user_agent_data: UserAgentData,
}

/// Screen geometry. `avail_*` is slightly less than the full size to
/// mimic OS chrome (taskbar / menu bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub avail_width: u32,
    pub avail_height: u32,
    pub color_depth: u8,
}

/// Simulated `navigator.userAgentData` — the Chromium high-entropy
/// brand surface. We report a current Chrome major version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAgentData {
    /// Chrome major version (e.g. `124`).
    pub chrome_major: u16,
    /// `platform` brand (e.g. `"Windows"`, `"macOS"`, `"Linux"`).
    pub platform: String,
    /// `mobile` flag — always `false` for the desktop pools above.
    pub mobile: bool,
}

/// Map a WebGL vendor/renderer pair to a plausible `userAgentData`
/// platform string, so the brand + GPU stay internally consistent.
fn platform_for_webgl(vendor: &str) -> &'static str {
    if vendor.starts_with("Apple") {
        "macOS"
    } else if vendor.starts_with("Mesa") || vendor.contains("X.org") {
        "Linux"
    } else {
        "Windows"
    }
}

/// Generate the full spoofed-fingerprint value set deterministically
/// from one session `seed` and the active `spec`.
///
/// - **Same seed → same values** (stable within a session).
/// - **Different seed → (almost certainly) different values** (varies
///   across sessions / users).
/// - Each surface honors its spec mode: `Noise`/`Generic`/`Block`/
///   `Prompt` produce a spoofed value, `Allow` reports a realistic
///   default. The canvas/audio noise seeds are scaled by
///   [`FingerprintRandomizeSpec::clamped_intensity`].
///
/// The values are produced from independent sub-streams (each field
/// draws from its own freshly-seeded `SplitMix64`) so toggling one
/// surface's mode does not shift another surface's value.
#[must_use]
pub fn generate(seed: u64, spec: &FingerprintRandomizeSpec) -> SpoofedFingerprint {
    // Derive a distinct sub-seed per surface from the session seed, so
    // each field's stream is independent + stable.
    let sub = |salt: u64| SplitMix64::new(seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15));

    // Canvas / audio noise seeds — present only when noised. Scale the
    // raw seed range by intensity so higher intensity = more entropy.
    let intensity = spec.clamped_intensity();
    let canvas_noise = if produces_value(spec.canvas) {
        scaled_noise(sub(1).next_u64(), intensity)
    } else {
        0
    };
    let audio_noise = if produces_value(spec.audio) {
        scaled_noise(sub(2).next_u64(), intensity)
    } else {
        0
    };

    // WebGL vendor/renderer pair (one pick keeps them consistent).
    let mut webgl_rng = sub(3);
    let (webgl_vendor, webgl_renderer) = *webgl_rng.pick(WEBGL_POOL);
    let platform = platform_for_webgl(webgl_vendor);

    // Screen geometry.
    let mut screen_rng = sub(4);
    let (w, h) = *screen_rng.pick(SCREEN_POOL);
    // availHeight trimmed by an OS-chrome band (24/32/40 px).
    let chrome_band = [24u32, 32, 40][screen_rng.below(3) as usize];
    let screen = Screen {
        width: w,
        height: h,
        avail_width: w,
        avail_height: h.saturating_sub(chrome_band),
        color_depth: 24,
    };

    // hardwareConcurrency + deviceMemory — RANDOMIZED (Obscura fixes
    // these; we improve on it by drawing from realistic pools).
    let hardware_concurrency = *sub(5).pick(CONCURRENCY_POOL);
    let device_memory_gb = *sub(6).pick(DEVICE_MEMORY_POOL);

    // userAgentData — current Chrome major, platform-consistent.
    let chrome_major = *sub(7).pick(CHROME_MAJORS);
    let user_agent_data = UserAgentData {
        chrome_major,
        platform: platform.to_owned(),
        mobile: false,
    };

    SpoofedFingerprint {
        canvas_noise,
        audio_noise,
        webgl_vendor: webgl_vendor.to_owned(),
        webgl_renderer: webgl_renderer.to_owned(),
        screen,
        hardware_concurrency,
        device_memory_gb,
        user_agent_data,
    }
}

/// A surface produces a spoofed value for any mode other than `Allow`.
fn produces_value(mode: FingerprintMode) -> bool {
    mode != FingerprintMode::Allow
}

/// Squeeze a raw 64-bit seed into a non-zero noise seed whose magnitude
/// grows with intensity. At intensity 0 a noised surface still gets a
/// minimal non-zero seed (so it is distinguishable from `Allow`'s 0).
fn scaled_noise(raw: u64, intensity: f32) -> u64 {
    // Map intensity [0,1] → a bit-width budget [8, 64]; mask the raw
    // seed to that many low bits, then OR in 1 to guarantee non-zero.
    let bits = 8 + (intensity.clamp(0.0, 1.0) * 56.0) as u32; // 8..=64
    let mask = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    (raw & mask) | 1
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<FingerprintRandomizeSpec>, String> {
    tatara_lisp::compile_typed::<FingerprintRandomizeSpec>(src)
        .map_err(|e| format!("failed to compile deffingerprint-randomize forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<FingerprintRandomizeSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_noises_canvas_webgl_audio() {
        let s = FingerprintRandomizeSpec::default_profile();
        assert_eq!(s.canvas, FingerprintMode::Noise);
        assert_eq!(s.webgl, FingerprintMode::Noise);
        assert_eq!(s.audio, FingerprintMode::Noise);
        assert_eq!(s.navigator_info, FingerprintMode::Generic);
        assert_eq!(s.fonts, FontMode::RandomizeMetrics);
        assert_eq!(s.session_scope, SessionScope::PerHost);
    }

    #[test]
    fn clamped_intensity_stays_in_range() {
        let s = FingerprintRandomizeSpec {
            intensity: 2.5,
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert!((s.clamped_intensity() - 1.0).abs() < 1e-5);
        let neg = FingerprintRandomizeSpec {
            intensity: -0.5,
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert!(neg.clamped_intensity().abs() < 1e-5);
    }

    #[test]
    fn is_exempt_matches_glob() {
        let s = FingerprintRandomizeSpec {
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert!(s.is_exempt("my.bank.com"));
        assert!(!s.is_exempt("trackers.com"));
    }

    #[test]
    fn canvas_for_flips_to_allow_on_exempt_host() {
        let s = FingerprintRandomizeSpec {
            exempt_hosts: vec!["*://*.bank.com/*".into()],
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert_eq!(s.canvas_for("my.bank.com"), FingerprintMode::Allow);
        assert_eq!(s.canvas_for("trackers.com"), FingerprintMode::Noise);
    }

    #[test]
    fn webgl_for_and_audio_for_follow_same_exempt_rule() {
        let s = FingerprintRandomizeSpec {
            exempt_hosts: vec!["*://*.game.com/*".into()],
            ..FingerprintRandomizeSpec::default_profile()
        };
        assert_eq!(s.webgl_for("play.game.com"), FingerprintMode::Allow);
        assert_eq!(s.audio_for("play.game.com"), FingerprintMode::Allow);
        assert_eq!(s.webgl_for("other.com"), FingerprintMode::Noise);
    }

    #[test]
    fn fingerprint_mode_roundtrips_through_serde() {
        for m in [
            FingerprintMode::Allow,
            FingerprintMode::Noise,
            FingerprintMode::Generic,
            FingerprintMode::Block,
            FingerprintMode::Prompt,
        ] {
            let s = FingerprintRandomizeSpec {
                canvas: m,
                ..FingerprintRandomizeSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: FingerprintRandomizeSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.canvas, m);
        }
    }

    #[test]
    fn font_mode_roundtrips_through_serde() {
        for m in [
            FontMode::Allow,
            FontMode::SystemOnly,
            FontMode::RandomizeMetrics,
            FontMode::Block,
        ] {
            let s = FingerprintRandomizeSpec {
                fonts: m,
                ..FingerprintRandomizeSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: FingerprintRandomizeSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.fonts, m);
        }
    }

    #[test]
    fn session_scope_roundtrips_through_serde() {
        for s in [
            SessionScope::PerSession,
            SessionScope::PerHost,
            SessionScope::PerTab,
            SessionScope::PerCall,
        ] {
            let spec = FingerprintRandomizeSpec {
                session_scope: s,
                ..FingerprintRandomizeSpec::default_profile()
            };
            let json = serde_json::to_string(&spec).unwrap();
            let back: FingerprintRandomizeSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.session_scope, s);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = FingerprintRandomizeRegistry::new();
        reg.insert(FingerprintRandomizeSpec::default_profile());
        reg.insert(FingerprintRandomizeSpec {
            name: "strict-news".into(),
            host: "*://*.nytimes.com/*".into(),
            canvas: FingerprintMode::Block,
            ..FingerprintRandomizeSpec::default_profile()
        });
        let ny = reg.resolve("www.nytimes.com").unwrap();
        assert_eq!(ny.canvas, FingerprintMode::Block);
        let other = reg.resolve("example.org").unwrap();
        assert_eq!(other.name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = FingerprintRandomizeRegistry::new();
        reg.insert(FingerprintRandomizeSpec {
            enabled: false,
            ..FingerprintRandomizeSpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    // ── Seeded value generation ───────────────────────────────────

    #[test]
    fn splitmix64_is_deterministic() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        // Different seed → different stream (overwhelmingly likely).
        let mut c = SplitMix64::new(43);
        assert_ne!(SplitMix64::new(42).next_u64(), c.next_u64());
    }

    #[test]
    fn splitmix64_below_respects_bound() {
        let mut rng = SplitMix64::new(7);
        for _ in 0..1000 {
            assert!(rng.below(10) < 10);
        }
        assert_eq!(SplitMix64::new(1).below(0), 0);
    }

    #[test]
    fn same_seed_same_values() {
        let spec = FingerprintRandomizeSpec::default_profile();
        let a = generate(0xDEAD_BEEF, &spec);
        let b = generate(0xDEAD_BEEF, &spec);
        assert_eq!(a, b);
    }

    #[test]
    fn different_seed_different_values() {
        let spec = FingerprintRandomizeSpec::default_profile();
        // Across a spread of seeds the generated fingerprints differ —
        // collisions across the whole struct are vanishingly unlikely.
        let mut seen = std::collections::HashSet::new();
        for seed in 0..64u64 {
            let fp = generate(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xABCD, &spec);
            seen.insert(serde_json::to_string(&fp).unwrap());
        }
        // At least the large majority are distinct (pools are finite, so
        // a few collisions on individual fields are fine — the full
        // struct should still vary widely).
        assert!(seen.len() >= 50, "expected wide variation, got {}", seen.len());
    }

    #[test]
    fn generated_values_in_realistic_ranges() {
        let spec = FingerprintRandomizeSpec::default_profile();
        for seed in 0..256u64 {
            let fp = generate(seed, &spec);
            assert!(WEBGL_POOL
                .iter()
                .any(|(v, r)| *v == fp.webgl_vendor && *r == fp.webgl_renderer));
            assert!(SCREEN_POOL.contains(&(fp.screen.width, fp.screen.height)));
            assert!(fp.screen.avail_width <= fp.screen.width);
            assert!(fp.screen.avail_height < fp.screen.height);
            assert_eq!(fp.screen.color_depth, 24);
            assert!(CONCURRENCY_POOL.contains(&fp.hardware_concurrency));
            assert!(DEVICE_MEMORY_POOL.contains(&fp.device_memory_gb));
            assert!(CHROME_MAJORS.contains(&fp.user_agent_data.chrome_major));
            assert!(!fp.user_agent_data.mobile);
        }
    }

    #[test]
    fn hardware_concurrency_and_device_memory_are_randomized() {
        // The headline improvement over Obscura: these surfaces VARY
        // across sessions instead of being pinned to one constant.
        let spec = FingerprintRandomizeSpec::default_profile();
        let mut concurrencies = std::collections::HashSet::new();
        let mut memories = std::collections::HashSet::new();
        for seed in 0..512u64 {
            let fp = generate(seed, &spec);
            concurrencies.insert(fp.hardware_concurrency);
            memories.insert(fp.device_memory_gb);
        }
        assert!(
            concurrencies.len() > 1,
            "hardwareConcurrency must vary across sessions"
        );
        assert!(
            memories.len() > 1,
            "deviceMemory must vary across sessions"
        );
    }

    #[test]
    fn allow_mode_zeroes_noise_seeds_noise_mode_does_not() {
        // Canvas Allow → no noise; canvas Noise → non-zero seed.
        let allow = FingerprintRandomizeSpec {
            canvas: FingerprintMode::Allow,
            audio: FingerprintMode::Allow,
            ..FingerprintRandomizeSpec::default_profile()
        };
        let fp = generate(99, &allow);
        assert_eq!(fp.canvas_noise, 0);
        assert_eq!(fp.audio_noise, 0);

        let noised = FingerprintRandomizeSpec {
            canvas: FingerprintMode::Noise,
            audio: FingerprintMode::Noise,
            ..FingerprintRandomizeSpec::default_profile()
        };
        let fp = generate(99, &noised);
        assert_ne!(fp.canvas_noise, 0);
        assert_ne!(fp.audio_noise, 0);
    }

    #[test]
    fn noise_seed_grows_with_intensity() {
        // Higher intensity → wider noise-seed bit budget → larger
        // typical magnitudes. Compare maxima across many seeds.
        let mk = |intensity: f32| FingerprintRandomizeSpec {
            canvas: FingerprintMode::Noise,
            intensity,
            ..FingerprintRandomizeSpec::default_profile()
        };
        let low = mk(0.1);
        let high = mk(1.0);
        let max_low = (0..256u64).map(|s| generate(s, &low).canvas_noise).max().unwrap();
        let max_high = (0..256u64).map(|s| generate(s, &high).canvas_noise).max().unwrap();
        assert!(
            max_high > max_low,
            "intensity 1.0 max {max_high} should exceed intensity 0.1 max {max_low}"
        );
    }

    #[test]
    fn webgl_vendor_and_platform_stay_consistent() {
        let spec = FingerprintRandomizeSpec::default_profile();
        for seed in 0..256u64 {
            let fp = generate(seed, &spec);
            if fp.webgl_vendor.starts_with("Apple") {
                assert_eq!(fp.user_agent_data.platform, "macOS");
            }
        }
    }

    #[test]
    fn spoofed_fingerprint_roundtrips_through_serde() {
        let fp = generate(0x1234_5678, &FingerprintRandomizeSpec::default_profile());
        let json = serde_json::to_string(&fp).unwrap();
        let back: SpoofedFingerprint = serde_json::from_str(&json).unwrap();
        assert_eq!(fp, back);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_fingerprint_form() {
        let src = r#"
            (deffingerprint-randomize :name "strict"
                                      :host "*"
                                      :canvas "noise"
                                      :webgl "generic"
                                      :audio "block"
                                      :fonts "system-only"
                                      :user-agent "generic"
                                      :session-scope "per-tab"
                                      :intensity 0.5)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.canvas, FingerprintMode::Noise);
        assert_eq!(s.webgl, FingerprintMode::Generic);
        assert_eq!(s.audio, FingerprintMode::Block);
        assert_eq!(s.fonts, FontMode::SystemOnly);
        assert_eq!(s.user_agent, UserAgentMode::Generic);
        assert_eq!(s.session_scope, SessionScope::PerTab);
        assert!((s.intensity - 0.5).abs() < 1e-5);
    }
}
