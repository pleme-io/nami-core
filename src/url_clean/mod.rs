//! `(defurl-clean)` — strip tracking parameters from URLs.
//!
//! Absorbs ClearURLs, Neat URL, and PureURL. Each rule scopes to a
//! host glob + declares exact parameter names and regex patterns to
//! remove from the query string. Composes with [`crate::redirect`]:
//! ClearURLs-style cleaning then Invidious-style redirect = Tor-
//! Browser-ish URL hygiene without the bundle.
//!
//! ```lisp
//! (defurl-clean :name            "utm-global"
//!               :host            "*"
//!               :strip           ("utm_source" "utm_medium" "utm_campaign"
//!                                 "utm_term"   "utm_content"
//!                                 "gclid"      "fbclid"      "mc_cid"
//!                                 "mc_eid"     "yclid"       "msclkid"))
//!
//! (defurl-clean :name            "amazon-tracking"
//!               :host            "*://*.amazon.*/*"
//!               :strip           ("tag" "ref" "ref_" "pf_rd_*"))
//! ```
//!
//! Patterns ending in `*` are prefix-wildcard matches (so
//! `pf_rd_*` removes `pf_rd_r`, `pf_rd_p`, etc.). The rest are
//! exact name matches. No regex — keeps the substrate dep-free.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// URL-cleaning rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defurl-clean"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UrlCleanSpec {
    pub name: String,
    /// Host glob; `"*"` = everywhere.
    #[serde(default = "crate::extension::default_star_host")]
    pub host: String,
    /// Parameter-name matchers. Entry ending in `*` is a prefix
    /// match; anything else is exact (case-sensitive — matches HTTP
    /// URL semantics).
    #[serde(default)]
    pub strip: Vec<String>,
    /// Runtime toggle.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl UrlCleanSpec {
    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    /// Returns true if `param_name` matches any strip entry.
    #[must_use]
    pub fn strips_param(&self, param_name: &str) -> bool {
        self.strip.iter().any(|pat| param_matches(pat, param_name))
    }
}

