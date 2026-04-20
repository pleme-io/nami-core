//! `(defsecurity-policy)` — declarative CSP + Permissions-Policy.
//!
//! Absorbs per-origin Content-Security-Policy, Permissions-Policy,
//! Referrer-Policy, and Cross-Origin-* headers into the substrate.
//! Each form declares one policy scoped by host glob; the registry
//! resolves the most-specific match at request time.
//!
//! ```lisp
//! (defsecurity-policy
//!   :name  "default"
//!   :host  "*"
//!   :csp   "default-src 'self'; script-src 'self' https://cdn.example.com"
//!   :permissions-policy "camera=(), microphone=(self)"
//!   :referrer-policy    "strict-origin-when-cross-origin"
//!   :frame-ancestors    "'self'"
//!   :report-uri         "https://example.com/csp-reports")
//!
//! (defsecurity-policy
//!   :name     "github"
//!   :host     "*://*.github.com/*"
//!   :csp      "default-src 'self' *.githubusercontent.com"
//!   :upgrade-insecure-requests #t)
//! ```
//!
//! The policy is data — namimado's fetch pipeline emits the matched
//! policy as HTTP headers on outbound requests (for pages it hosts)
//! and enforces it as a pre-fetch gate for subresources (checked
//! against the navigating host's resolved policy). Same rules apply
//! to downloaded content — the sekiban/tameshi attestation chain
//! treats a policy-violation as an Invalid verdict.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One security-policy rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsecurity-policy"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityPolicySpec {
    pub name: String,
    /// Host glob pattern — `"*"` matches any, `"*://*.example.com/*"`
    /// matches subdomains. WebExtensions-compatible syntax, reuses
    /// the matcher from the extension module.
    #[serde(default = "default_host")]
    pub host: String,
    /// Raw CSP header value. Emitted as `Content-Security-Policy:`.
    /// None → no CSP header emitted.
    #[serde(default)]
    pub csp: Option<String>,
    /// CSP in report-only mode — emitted alongside or instead of the
    /// enforcing CSP. Header: `Content-Security-Policy-Report-Only:`.
    #[serde(default)]
    pub csp_report_only: Option<String>,
    /// Raw Permissions-Policy header (formerly Feature-Policy).
    /// Example: `"camera=(), microphone=(self), geolocation=(self 'https://maps.example.com')"`
    #[serde(default)]
    pub permissions_policy: Option<String>,
    /// Referrer-Policy header. `"no-referrer"`, `"same-origin"`,
    /// `"strict-origin-when-cross-origin"`, etc.
    #[serde(default)]
    pub referrer_policy: Option<String>,
    /// Convenience for frame-ancestors CSP directive. If set, merged
    /// into the CSP output.
    #[serde(default)]
    pub frame_ancestors: Option<String>,
    /// Cross-Origin-Opener-Policy — `"same-origin"`, `"unsafe-none"`, etc.
    #[serde(default)]
    pub cross_origin_opener_policy: Option<String>,
    /// Cross-Origin-Embedder-Policy — `"require-corp"`, etc.
    #[serde(default)]
    pub cross_origin_embedder_policy: Option<String>,
    /// X-Frame-Options — `"DENY"`, `"SAMEORIGIN"`. For legacy
    /// consumers that don't parse frame-ancestors.
    #[serde(default)]
    pub x_frame_options: Option<String>,
    /// When true, upgrade all insecure-scheme subresource requests
    /// to https. Merged into CSP as `upgrade-insecure-requests;`.
    #[serde(default)]
    pub upgrade_insecure_requests: bool,
    /// CSP `report-uri` directive — where violations POST. Merged
    /// into both enforcing + report-only CSPs.
    #[serde(default)]
    pub report_uri: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}

/// Rendered HTTP headers ready for the fetch pipeline / outbound
/// response. Caller emits each `(name, value)` pair verbatim.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyHeaders {
    pub headers: Vec<(String, String)>,
}

impl SecurityPolicySpec {
    /// Does this rule apply to `host`? Empty/`"*"` matches all.
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// Render as a list of HTTP headers. Directives are merged
    /// where the spec has them; missing fields produce no header.
    #[must_use]
    pub fn render_headers(&self) -> PolicyHeaders {
        let mut out: Vec<(String, String)> = Vec::new();

        if let Some(mut csp) = self.csp.clone() {
            csp = augment_csp(csp, self);
            out.push(("Content-Security-Policy".into(), csp));
        } else if self.upgrade_insecure_requests || self.frame_ancestors.is_some()
            || self.report_uri.is_some()
        {
            // User only supplied convenience fields, no :csp. Build one.
            let base = String::new();
            let csp = augment_csp(base, self);
            if !csp.trim().is_empty() {
                out.push(("Content-Security-Policy".into(), csp));
            }
        }

        if let Some(ro) = &self.csp_report_only {
            let mut ro = ro.clone();
            ro = augment_csp(ro, self);
            out.push(("Content-Security-Policy-Report-Only".into(), ro));
        }

        if let Some(p) = &self.permissions_policy {
            out.push(("Permissions-Policy".into(), p.clone()));
        }

        if let Some(r) = &self.referrer_policy {
            out.push(("Referrer-Policy".into(), r.clone()));
        }

        if let Some(c) = &self.cross_origin_opener_policy {
            out.push(("Cross-Origin-Opener-Policy".into(), c.clone()));
        }

        if let Some(c) = &self.cross_origin_embedder_policy {
            out.push(("Cross-Origin-Embedder-Policy".into(), c.clone()));
        }

        if let Some(x) = &self.x_frame_options {
            out.push(("X-Frame-Options".into(), x.clone()));
        }

        PolicyHeaders { headers: out }
    }
}

