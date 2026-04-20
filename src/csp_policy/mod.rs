//! `(defcsp-policy)` — typed Content-Security-Policy builder.
//!
//! **Novel** — browsers ship CSP as an opaque HTTP header. No
//! mainstream browser gives authors a typed build API for their own
//! page's CSP, let alone one that enforces the mutual-exclusion
//! invariants CSP Level 3 bakes in (e.g. `'unsafe-inline'` is
//! ignored when `'nonce-…'` or `'strict-dynamic'` is present). This
//! module declares one. The spec captures every directive as a
//! typed field; `render()` emits a spec-valid header value with
//! canonical ordering; `validate()` returns errors for the known
//! foot-guns.
//!
//! ```lisp
//! (defcsp-policy :name      "strict"
//!                :host      "*"
//!                :default-src  (self_)
//!                :script-src   (self_ (nonce "abc123") strict-dynamic)
//!                :style-src    (self_ (sha256 "xyzhash=="))
//!                :img-src      (self_ data scheme-https)
//!                :connect-src  (self_ (origin "https://api.example.com"))
//!                :object-src   (none)
//!                :report-uri   "/csp-report"
//!                :upgrade-insecure-requests #t
//!                :block-all-mixed-content   #t
//!                :mode         :enforce)
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// CSP source expression — strongly typed instead of the usual
/// string-templated style.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// `'none'` — bans everything for the directive.
    None,
    /// `'self'` — the document's own origin.
    Self_,
    /// `'unsafe-inline'` — required for inline `<style>`/`<script>`
    /// when no nonce + no strict-dynamic.
    UnsafeInline,
    /// `'unsafe-eval'` — `eval()` + `new Function(...)`.
    UnsafeEval,
    /// `'strict-dynamic'` — CSP3 hardening; pair with Nonce.
    StrictDynamic,
    /// `'unsafe-hashes'` — allow inline event handlers matching a
    /// listed hash.
    UnsafeHashes,
    /// `'wasm-unsafe-eval'` — WebAssembly compile + instantiate.
    WasmUnsafeEval,
    /// Exact origin, e.g. `"https://api.example.com"`.
    Origin(String),
    /// Scheme, e.g. `"https"`, `"data"`, `"blob"`.
    Scheme(String),
    /// Nonce value (just the random, not the `'nonce-'` prefix).
    Nonce(String),
    /// SHA-256 base64 digest (just the digest, not the prefix).
    Sha256(String),
    Sha384(String),
    Sha512(String),
    /// Wildcard `*` — any origin. Strongly discouraged.
    Wildcard,
    /// Reporting endpoint name for `report-to` values only. Ignored
    /// in source-list directives.
    ReportTo(String),
}

/// Directive kinds included in the Level 3 spec.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Directive {
    DefaultSrc,
    ScriptSrc,
    ScriptSrcElem,
    ScriptSrcAttr,
    StyleSrc,
    StyleSrcElem,
    StyleSrcAttr,
    ImgSrc,
    ConnectSrc,
    FontSrc,
    ObjectSrc,
    MediaSrc,
    FrameSrc,
    WorkerSrc,
    ChildSrc,
    ManifestSrc,
    PrefetchSrc,
    FormAction,
    FrameAncestors,
    BaseUri,
    Sandbox,
    NavigateTo,
    TrustedTypes,
    RequireTrustedTypesFor,
}

/// Enforce vs report-only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Enforce,
    ReportOnly,
}

/// Validation error surface.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum CspError {
    #[error("{directive:?}: 'unsafe-inline' is present alongside a nonce — UnsafeInline will be ignored by the browser")]
    UnsafeInlineWithNonce { directive: String },
    #[error("{directive:?}: 'unsafe-inline' is present alongside 'strict-dynamic' — UnsafeInline will be ignored")]
    UnsafeInlineWithStrictDynamic { directive: String },
    #[error("{directive:?}: Wildcard + Self_ is redundant")]
    WildcardAndSelf { directive: String },
    #[error("{directive:?}: 'strict-dynamic' without a Nonce or hash — it has no whitelist to bootstrap from")]
    StrictDynamicWithoutBootstrap { directive: String },
    #[error("report_uri set but report-to is the Level 3 replacement — consider migrating")]
    ReportUriDeprecated,
    #[error("source {value:?} in origin field contains whitespace — CSP tokens cannot be broken")]
    OriginWithWhitespace { value: String },
    #[error("sandbox token {token:?} is not in the spec-valid set")]
    InvalidSandboxToken { token: String },
}

/// Profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defcsp-policy"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CspPolicySpec {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub default_src: Vec<Source>,
    #[serde(default)]
    pub script_src: Vec<Source>,
    #[serde(default)]
    pub script_src_elem: Vec<Source>,
    #[serde(default)]
    pub script_src_attr: Vec<Source>,
    #[serde(default)]
    pub style_src: Vec<Source>,
    #[serde(default)]
    pub style_src_elem: Vec<Source>,
    #[serde(default)]
    pub style_src_attr: Vec<Source>,
    #[serde(default)]
    pub img_src: Vec<Source>,
    #[serde(default)]
    pub connect_src: Vec<Source>,
    #[serde(default)]
    pub font_src: Vec<Source>,
    #[serde(default)]
    pub object_src: Vec<Source>,
    #[serde(default)]
    pub media_src: Vec<Source>,
    #[serde(default)]
    pub frame_src: Vec<Source>,
    #[serde(default)]
    pub worker_src: Vec<Source>,
    #[serde(default)]
    pub child_src: Vec<Source>,
    #[serde(default)]
    pub manifest_src: Vec<Source>,
    #[serde(default)]
    pub prefetch_src: Vec<Source>,
    #[serde(default)]
    pub form_action: Vec<Source>,
    #[serde(default)]
    pub frame_ancestors: Vec<Source>,
    #[serde(default)]
    pub base_uri: Vec<Source>,
    /// Sandbox tokens — e.g. `["allow-forms", "allow-scripts"]`.
    #[serde(default)]
    pub sandbox: Vec<String>,
    #[serde(default)]
    pub navigate_to: Vec<Source>,
    #[serde(default)]
    pub trusted_types: Vec<String>,
    /// Directives requiring Trusted Types — usually `["script"]`.
    #[serde(default)]
    pub require_trusted_types_for: Vec<String>,
    /// Report-to group name (Level 3).
    #[serde(default)]
    pub report_to: Option<String>,
    /// Report-uri — deprecated in Level 3 but still honored.
    #[serde(default)]
    pub report_uri: Option<String>,
    #[serde(default)]
    pub upgrade_insecure_requests: bool,
    #[serde(default)]
    pub block_all_mixed_content: bool,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_enabled() -> bool {
    true
}

const SANDBOX_TOKENS: &[&str] = &[
    "allow-downloads",
    "allow-forms",
    "allow-modals",
    "allow-orientation-lock",
    "allow-pointer-lock",
    "allow-popups",
    "allow-popups-to-escape-sandbox",
    "allow-presentation",
    "allow-same-origin",
    "allow-scripts",
    "allow-storage-access-by-user-activation",
    "allow-top-navigation",
    "allow-top-navigation-by-user-activation",
    "allow-top-navigation-to-custom-protocols",
];