fn param_matches(pattern: &str, name: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        pattern == name
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct UrlCleanRegistry {
    specs: Vec<UrlCleanSpec>,
}

impl UrlCleanRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: UrlCleanSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = UrlCleanSpec>) {
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
    pub fn specs(&self) -> &[UrlCleanSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&UrlCleanSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Every enabled rule whose host matches — multiple can apply
    /// (e.g., a wildcard `utm-global` + a host-specific `amazon-tracking`).
    #[must_use]
    pub fn applicable(&self, host: &str) -> Vec<&UrlCleanSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.matches_host(host))
            .collect()
    }

    /// Apply every applicable rule to `input_url`, removing any
    /// matching query param. Returns the cleaned URL; passes the
    /// input through unchanged when nothing applies.
    #[must_use]
    pub fn apply(&self, input_url: &str) -> String {
        let Ok(parsed) = url::Url::parse(input_url) else {
            return input_url.to_owned();
        };
        let host = parsed.host_str().unwrap_or("");
        let applicable = self.applicable(host);
        if applicable.is_empty() {
            return input_url.to_owned();
        }

        // Collect surviving (k, v) pairs.
        let query_pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .filter(|(k, _)| {
                !applicable.iter().any(|rule| rule.strips_param(k.as_ref()))
            })
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        let mut cleaned = parsed.clone();
        if query_pairs.is_empty() {
            cleaned.set_query(None);
        } else {
            cleaned.query_pairs_mut().clear().extend_pairs(&query_pairs);
        }
        cleaned.to_string()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<UrlCleanSpec>, String> {
    tatara_lisp::compile_typed::<UrlCleanSpec>(src)
        .map_err(|e| format!("failed to compile defurl-clean forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<UrlCleanSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, host: &str, params: &[&str]) -> UrlCleanSpec {
        UrlCleanSpec {
            name: name.into(),
            host: host.into(),
            strip: params.iter().map(|s| (*s).into()).collect(),
            enabled: true,
            description: None,
        }
    }

    #[test]
    fn strips_exact_param_name() {
        let s = sample("utm", "*", &["utm_source"]);
        assert!(s.strips_param("utm_source"));
        assert!(!s.strips_param("utm_medium"));
    }

    #[test]
    fn strips_prefix_wildcard() {
        let s = sample("pf", "*", &["pf_rd_*"]);
        assert!(s.strips_param("pf_rd_r"));
        assert!(s.strips_param("pf_rd_p"));
        assert!(!s.strips_param("pf_something"));
    }

    #[test]
    fn param_name_match_is_case_sensitive() {
        let s = sample("x", "*", &["UTM_Source"]);
        assert!(s.strips_param("UTM_Source"));
        assert!(!s.strips_param("utm_source"));
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = UrlCleanRegistry::new();
        reg.insert(sample("a", "*", &["x"]));
        reg.insert(sample("a", "*", &["y"]));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].strip, vec!["y"]);
    }

    #[test]
    fn apply_strips_matching_params() {
        let mut reg = UrlCleanRegistry::new();
        reg.insert(sample("utm", "*", &["utm_source", "utm_medium"]));
        let cleaned = reg.apply(
            "https://example.com/article?id=1&utm_source=twitter&utm_medium=social",
        );
        assert!(cleaned.contains("id=1"));
        assert!(!cleaned.contains("utm_source"));
        assert!(!cleaned.contains("utm_medium"));
    }

    #[test]
    fn apply_drops_query_string_when_all_params_stripped() {
        let mut reg = UrlCleanRegistry::new();
        reg.insert(sample("utm", "*", &["utm_source"]));
        let cleaned = reg.apply("https://example.com/article?utm_source=x");
        assert!(!cleaned.contains('?'));
    }

    #[test]
    fn apply_is_noop_when_nothing_matches() {
        let reg = UrlCleanRegistry::new();
        let input = "https://example.com/x?a=1&b=2";
        assert_eq!(reg.apply(input), input);
    }

    #[test]
    fn apply_passes_through_invalid_urls() {
        let reg = UrlCleanRegistry::new();
        assert_eq!(reg.apply("not-a-url"), "not-a-url");
    }

    #[test]
    fn apply_honors_host_scope() {
        let mut reg = UrlCleanRegistry::new();
        reg.insert(sample(
            "amz",
            "*://*.amazon.com/*",
            &["tag", "ref", "ref_"],
        ));
        let cleaned = reg.apply("https://www.amazon.com/x?tag=a&ref=b&keep=c");
        assert!(!cleaned.contains("tag=a"));
        assert!(!cleaned.contains("ref=b"));
        assert!(cleaned.contains("keep=c"));

        // Same params on a non-matching host pass through.
        let untouched = reg.apply("https://example.com/x?tag=a&ref=b");
        assert!(untouched.contains("tag=a"));
    }

    #[test]
    fn applicable_excludes_disabled() {
        let mut reg = UrlCleanRegistry::new();
        let s = UrlCleanSpec {
            enabled: false,
            ..sample("x", "*", &["utm_source"])
        };
        reg.insert(s);
        assert!(reg.applicable("example.com").is_empty());
    }

    #[test]
    fn applicable_stacks_multiple_rules() {
        let mut reg = UrlCleanRegistry::new();
        reg.insert(sample("a", "*", &["utm_source"]));
        reg.insert(sample("b", "*://*.amazon.com/*", &["tag"]));
        assert_eq!(reg.applicable("www.amazon.com").len(), 2);
        assert_eq!(reg.applicable("example.com").len(), 1);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_url_clean_form() {
        let src = r#"
            (defurl-clean :name  "utm"
                          :host  "*"
                          :strip ("utm_source" "utm_medium" "pf_rd_*"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.strip.len(), 3);
        assert!(s.strips_param("pf_rd_r"));
    }
}