/// Merge convenience fields into a CSP string. Idempotent — re-
/// appending a directive already present is avoided by substring
/// check (V1 simplification; real CSPs have a proper directive
/// parser in V2).
fn augment_csp(mut csp: String, spec: &SecurityPolicySpec) -> String {
    if spec.upgrade_insecure_requests && !csp.contains("upgrade-insecure-requests") {
        append_directive(&mut csp, "upgrade-insecure-requests");
    }
    if let Some(fa) = &spec.frame_ancestors {
        let directive = format!("frame-ancestors {fa}");
        if !csp.contains("frame-ancestors") {
            append_directive(&mut csp, &directive);
        }
    }
    if let Some(uri) = &spec.report_uri {
        let directive = format!("report-uri {uri}");
        if !csp.contains("report-uri") {
            append_directive(&mut csp, &directive);
        }
    }
    csp
}

fn append_directive(csp: &mut String, directive: &str) {
    let trimmed = csp.trim_end();
    let trim_len = trimmed.len();
    csp.truncate(trim_len);
    if csp.is_empty() {
        csp.push_str(directive);
    } else {
        let needs_sep = !csp.ends_with(';');
        if needs_sep {
            csp.push(';');
        }
        csp.push(' ');
        csp.push_str(directive);
    }
}

/// Registry of policies, most-specific host match wins at resolve time.
#[derive(Debug, Clone, Default)]
pub struct SecurityPolicyRegistry {
    specs: Vec<SecurityPolicySpec>,
}

impl SecurityPolicyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SecurityPolicySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SecurityPolicySpec>) {
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
    pub fn specs(&self) -> &[SecurityPolicySpec] {
        &self.specs
    }

    /// Resolve the matching policy for `host`. Host-specific rules
    /// (anything NOT "*" or empty) outrank wildcards. Returns `None`
    /// if no rule matches.
    #[must_use]
    pub fn resolve(&self, host: &str) -> Option<&SecurityPolicySpec> {
        let specific = self
            .specs
            .iter()
            .find(|s| !s.host.is_empty() && s.host != "*" && s.matches_host(host));
        specific.or_else(|| self.specs.iter().find(|s| s.matches_host(host)))
    }

