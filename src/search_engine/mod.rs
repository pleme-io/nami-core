//! `(defsearch-engine)` — declarative search engine.
//!
//! Absorbs Chrome custom search engines, Firefox keyword searches,
//! Safari search providers, Arc/Brave/Vivaldi custom engines. One
//! Lisp form declares a URL template (`%s` = query substitution), a
//! keyword shortcut, optional suggest endpoint, and encoding.
//!
//! ```lisp
//! (defsearch-engine :name       "kagi"
//!                   :keyword    "k"
//!                   :url        "https://kagi.com/search?q=%s"
//!                   :suggest    "https://kagi.com/api/autosuggest?q=%s"
//!                   :encoding   :percent-plus
//!                   :method     :get
//!                   :default    #t
//!                   :category   :web
//!                   :favicon    "https://kagi.com/favicon.ico")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Encoding for the query substitution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum QueryEncoding {
    /// `application/x-www-form-urlencoded` — spaces → `+`, most common.
    #[default]
    PercentPlus,
    /// RFC 3986 — spaces → `%20`.
    PercentStrict,
    /// Pass the raw query through untouched.
    Raw,
}

/// HTTP method.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SearchMethod {
    #[default]
    Get,
    Post,
}

/// Category hint — lets the omnibox group engines.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SearchCategory {
    #[default]
    Web,
    Images,
    Videos,
    News,
    Shopping,
    Maps,
    Code,
    Social,
    Academic,
    Ai,
    Developer,
    Reference,
    Other,
}

/// Search-engine profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsearch-engine"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchEngineSpec {
    pub name: String,
    /// Omnibox keyword shortcut — e.g. `"k"` means typing `k` + space
    /// + query uses this engine. Empty = no shortcut.
    #[serde(default)]
    pub keyword: String,
    /// URL template with `%s` or `{query}` for the encoded query.
    pub url: String,
    /// Optional autocomplete suggestion endpoint (same substitution).
    #[serde(default)]
    pub suggest: Option<String>,
    #[serde(default)]
    pub encoding: QueryEncoding,
    #[serde(default)]
    pub method: SearchMethod,
    /// POST body template (ignored for GET).
    #[serde(default)]
    pub post_body: Option<String>,
    /// Is this the default engine when no keyword matches?
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub category: SearchCategory,
    /// Favicon URL for omnibox rendering.
    #[serde(default)]
    pub favicon: Option<String>,
    /// Include POST login cookies when searching (vs ephemeral).
    #[serde(default = "default_auth_cookies")]
    pub auth_cookies: bool,
    /// Priority — higher wins when two engines share a keyword.
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_auth_cookies() -> bool {
    true
}
fn default_enabled() -> bool {
    true
}

impl SearchEngineSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "ddg".into(),
            keyword: "d".into(),
            url: "https://duckduckgo.com/?q=%s".into(),
            suggest: Some("https://duckduckgo.com/ac/?q=%s&type=list".into()),
            encoding: QueryEncoding::PercentPlus,
            method: SearchMethod::Get,
            post_body: None,
            default: true,
            category: SearchCategory::Web,
            favicon: Some("https://duckduckgo.com/favicon.ico".into()),
            auth_cookies: true,
            priority: 0,
            enabled: true,
            description: Some(
                "Default engine — DuckDuckGo with autocomplete suggestions.".into(),
            ),
        }
    }

    /// Substitute `query` into the URL template.
    #[must_use]
    pub fn render_url(&self, query: &str) -> String {
        render_template(&self.url, query, self.encoding)
    }

    /// Substitute `query` into the suggest template (if any).
    #[must_use]
    pub fn render_suggest(&self, query: &str) -> Option<String> {
        self.suggest
            .as_deref()
            .map(|t| render_template(t, query, self.encoding))
    }

    /// Substitute `query` into the POST body template (if any).
    #[must_use]
    pub fn render_body(&self, query: &str) -> Option<String> {
        self.post_body
            .as_deref()
            .map(|t| render_template(t, query, self.encoding))
    }
}

