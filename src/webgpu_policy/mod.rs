//! `(defwebgpu-policy)` — per-host WebGPU access + adapter disclosure.
//!
//! Absorbs Chrome's WebGPU origin-trial policy, Firefox's WebGPU
//! gating, Safari's Model element permissioning, plus the
//! GPUAdapterInfo → fingerprint concern that Brave + Tor already
//! raised for WebGL. One Lisp form declares what WebGPU surfaces a
//! host sees.
//!
//! ```lisp
//! (defwebgpu-policy :name           "default"
//!                   :host           "*"
//!                   :access         :allow-with-prompt
//!                   :adapter-info   :generic
//!                   :compute        :block
//!                   :max-buffer-mb  256
//!                   :timestamp-query #f
//!                   :shader-feature-set :web-standard)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// What happens when a page calls `navigator.gpu.requestAdapter()`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GpuAccess {
    /// Grant without asking.
    Allow,
    /// Grant after a one-time user prompt (per host).
    #[default]
    AllowWithPrompt,
    /// Grant but disable compute shaders (render-only).
    AllowRenderOnly,
    /// Return a software-rasterizer fallback.
    SoftwareFallback,
    /// Reject the call.
    Block,
}

/// How GPUAdapterInfo is filled in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterInfoDisclosure {
    /// Full detail — vendor/architecture/device/description strings.
    Full,
    /// Vendor + "generic" architecture, everything else empty.
    #[default]
    Generic,
    /// All fields empty — maximum anti-fingerprint.
    Empty,
    /// User-selected pre-canned shape (e.g. "latest Apple M-series").
    Masquerade,
}

/// Compute-shader policy. Compute is the canonical crypto-mining +
/// fingerprinting surface so it gets its own toggle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ComputePolicy {
    /// Full compute access.
    Allow,
    /// Allow but throttle to a small workgroup count / fuel limit.
    Throttled,
    /// Only allow compute in secure contexts (HTTPS + isolated).
    SecureContextOnly,
    /// Compute is dropped.
    #[default]
    Block,
}

/// Shader feature set — controls which optional GPU features pages
/// can request (timestamp-query, indirect-first-instance, …).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ShaderFeatureSet {
    /// Only baseline WebGPU spec features.
    #[default]
    WebStandard,
    /// Spec + stable-optional features.
    Extended,
    /// Spec + experimental features.
    Experimental,
    /// No features past the mandatory minimum.
    Minimal,
}