impl CspPolicySpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "strict".into(),
            host: "*".into(),
            default_src: vec![Source::Self_],
            script_src: vec![Source::Self_],
            script_src_elem: vec![],
            script_src_attr: vec![],
            style_src: vec![Source::Self_],
            style_src_elem: vec![],
            style_src_attr: vec![],
            img_src: vec![Source::Self_, Source::Scheme("data".into())],
            connect_src: vec![Source::Self_],
            font_src: vec![Source::Self_, Source::Scheme("data".into())],
            object_src: vec![Source::None],
            media_src: vec![Source::Self_],
            frame_src: vec![Source::Self_],
            worker_src: vec![Source::Self_],
            child_src: vec![],
            manifest_src: vec![Source::Self_],
            prefetch_src: vec![],
            form_action: vec![Source::Self_],
            frame_ancestors: vec![Source::None],
            base_uri: vec![Source::Self_],
            sandbox: vec![],
            navigate_to: vec![],
            trusted_types: vec![],
            require_trusted_types_for: vec![],
            report_to: None,
            report_uri: None,
            upgrade_insecure_requests: true,
            block_all_mixed_content: true,
            mode: Mode::Enforce,
            enabled: true,
            description: Some(
                "Default strict CSP — self-only for everything, object-src 'none', frame-ancestors 'none', upgrade-insecure.".into(),
            ),
        }
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        if self.host.is_empty() || self.host == "*" {
            return true;
        }
        crate::extension::glob_match_host(&self.host, host)
    }

    /// Render to canonical header value. Caller chooses
    /// `Content-Security-Policy` vs `-Report-Only` based on `mode`.
    #[must_use]
    pub fn render(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        // Canonical directive order — matches MDN's documentation
        // ordering, keeps diffs small.
        push(&mut parts, "default-src", &self.default_src);
        push(&mut parts, "script-src", &self.script_src);
        push(&mut parts, "script-src-elem", &self.script_src_elem);
        push(&mut parts, "script-src-attr", &self.script_src_attr);
        push(&mut parts, "style-src", &self.style_src);
        push(&mut parts, "style-src-elem", &self.style_src_elem);
        push(&mut parts, "style-src-attr", &self.style_src_attr);
        push(&mut parts, "img-src", &self.img_src);
        push(&mut parts, "connect-src", &self.connect_src);
        push(&mut parts, "font-src", &self.font_src);
        push(&mut parts, "object-src", &self.object_src);
        push(&mut parts, "media-src", &self.media_src);
        push(&mut parts, "frame-src", &self.frame_src);
        push(&mut parts, "worker-src", &self.worker_src);
        push(&mut parts, "child-src", &self.child_src);
        push(&mut parts, "manifest-src", &self.manifest_src);
        push(&mut parts, "prefetch-src", &self.prefetch_src);
        push(&mut parts, "form-action", &self.form_action);
        push(&mut parts, "frame-ancestors", &self.frame_ancestors);
        push(&mut parts, "base-uri", &self.base_uri);
        push(&mut parts, "navigate-to", &self.navigate_to);
        if !self.sandbox.is_empty() {
            parts.push(format!("sandbox {}", self.sandbox.join(" ")));
        }
        if !self.trusted_types.is_empty() {
            parts.push(format!("trusted-types {}", self.trusted_types.join(" ")));
        }
        if !self.require_trusted_types_for.is_empty() {
            parts.push(format!(
                "require-trusted-types-for {}",
                self.require_trusted_types_for
                    .iter()
                    .map(|t| format!("'{t}'"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if let Some(r) = &self.report_to {
            parts.push(format!("report-to {r}"));
        }
        if let Some(u) = &self.report_uri {
            parts.push(format!("report-uri {u}"));
        }
        if self.upgrade_insecure_requests {
            parts.push("upgrade-insecure-requests".into());
        }
        if self.block_all_mixed_content {
            parts.push("block-all-mixed-content".into());
        }
        parts.join("; ")
    }

    /// The canonical HTTP header name for this policy — flips with
    /// `mode`.
    #[must_use]
    pub fn header_name(&self) -> &'static str {
        match self.mode {
            Mode::Enforce => "Content-Security-Policy",
            Mode::ReportOnly => "Content-Security-Policy-Report-Only",
        }
    }

    /// Run the mutual-exclusion checks. Returns every warning; the
    /// caller chooses to fail or just log.
    #[must_use]
    pub fn validate(&self) -> Vec<CspError> {
        let mut errs = Vec::new();

        let check = |errs: &mut Vec<CspError>, name: &str, list: &[Source]| {
            if list.is_empty() {
                return;
            }
            let has_inline = list.contains(&Source::UnsafeInline);
            let has_nonce = list.iter().any(|s| matches!(s, Source::Nonce(_)));
            let has_hash = list.iter().any(|s| {
                matches!(
                    s,
                    Source::Sha256(_) | Source::Sha384(_) | Source::Sha512(_)
                )
            });
            let has_strict = list.contains(&Source::StrictDynamic);
            let has_wildcard = list.contains(&Source::Wildcard);
            let has_self = list.contains(&Source::Self_);

            if has_inline && has_nonce {
                errs.push(CspError::UnsafeInlineWithNonce {
                    directive: name.into(),
                });
            }
            if has_inline && has_strict {
                errs.push(CspError::UnsafeInlineWithStrictDynamic {
                    directive: name.into(),
                });
            }
            if has_strict && !has_nonce && !has_hash {
                errs.push(CspError::StrictDynamicWithoutBootstrap {
                    directive: name.into(),
                });
            }
            if has_wildcard && has_self {
                errs.push(CspError::WildcardAndSelf {
                    directive: name.into(),
                });
            }
            for s in list {
                if let Source::Origin(v) = s {
                    if v.chars().any(char::is_whitespace) {
                        errs.push(CspError::OriginWithWhitespace {
                            value: v.clone(),
                        });
                    }
                }
            }
        };

        check(&mut errs, "default-src", &self.default_src);
        check(&mut errs, "script-src", &self.script_src);
        check(&mut errs, "script-src-elem", &self.script_src_elem);
        check(&mut errs, "style-src", &self.style_src);
        check(&mut errs, "style-src-elem", &self.style_src_elem);
        check(&mut errs, "img-src", &self.img_src);
        check(&mut errs, "connect-src", &self.connect_src);
        check(&mut errs, "font-src", &self.font_src);
        check(&mut errs, "object-src", &self.object_src);
        check(&mut errs, "media-src", &self.media_src);
        check(&mut errs, "frame-src", &self.frame_src);
        check(&mut errs, "worker-src", &self.worker_src);
        check(&mut errs, "manifest-src", &self.manifest_src);
        check(&mut errs, "form-action", &self.form_action);
        check(&mut errs, "frame-ancestors", &self.frame_ancestors);
        check(&mut errs, "base-uri", &self.base_uri);
        check(&mut errs, "navigate-to", &self.navigate_to);

        for t in &self.sandbox {
            if !SANDBOX_TOKENS.contains(&t.as_str()) {
                errs.push(CspError::InvalidSandboxToken { token: t.clone() });
            }
        }

        if self.report_uri.is_some() && self.report_to.is_none() {
            errs.push(CspError::ReportUriDeprecated);
        }

        errs
    }
}

fn push(parts: &mut Vec<String>, name: &str, list: &[Source]) {
    if list.is_empty() {
        return;
    }
    let joined: Vec<String> = list.iter().map(render_source).collect();
    parts.push(format!("{name} {}", joined.join(" ")));
}

fn render_source(s: &Source) -> String {
    match s {
        Source::None => "'none'".into(),
        Source::Self_ => "'self'".into(),
        Source::UnsafeInline => "'unsafe-inline'".into(),
        Source::UnsafeEval => "'unsafe-eval'".into(),
        Source::StrictDynamic => "'strict-dynamic'".into(),
        Source::UnsafeHashes => "'unsafe-hashes'".into(),
        Source::WasmUnsafeEval => "'wasm-unsafe-eval'".into(),
        Source::Origin(v) => v.clone(),
        Source::Scheme(v) => format!("{v}:"),
        Source::Nonce(v) => format!("'nonce-{v}'"),
        Source::Sha256(v) => format!("'sha256-{v}'"),
        Source::Sha384(v) => format!("'sha384-{v}'"),
        Source::Sha512(v) => format!("'sha512-{v}'"),
        Source::Wildcard => "*".into(),
        Source::ReportTo(v) => v.clone(), // only meaningful in report-to, passed through as-is
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct CspPolicyRegistry {
    specs: Vec<CspPolicySpec>,
}

impl CspPolicyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: CspPolicySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = CspPolicySpec>) {
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
    pub fn specs(&self) -> &[CspPolicySpec] {
        &self.specs
    }

    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&CspPolicySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| s.enabled && !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.enabled && s.matches_host(host)))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<CspPolicySpec>, String> {
    tatara_lisp::compile_typed::<CspPolicySpec>(src)
        .map_err(|e| format!("failed to compile defcsp-policy forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<CspPolicySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_strict_self_only() {
        let s = CspPolicySpec::default_profile();
        assert_eq!(s.default_src, vec![Source::Self_]);
        assert_eq!(s.object_src, vec![Source::None]);
        assert_eq!(s.frame_ancestors, vec![Source::None]);
        assert!(s.upgrade_insecure_requests);
        assert!(s.block_all_mixed_content);
    }

    #[test]
    fn render_default_matches_canonical_form() {
        let s = CspPolicySpec::default_profile();
        let h = s.render();
        assert!(h.contains("default-src 'self'"));
        assert!(h.contains("object-src 'none'"));
        assert!(h.contains("frame-ancestors 'none'"));
        assert!(h.contains("upgrade-insecure-requests"));
        assert!(h.contains("block-all-mixed-content"));
    }

    #[test]
    fn render_nonce_sha256_origin_scheme() {
        let s = CspPolicySpec {
            name: "t".into(),
            script_src: vec![
                Source::Self_,
                Source::Nonce("abc123".into()),
                Source::Sha256("xyz==".into()),
                Source::Origin("https://api.example.com".into()),
                Source::Scheme("https".into()),
            ],
            ..CspPolicySpec::default_profile()
        };
        let h = s.render();
        assert!(h.contains("script-src 'self' 'nonce-abc123' 'sha256-xyz==' https://api.example.com https:"));
    }

    #[test]
    fn render_wildcard() {
        let s = CspPolicySpec {
            img_src: vec![Source::Wildcard],
            ..CspPolicySpec::default_profile()
        };
        assert!(s.render().contains("img-src *"));
    }

    #[test]
    fn render_sandbox_tokens_preserved() {
        let s = CspPolicySpec {
            sandbox: vec!["allow-forms".into(), "allow-scripts".into()],
            ..CspPolicySpec::default_profile()
        };
        assert!(s.render().contains("sandbox allow-forms allow-scripts"));
    }

    #[test]
    fn render_trusted_types_and_require() {
        let s = CspPolicySpec {
            trusted_types: vec!["lit-html".into(), "default".into()],
            require_trusted_types_for: vec!["script".into()],
            ..CspPolicySpec::default_profile()
        };
        let h = s.render();
        assert!(h.contains("trusted-types lit-html default"));
        assert!(h.contains("require-trusted-types-for 'script'"));
    }

    #[test]
    fn header_name_flips_with_mode() {
        let mut s = CspPolicySpec::default_profile();
        assert_eq!(s.header_name(), "Content-Security-Policy");
        s.mode = Mode::ReportOnly;
        assert_eq!(s.header_name(), "Content-Security-Policy-Report-Only");
    }

    #[test]
    fn validate_unsafe_inline_plus_nonce_is_flagged() {
        let s = CspPolicySpec {
            script_src: vec![Source::UnsafeInline, Source::Nonce("abc".into())],
            ..CspPolicySpec::default_profile()
        };
        let errs = s.validate();
        assert!(errs.iter().any(|e| matches!(
            e,
            CspError::UnsafeInlineWithNonce { directive } if directive == "script-src"
        )));
    }

    #[test]
    fn validate_unsafe_inline_plus_strict_dynamic_is_flagged() {
        let s = CspPolicySpec {
            script_src: vec![
                Source::UnsafeInline,
                Source::StrictDynamic,
                Source::Nonce("abc".into()),
            ],
            ..CspPolicySpec::default_profile()
        };
        let errs = s.validate();
        // Both the inline+strict-dynamic and inline+nonce checks fire.
        assert!(errs.iter().any(|e| matches!(
            e,
            CspError::UnsafeInlineWithStrictDynamic { .. }
        )));
    }

    #[test]
    fn validate_strict_dynamic_without_bootstrap_is_flagged() {
        let s = CspPolicySpec {
            script_src: vec![Source::Self_, Source::StrictDynamic],
            ..CspPolicySpec::default_profile()
        };
        let errs = s.validate();
        assert!(errs.iter().any(|e| matches!(
            e,
            CspError::StrictDynamicWithoutBootstrap { .. }
        )));
    }

    #[test]
    fn validate_strict_dynamic_with_nonce_is_ok() {
        let s = CspPolicySpec {
            script_src: vec![
                Source::StrictDynamic,
                Source::Nonce("abc".into()),
            ],
            ..CspPolicySpec::default_profile()
        };
        let errs = s.validate();
        assert!(
            !errs
                .iter()
                .any(|e| matches!(e, CspError::StrictDynamicWithoutBootstrap { .. })),
            "{errs:?}"
        );
    }

    #[test]
    fn validate_wildcard_plus_self_is_flagged() {
        let s = CspPolicySpec {
            img_src: vec![Source::Wildcard, Source::Self_],
            ..CspPolicySpec::default_profile()
        };
        let errs = s.validate();
        assert!(errs.iter().any(|e| matches!(
            e,
            CspError::WildcardAndSelf { directive } if directive == "img-src"
        )));
    }

    #[test]
    fn validate_origin_with_whitespace_is_flagged() {
        let s = CspPolicySpec {
            connect_src: vec![Source::Origin("https://ex.com /api".into())],
            ..CspPolicySpec::default_profile()
        };
        assert!(s
            .validate()
            .iter()
            .any(|e| matches!(e, CspError::OriginWithWhitespace { .. })));
    }

    #[test]
    fn validate_unknown_sandbox_token_is_flagged() {
        let s = CspPolicySpec {
            sandbox: vec!["allow-everything".into()],
            ..CspPolicySpec::default_profile()
        };
        assert!(s
            .validate()
            .iter()
            .any(|e| matches!(e, CspError::InvalidSandboxToken { .. })));
    }

    #[test]
    fn validate_sandbox_spec_valid_tokens_pass() {
        let s = CspPolicySpec {
            sandbox: vec![
                "allow-forms".into(),
                "allow-scripts".into(),
                "allow-same-origin".into(),
            ],
            ..CspPolicySpec::default_profile()
        };
        assert!(!s
            .validate()
            .iter()
            .any(|e| matches!(e, CspError::InvalidSandboxToken { .. })));
    }

    #[test]
    fn validate_report_uri_alone_flags_deprecation() {
        let s = CspPolicySpec {
            report_uri: Some("/csp-report".into()),
            report_to: None,
            ..CspPolicySpec::default_profile()
        };
        assert!(s
            .validate()
            .iter()
            .any(|e| matches!(e, CspError::ReportUriDeprecated)));
    }

    #[test]
    fn validate_report_uri_plus_report_to_no_warning() {
        let s = CspPolicySpec {
            report_uri: Some("/csp-report".into()),
            report_to: Some("csp-group".into()),
            ..CspPolicySpec::default_profile()
        };
        assert!(!s
            .validate()
            .iter()
            .any(|e| matches!(e, CspError::ReportUriDeprecated)));
    }

    #[test]
    fn validate_default_profile_clean() {
        let s = CspPolicySpec::default_profile();
        assert!(s.validate().is_empty(), "{:?}", s.validate());
    }

    #[test]
    fn render_omits_empty_directives() {
        let s = CspPolicySpec {
            default_src: vec![Source::Self_],
            script_src: vec![],
            style_src: vec![],
            img_src: vec![],
            connect_src: vec![],
            font_src: vec![],
            object_src: vec![],
            media_src: vec![],
            frame_src: vec![],
            worker_src: vec![],
            manifest_src: vec![],
            form_action: vec![],
            frame_ancestors: vec![],
            base_uri: vec![],
            upgrade_insecure_requests: false,
            block_all_mixed_content: false,
            ..CspPolicySpec::default_profile()
        };
        let h = s.render();
        assert!(h.contains("default-src 'self'"));
        assert!(!h.contains("script-src"));
        assert!(!h.contains("object-src"));
    }

    #[test]
    fn source_roundtrips_through_serde() {
        let cases = vec![
            Source::None,
            Source::Self_,
            Source::UnsafeInline,
            Source::UnsafeEval,
            Source::StrictDynamic,
            Source::UnsafeHashes,
            Source::WasmUnsafeEval,
            Source::Origin("https://api.ex.com".into()),
            Source::Scheme("data".into()),
            Source::Nonce("abc".into()),
            Source::Sha256("hash==".into()),
            Source::Sha384("hash==".into()),
            Source::Sha512("hash==".into()),
            Source::Wildcard,
            Source::ReportTo("grp".into()),
        ];
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: Source = serde_json::from_str(&json).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn mode_roundtrips_through_serde() {
        for m in [Mode::Enforce, Mode::ReportOnly] {
            let s = CspPolicySpec {
                mode: m,
                ..CspPolicySpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: CspPolicySpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.mode, m);
        }
    }

    #[test]
    fn registry_prefers_specific_host() {
        let mut reg = CspPolicyRegistry::new();
        reg.insert(CspPolicySpec::default_profile());
        reg.insert(CspPolicySpec {
            name: "admin".into(),
            host: "*://admin.example.com/*".into(),
            default_src: vec![Source::Wildcard],
            ..CspPolicySpec::default_profile()
        });
        let a = reg.resolve("admin.example.com").unwrap();
        assert_eq!(a.name, "admin");
        let other = reg.resolve("www.example.org").unwrap();
        assert_eq!(other.name, "strict");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_csp_policy_form() {
        let src = r#"
            (defcsp-policy :name "strict"
                           :host "*"
                           :mode "enforce"
                           :upgrade-insecure-requests #t
                           :block-all-mixed-content #t
                           :report-uri "/csp-report")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.mode, Mode::Enforce);
        assert!(s.upgrade_insecure_requests);
        assert_eq!(s.report_uri.as_deref(), Some("/csp-report"));
    }
}