fn render_template(template: &str, query: &str, encoding: QueryEncoding) -> String {
    let encoded = match encoding {
        QueryEncoding::PercentPlus => percent_encode(query, true),
        QueryEncoding::PercentStrict => percent_encode(query, false),
        QueryEncoding::Raw => query.to_owned(),
    };
    template.replace("%s", &encoded).replace("{query}", &encoded)
}

fn percent_encode(s: &str, plus_space: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' if plus_space => out.push('+'),
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct SearchEngineRegistry {
    specs: Vec<SearchEngineSpec>,
}

impl SearchEngineRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SearchEngineSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SearchEngineSpec>) {
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
    pub fn specs(&self) -> &[SearchEngineSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SearchEngineSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// Look up by keyword shortcut. Ties broken by higher priority.
    #[must_use]
    pub fn by_keyword(&self, keyword: &str) -> Option<&SearchEngineSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.keyword == keyword)
            .max_by_key(|s| s.priority)
    }

    /// Get the default engine. Prefers enabled default=true; ties
    /// broken by highest priority.
    #[must_use]
    pub fn default_engine(&self) -> Option<&SearchEngineSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.default)
            .max_by_key(|s| s.priority)
            .or_else(|| self.specs.iter().find(|s| s.enabled))
    }

    /// All enabled engines in a category.
    #[must_use]
    pub fn by_category(&self, category: SearchCategory) -> Vec<&SearchEngineSpec> {
        self.specs
            .iter()
            .filter(|s| s.enabled && s.category == category)
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SearchEngineSpec>, String> {
    tatara_lisp::compile_typed::<SearchEngineSpec>(src)
        .map_err(|e| format!("failed to compile defsearch-engine forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SearchEngineSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_ddg_with_keyword_d() {
        let s = SearchEngineSpec::default_profile();
        assert_eq!(s.keyword, "d");
        assert!(s.default);
        assert!(s.suggest.is_some());
    }

    #[test]
    fn render_url_percent_plus_encodes_space() {
        let s = SearchEngineSpec::default_profile();
        assert_eq!(
            s.render_url("rust async"),
            "https://duckduckgo.com/?q=rust+async"
        );
    }

    #[test]
    fn render_url_percent_strict_encodes_space() {
        let s = SearchEngineSpec {
            encoding: QueryEncoding::PercentStrict,
            ..SearchEngineSpec::default_profile()
        };
        assert_eq!(
            s.render_url("rust async"),
            "https://duckduckgo.com/?q=rust%20async"
        );
    }

    #[test]
    fn render_url_raw_passes_through() {
        let s = SearchEngineSpec {
            url: "https://ex.com/?q=%s".into(),
            encoding: QueryEncoding::Raw,
            ..SearchEngineSpec::default_profile()
        };
        assert_eq!(s.render_url("foo bar"), "https://ex.com/?q=foo bar");
    }

    #[test]
    fn render_url_encodes_unicode_and_special_chars() {
        let s = SearchEngineSpec {
            url: "https://ex.com/?q=%s".into(),
            encoding: QueryEncoding::PercentStrict,
            ..SearchEngineSpec::default_profile()
        };
        // Japanese "hello" (こんにちは) + a plus sign
        let rendered = s.render_url("こんにちは+");
        assert!(rendered.contains("%E3%81%93"));
        assert!(rendered.contains("%2B"));
    }

    #[test]
    fn render_url_supports_brace_query_token() {
        let s = SearchEngineSpec {
            url: "https://ex.com/?q={query}".into(),
            ..SearchEngineSpec::default_profile()
        };
        assert_eq!(s.render_url("test"), "https://ex.com/?q=test");
    }

    #[test]
    fn render_suggest_returns_none_when_absent() {
        let s = SearchEngineSpec {
            suggest: None,
            ..SearchEngineSpec::default_profile()
        };
        assert!(s.render_suggest("foo").is_none());
    }

    #[test]
    fn render_body_returns_none_when_absent() {
        let s = SearchEngineSpec {
            post_body: None,
            ..SearchEngineSpec::default_profile()
        };
        assert!(s.render_body("foo").is_none());
    }

    #[test]
    fn render_body_substitutes_when_present() {
        let s = SearchEngineSpec {
            method: SearchMethod::Post,
            post_body: Some("query=%s".into()),
            ..SearchEngineSpec::default_profile()
        };
        assert_eq!(s.render_body("rust async").unwrap(), "query=rust+async");
    }

    #[test]
    fn by_keyword_uses_priority_tiebreak() {
        let mut reg = SearchEngineRegistry::new();
        reg.insert(SearchEngineSpec {
            name: "ddg".into(),
            keyword: "d".into(),
            priority: 0,
            ..SearchEngineSpec::default_profile()
        });
        reg.insert(SearchEngineSpec {
            name: "custom-ddg".into(),
            keyword: "d".into(),
            priority: 10,
            ..SearchEngineSpec::default_profile()
        });
        assert_eq!(reg.by_keyword("d").unwrap().name, "custom-ddg");
    }

    #[test]
    fn default_engine_picks_highest_priority_default() {
        let mut reg = SearchEngineRegistry::new();
        reg.insert(SearchEngineSpec {
            name: "a".into(),
            default: true,
            priority: 5,
            ..SearchEngineSpec::default_profile()
        });
        reg.insert(SearchEngineSpec {
            name: "b".into(),
            default: true,
            priority: 20,
            ..SearchEngineSpec::default_profile()
        });
        assert_eq!(reg.default_engine().unwrap().name, "b");
    }

    #[test]
    fn default_engine_falls_back_to_first_enabled_when_no_default() {
        let mut reg = SearchEngineRegistry::new();
        reg.insert(SearchEngineSpec {
            name: "x".into(),
            default: false,
            ..SearchEngineSpec::default_profile()
        });
        assert_eq!(reg.default_engine().unwrap().name, "x");
    }

    #[test]
    fn by_category_filters_correctly() {
        let mut reg = SearchEngineRegistry::new();
        reg.insert(SearchEngineSpec::default_profile());
        reg.insert(SearchEngineSpec {
            name: "gh".into(),
            category: SearchCategory::Code,
            ..SearchEngineSpec::default_profile()
        });
        assert_eq!(reg.by_category(SearchCategory::Code).len(), 1);
        assert_eq!(reg.by_category(SearchCategory::Code)[0].name, "gh");
    }

    #[test]
    fn disabled_engine_ignored_by_keyword_and_default() {
        let mut reg = SearchEngineRegistry::new();
        reg.insert(SearchEngineSpec {
            name: "off".into(),
            keyword: "o".into(),
            default: true,
            enabled: false,
            ..SearchEngineSpec::default_profile()
        });
        assert!(reg.by_keyword("o").is_none());
        assert!(reg.default_engine().is_none());
    }

    #[test]
    fn encoding_roundtrips_through_serde() {
        for e in [
            QueryEncoding::PercentPlus,
            QueryEncoding::PercentStrict,
            QueryEncoding::Raw,
        ] {
            let s = SearchEngineSpec {
                encoding: e,
                ..SearchEngineSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SearchEngineSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.encoding, e);
        }
    }

    #[test]
    fn method_roundtrips_through_serde() {
        for m in [SearchMethod::Get, SearchMethod::Post] {
            let s = SearchEngineSpec {
                method: m,
                ..SearchEngineSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SearchEngineSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.method, m);
        }
    }

    #[test]
    fn category_roundtrips_through_serde() {
        for c in [
            SearchCategory::Web,
            SearchCategory::Images,
            SearchCategory::Videos,
            SearchCategory::News,
            SearchCategory::Shopping,
            SearchCategory::Maps,
            SearchCategory::Code,
            SearchCategory::Social,
            SearchCategory::Academic,
            SearchCategory::Ai,
            SearchCategory::Developer,
            SearchCategory::Reference,
            SearchCategory::Other,
        ] {
            let s = SearchEngineSpec {
                category: c,
                ..SearchEngineSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: SearchEngineSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.category, c);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_search_engine_form() {
        let src = r#"
            (defsearch-engine :name "kagi"
                              :keyword "k"
                              :url "https://kagi.com/search?q=%s"
                              :suggest "https://kagi.com/api/autosuggest?q=%s"
                              :encoding "percent-plus"
                              :method "get"
                              :default #t
                              :category "web"
                              :priority 10)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "kagi");
        assert_eq!(s.keyword, "k");
        assert!(s.default);
        assert_eq!(s.priority, 10);
    }
}