/// WebGPU policy profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defwebgpu-policy"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebgpuPolicySpec {
    pub name: String,
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    #[serde(default)]
    pub access: GpuAccess,
    #[serde(default)]
    pub adapter_info: AdapterInfoDisclosure,
    /// Masquerade shape (vendor/architecture) used only when
    /// `adapter_info = masquerade`.
    #[serde(default)]
    pub masquerade_vendor: Option<String>,
    #[serde(default)]
    pub masquerade_architecture: Option<String>,
    #[serde(default)]
    pub compute: ComputePolicy,
    /// Max GPU buffer size in MB. 0 = spec default.
    #[serde(default)]
    pub max_buffer_mb: u32,
    /// Max total GPU memory a page can hold (MB). 0 = unlimited.
    #[serde(default)]
    pub max_total_memory_mb: u32,
    /// Allow timestamp-query extension (precision clock = fingerprint
    /// + side-channel risk).
    #[serde(default)]
    pub timestamp_query: bool,
    /// Allow `shader-f16` feature (smaller but different precision
    /// fingerprint).
    #[serde(default)]
    pub shader_f16: bool,
    #[serde(default)]
    pub shader_feature_set: ShaderFeatureSet,
    /// Hosts that always get full WebGPU (games, modeling apps).
    #[serde(default)]
    pub allow_hosts: Vec<String>,
    /// Hosts that never get WebGPU (force block).
    #[serde(default)]
    pub block_hosts: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl WebgpuPolicySpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            host: "*".into(),
            access: GpuAccess::AllowWithPrompt,
            adapter_info: AdapterInfoDisclosure::Generic,
            masquerade_vendor: None,
            masquerade_architecture: None,
            compute: ComputePolicy::Block,
            max_buffer_mb: 0,
            max_total_memory_mb: 0,
            timestamp_query: false,
            shader_f16: false,
            shader_feature_set: ShaderFeatureSet::WebStandard,
            allow_hosts: vec![],
            block_hosts: vec![],
            enabled: true,
            description: Some(
                "Default WebGPU — prompt-on-first-use, generic adapter info, compute blocked.".into(),
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

    /// Effective access for `host` after allow/block overrides.
    #[must_use]
    pub fn access_for(&self, host: &str) -> GpuAccess {
        if self.is_blocked(host) {
            GpuAccess::Block
        } else if self.is_allowed(host) {
            GpuAccess::Allow
        } else {
            self.access
        }
    }

    /// Effective compute policy — block-listed hosts always get Block.
    #[must_use]
    pub fn compute_for(&self, host: &str) -> ComputePolicy {
        if self.is_blocked(host) {
            ComputePolicy::Block
        } else {
            self.compute
        }
    }

    /// Is this buffer size within the allowed cap?
    #[must_use]
    pub fn accepts_buffer_bytes(&self, bytes: u64) -> bool {
        if self.max_buffer_mb == 0 {
            return true;
        }
        bytes <= u64::from(self.max_buffer_mb) * 1024 * 1024
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct WebgpuPolicyRegistry {
    specs: Vec<WebgpuPolicySpec>,
}

impl WebgpuPolicyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: WebgpuPolicySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = WebgpuPolicySpec>) {
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
    pub fn specs(&self) -> &[WebgpuPolicySpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&WebgpuPolicySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<WebgpuPolicySpec>, String> {
    tatara_lisp::compile_typed::<WebgpuPolicySpec>(src)
        .map_err(|e| format!("failed to compile defwebgpu-policy forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<WebgpuPolicySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_prompts_and_generalizes_adapter_info() {
        let s = WebgpuPolicySpec::default_profile();
        assert_eq!(s.access, GpuAccess::AllowWithPrompt);
        assert_eq!(s.adapter_info, AdapterInfoDisclosure::Generic);
        assert_eq!(s.compute, ComputePolicy::Block);
        assert!(!s.timestamp_query);
    }

    #[test]
    fn access_for_is_block_on_block_hosts() {
        let s = WebgpuPolicySpec {
            access: GpuAccess::Allow,
            block_hosts: vec!["*://*.tracker.com/*".into()],
            ..WebgpuPolicySpec::default_profile()
        };
        assert_eq!(s.access_for("scripts.tracker.com"), GpuAccess::Block);
        assert_eq!(s.access_for("example.com"), GpuAccess::Allow);
    }

    #[test]
    fn access_for_is_allow_on_allow_hosts() {
        let s = WebgpuPolicySpec {
            access: GpuAccess::AllowWithPrompt,
            allow_hosts: vec!["*://*.figma.com/*".into()],
            ..WebgpuPolicySpec::default_profile()
        };
        assert_eq!(s.access_for("www.figma.com"), GpuAccess::Allow);
        assert_eq!(s.access_for("example.com"), GpuAccess::AllowWithPrompt);
    }

    #[test]
    fn block_host_beats_allow_host_when_both_match() {
        // Precedence: block wins — user safety first.
        let s = WebgpuPolicySpec {
            access: GpuAccess::Allow,
            allow_hosts: vec!["*://*.example.com/*".into()],
            block_hosts: vec!["*://ads.example.com/*".into()],
            ..WebgpuPolicySpec::default_profile()
        };
        assert_eq!(s.access_for("ads.example.com"), GpuAccess::Block);
        assert_eq!(s.access_for("www.example.com"), GpuAccess::Allow);
    }

    #[test]
    fn compute_for_is_block_on_block_hosts_regardless_of_policy() {
        let s = WebgpuPolicySpec {
            compute: ComputePolicy::Allow,
            block_hosts: vec!["*://*.tracker.com/*".into()],
            ..WebgpuPolicySpec::default_profile()
        };
        assert_eq!(s.compute_for("scripts.tracker.com"), ComputePolicy::Block);
        assert_eq!(s.compute_for("example.com"), ComputePolicy::Allow);
    }

    #[test]
    fn accepts_buffer_bytes_respects_cap() {
        let s = WebgpuPolicySpec {
            max_buffer_mb: 1,
            ..WebgpuPolicySpec::default_profile()
        };
        assert!(s.accepts_buffer_bytes(1024));
        assert!(s.accepts_buffer_bytes(1024 * 1024));
        assert!(!s.accepts_buffer_bytes(2 * 1024 * 1024));

        let unlimited = WebgpuPolicySpec {
            max_buffer_mb: 0,
            ..WebgpuPolicySpec::default_profile()
        };
        assert!(unlimited.accepts_buffer_bytes(u64::MAX));
    }

    #[test]
    fn access_roundtrips_through_serde() {
        for a in [
            GpuAccess::Allow,
            GpuAccess::AllowWithPrompt,
            GpuAccess::AllowRenderOnly,
            GpuAccess::SoftwareFallback,
            GpuAccess::Block,
        ] {
            let s = WebgpuPolicySpec {
                access: a,
                ..WebgpuPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: WebgpuPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.access, a);
        }
    }

    #[test]
    fn adapter_info_roundtrips_through_serde() {
        for a in [
            AdapterInfoDisclosure::Full,
            AdapterInfoDisclosure::Generic,
            AdapterInfoDisclosure::Empty,
            AdapterInfoDisclosure::Masquerade,
        ] {
            let s = WebgpuPolicySpec {
                adapter_info: a,
                ..WebgpuPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: WebgpuPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.adapter_info, a);
        }
    }

    #[test]
    fn compute_policy_roundtrips_through_serde() {
        for c in [
            ComputePolicy::Allow,
            ComputePolicy::Throttled,
            ComputePolicy::SecureContextOnly,
            ComputePolicy::Block,
        ] {
            let s = WebgpuPolicySpec {
                compute: c,
                ..WebgpuPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: WebgpuPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.compute, c);
        }
    }

    #[test]
    fn shader_feature_set_roundtrips_through_serde() {
        for f in [
            ShaderFeatureSet::WebStandard,
            ShaderFeatureSet::Extended,
            ShaderFeatureSet::Experimental,
            ShaderFeatureSet::Minimal,
        ] {
            let s = WebgpuPolicySpec {
                shader_feature_set: f,
                ..WebgpuPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: WebgpuPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.shader_feature_set, f);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = WebgpuPolicyRegistry::new();
        reg.insert(WebgpuPolicySpec::default_profile());
        reg.insert(WebgpuPolicySpec {
            name: "figma".into(),
            host: "*://*.figma.com/*".into(),
            access: GpuAccess::Allow,
            compute: ComputePolicy::Allow,
            ..WebgpuPolicySpec::default_profile()
        });
        assert_eq!(reg.resolve("www.figma.com").unwrap().compute, ComputePolicy::Allow);
        assert_eq!(reg.resolve("example.org").unwrap().name, "default");
    }

    #[test]
    fn disabled_profile_never_resolves() {
        let mut reg = WebgpuPolicyRegistry::new();
        reg.insert(WebgpuPolicySpec {
            enabled: false,
            ..WebgpuPolicySpec::default_profile()
        });
        assert!(reg.resolve("example.com").is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_webgpu_policy_form() {
        let src = r#"
            (defwebgpu-policy :name "default"
                              :host "*"
                              :access "allow-with-prompt"
                              :adapter-info "generic"
                              :compute "block"
                              :max-buffer-mb 256
                              :max-total-memory-mb 1024
                              :timestamp-query #f
                              :shader-f16 #f
                              :shader-feature-set "web-standard"
                              :allow-hosts ("*://*.figma.com/*"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.access, GpuAccess::AllowWithPrompt);
        assert_eq!(s.adapter_info, AdapterInfoDisclosure::Generic);
        assert_eq!(s.compute, ComputePolicy::Block);
        assert_eq!(s.max_buffer_mb, 256);
        assert_eq!(s.shader_feature_set, ShaderFeatureSet::WebStandard);
        assert_eq!(s.allow_hosts.len(), 1);
    }
}