    /// Convenience: resolve + render. Empty `PolicyHeaders` if no
    /// rule matches — callers should not emit any header in that case.
    #[must_use]
    pub fn headers_for(&self, host: &str) -> PolicyHeaders {
        match self.resolve(host) {
            Some(spec) => spec.render_headers(),
            None => PolicyHeaders::default(),
        }
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SecurityPolicySpec>, String> {
    tatara_lisp::compile_typed::<SecurityPolicySpec>(src)
        .map_err(|e| format!("failed to compile defsecurity-policy forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SecurityPolicySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> SecurityPolicySpec {
        SecurityPolicySpec {
            name: "x".into(),
            host: "*".into(),
            csp: None,
            csp_report_only: None,
            permissions_policy: None,
            referrer_policy: None,
            frame_ancestors: None,
            cross_origin_opener_policy: None,
            cross_origin_embedder_policy: None,
            x_frame_options: None,
            upgrade_insecure_requests: false,
            report_uri: None,
            description: None,
        }
    }

    #[test]
    fn wildcard_host_matches_everything() {
        let s = empty();
        assert!(s.matches_host("anywhere.com"));
        assert!(s.matches_host(""));
    }

    #[test]
    fn specific_host_glob_matches_subdomains() {
        let s = SecurityPolicySpec {
            host: "*://*.example.com/*".into(),
            ..empty()
        };
        assert!(s.matches_host("blog.example.com"));
        assert!(!s.matches_host("evil.com"));
    }

    #[test]
    fn render_emits_csp_header_when_set() {
        let s = SecurityPolicySpec {
            csp: Some("default-src 'self'".into()),
            ..empty()
        };
        let h = s.render_headers();
        assert_eq!(h.headers.len(), 1);
        assert_eq!(h.headers[0].0, "Content-Security-Policy");
        assert_eq!(h.headers[0].1, "default-src 'self'");
    }

    #[test]
    fn render_merges_upgrade_insecure_requests_into_csp() {
        let s = SecurityPolicySpec {
            csp: Some("default-src 'self'".into()),
            upgrade_insecure_requests: true,
            ..empty()
        };
        let h = s.render_headers();
        assert!(h.headers[0]
            .1
            .contains("upgrade-insecure-requests"));
    }

    #[test]
    fn render_merges_frame_ancestors_and_report_uri() {
        let s = SecurityPolicySpec {
            csp: Some("default-src 'self'".into()),
            frame_ancestors: Some("'self' https://trusted.example.com".into()),
            report_uri: Some("https://example.com/csp-report".into()),
            ..empty()
        };
        let h = s.render_headers();
        let csp = &h.headers[0].1;
        assert!(csp.contains("frame-ancestors 'self' https://trusted.example.com"));
        assert!(csp.contains("report-uri https://example.com/csp-report"));
    }

    #[test]
    fn render_builds_csp_from_convenience_fields_when_no_base() {
        let s = SecurityPolicySpec {
            upgrade_insecure_requests: true,
            frame_ancestors: Some("'none'".into()),
            ..empty()
        };
        let h = s.render_headers();
        // Synthetic CSP from convenience fields only.
        assert_eq!(h.headers[0].0, "Content-Security-Policy");
        assert!(h.headers[0].1.contains("upgrade-insecure-requests"));
        assert!(h.headers[0].1.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn augment_is_idempotent() {
        // Pre-existing directive → not duplicated.
        let s = SecurityPolicySpec {
            csp: Some("default-src 'self'; upgrade-insecure-requests".into()),
            upgrade_insecure_requests: true,
            ..empty()
        };
        let h = s.render_headers();
        let count = h.headers[0].1.matches("upgrade-insecure-requests").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn render_all_supported_headers() {
        let s = SecurityPolicySpec {
            csp: Some("default-src 'self'".into()),
            csp_report_only: Some("default-src 'self' https://ok".into()),
            permissions_policy: Some("camera=(), microphone=(self)".into()),
            referrer_policy: Some("strict-origin-when-cross-origin".into()),
            cross_origin_opener_policy: Some("same-origin".into()),
            cross_origin_embedder_policy: Some("require-corp".into()),
            x_frame_options: Some("DENY".into()),
            ..empty()
        };
        let h = s.render_headers();
        let names: Vec<&str> = h.headers.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"Content-Security-Policy"));
        assert!(names.contains(&"Content-Security-Policy-Report-Only"));
        assert!(names.contains(&"Permissions-Policy"));
        assert!(names.contains(&"Referrer-Policy"));
        assert!(names.contains(&"Cross-Origin-Opener-Policy"));
        assert!(names.contains(&"Cross-Origin-Embedder-Policy"));
        assert!(names.contains(&"X-Frame-Options"));
    }

    #[test]
    fn registry_resolves_host_specific_over_wildcard() {
        let mut reg = SecurityPolicyRegistry::new();
        reg.insert(SecurityPolicySpec {
            name: "default".into(),
            host: "*".into(),
            csp: Some("default-src 'self'".into()),
            ..empty()
        });
        reg.insert(SecurityPolicySpec {
            name: "github".into(),
            host: "*://*.github.com/*".into(),
            csp: Some("default-src 'self' *.githubusercontent.com".into()),
            ..empty()
        });
        let on_github = reg.resolve("blog.github.com").unwrap();
        assert_eq!(on_github.name, "github");
        let on_other = reg.resolve("example.org").unwrap();
        assert_eq!(on_other.name, "default");
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = SecurityPolicyRegistry::new();
        reg.insert(SecurityPolicySpec {
            name: "x".into(),
            csp: Some("one".into()),
            ..empty()
        });
        reg.insert(SecurityPolicySpec {
            name: "x".into(),
            csp: Some("two".into()),
            ..empty()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].csp.as_deref(), Some("two"));
    }

    #[test]
    fn headers_for_empty_when_no_match() {
        let reg = SecurityPolicyRegistry::new();
        assert!(reg.headers_for("anywhere.com").headers.is_empty());
    }

    #[test]
    fn render_no_headers_when_spec_is_bare() {
        let s = empty();
        let h = s.render_headers();
        assert!(h.headers.is_empty());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_full_form() {
        let src = r#"
            (defsecurity-policy
              :name               "default"
              :host               "*"
              :csp                "default-src 'self'"
              :permissions-policy "camera=(), microphone=(self)"
              :referrer-policy    "strict-origin-when-cross-origin"
              :upgrade-insecure-requests #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "default");
        assert_eq!(s.csp.as_deref(), Some("default-src 'self'"));
        assert_eq!(
            s.permissions_policy.as_deref(),
            Some("camera=(), microphone=(self)")
        );
        assert!(s.upgrade_insecure_requests);
    }
}
