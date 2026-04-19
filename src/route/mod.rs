//! Lisp-authored URL pattern routing.
//!
//! A [`RouteSpec`] declares a path pattern (with `:param` placeholders),
//! a set of state-cell bindings for extracted params, and an ordered
//! list of **on-match action names** — any mix of transform / plan /
//! effect / query / agent names that already exist elsewhere in the
//! Lisp substrate. When a fetched URL matches a route, the runtime
//! binds the params into the state store and fires the actions.
//!
//! ```lisp
//! (defroute :pattern "/users/:id"
//!           :bind (("user-id" "id"))
//!           :on-match ("load-user-query" "auto-reader-mode"))
//!
//! (defroute :pattern "/posts/:year/:month/:slug"
//!           :bind (("post-year" "year") ("post-month" "month") ("post-slug" "slug"))
//!           :on-match ("load-post-query"))
//!
//! (defroute :pattern "/"
//!           :on-match ("load-homepage"))
//! ```
//!
//! Pattern grammar — intentionally minimal:
//!
//!   pattern  := SEGMENT ('/' SEGMENT)*
//!   SEGMENT  := literal-text      -- matches exact (case-sensitive)
//!             | ':' IDENT          -- param; captures the segment
//!             | '*'                -- wildcard; matches any single segment
//!
//! Matching is **path-only** — the host and query string don't
//! participate. So `"/users/:id"` matches `https://anywhere.com/users/42`
//! and extracts `id = "42"`. Query strings and fragments are
//! ignored.
//!
//! The runtime owns the "what to do with on-match names" decision —
//! this module just returns the match. nami wires it into navigate
//! so the named actions resolve through its existing transform /
//! plan / agent / query registries.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// A pair `(cell-name, param-name)` — binds a route parameter into a
/// named state cell. Defaults to identity when the second element is
/// omitted in Lisp (`(":id")`).
pub type BindPair = (String, String);

/// A declarative route.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defroute"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteSpec {
    pub name: Option<String>,
    pub pattern: String,
    /// `((cell-name param-name) …)`. Absent = no state binding.
    #[serde(default)]
    pub bind: Vec<BindPair>,
    /// Names of transforms, plans, effects, queries, agents to fire
    /// when this route matches. The runtime resolves each name
    /// against whatever registry owns it.
    #[serde(default)]
    pub on_match: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A route's compiled form — pattern parsed into ordered segments.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Param(String),
    Wildcard,
}

/// One route's parsed pattern.
#[derive(Debug, Clone)]
struct Compiled {
    segments: Vec<Segment>,
}

impl Compiled {
    fn parse(pattern: &str) -> Self {
        let trimmed = pattern.trim_start_matches('/');
        let segments: Vec<Segment> = trimmed
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s == "*" {
                    Segment::Wildcard
                } else if let Some(name) = s.strip_prefix(':') {
                    Segment::Param(name.to_owned())
                } else {
                    Segment::Literal(s.to_owned())
                }
            })
            .collect();
        Self { segments }
    }

    /// Try to match a URL path (already extracted from any URL scheme
    /// / host). Returns the param map on success.
    fn match_path(&self, path: &str) -> Option<BTreeMap<String, String>> {
        let parts: Vec<&str> = path
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() != self.segments.len() {
            return None;
        }
        let mut params = BTreeMap::new();
        for (seg, part) in self.segments.iter().zip(parts.iter()) {
            match seg {
                Segment::Literal(l) => {
                    if l != part {
                        return None;
                    }
                }
                Segment::Param(name) => {
                    params.insert(name.clone(), (*part).to_owned());
                }
                Segment::Wildcard => {
                    // Matches any single segment; not captured.
                }
            }
        }
        Some(params)
    }
}

/// Result of a successful route match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    /// The route's name (or a fallback string based on the pattern).
    pub route: String,
    /// Extracted params from `:param` segments.
    pub params: BTreeMap<String, String>,
    /// Action names the runtime should fire, preserving author order.
    pub on_match: Vec<String>,
    /// `(cell-name, param-name)` pairs the runtime should resolve +
    /// write into the state store.
    pub bindings: Vec<BindPair>,
}

/// Index of route specs. `match_url` returns the FIRST matching
/// route in insertion order — callers should register more-specific
/// routes before more-general ones.
#[derive(Debug, Clone, Default)]
pub struct RouteRegistry {
    entries: Vec<(RouteSpec, Compiled)>,
}

impl RouteRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: RouteSpec) {
        let compiled = Compiled::parse(&spec.pattern);
        self.entries.push((spec, compiled));
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = RouteSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Match a URL (any shape — we only look at the path). Returns
    /// the first route whose pattern matches, with extracted params.
    #[must_use]
    pub fn match_url(&self, url: &str) -> Option<RouteMatch> {
        let path = extract_path(url);
        for (spec, compiled) in &self.entries {
            if let Some(params) = compiled.match_path(path) {
                let route = spec.name.clone().unwrap_or_else(|| spec.pattern.clone());
                return Some(RouteMatch {
                    route,
                    params,
                    on_match: spec.on_match.clone(),
                    bindings: spec.bind.clone(),
                });
            }
        }
        None
    }
}

/// Extract the path portion of a URL (everything after the host,
/// excluding query and fragment). Works with plain paths too.
///
/// Non-URL inputs like `"users/42"` are treated as paths.
fn extract_path(url: &str) -> &str {
    let after_scheme = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    // Strip host (everything up to the first `/`) — only if we saw `://`.
    let path_start = if url.contains("://") {
        after_scheme
            .find('/')
            .map(|i| i)
            .unwrap_or(after_scheme.len())
    } else {
        0
    };
    let rest = &after_scheme[path_start..];
    // Strip query + fragment.
    let end = rest
        .find(|c: char| c == '?' || c == '#')
        .unwrap_or(rest.len());
    &rest[..end]
}

/// Compile a Lisp document of `(defroute …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<RouteSpec>, String> {
    tatara_lisp::compile_typed::<RouteSpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defroute` keyword.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<RouteSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(pattern: &str, on_match: &[&str]) -> RouteSpec {
        RouteSpec {
            name: None,
            pattern: pattern.into(),
            bind: vec![],
            on_match: on_match.iter().map(|s| s.to_string()).collect(),
            description: None,
            tags: vec![],
        }
    }

    fn route_bind(pattern: &str, bindings: &[(&str, &str)], on_match: &[&str]) -> RouteSpec {
        RouteSpec {
            name: None,
            pattern: pattern.into(),
            bind: bindings
                .iter()
                .map(|(c, p)| (c.to_string(), p.to_string()))
                .collect(),
            on_match: on_match.iter().map(|s| s.to_string()).collect(),
            description: None,
            tags: vec![],
        }
    }

    #[test]
    fn literal_path_matches_exactly() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/about", &["load-about"]));
        assert!(reg.match_url("https://site.com/about").is_some());
        assert!(reg.match_url("https://site.com/other").is_none());
    }

    #[test]
    fn param_captures_single_segment() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/users/:id", &["load-user"]));
        let m = reg.match_url("https://site.com/users/42").unwrap();
        assert_eq!(m.params.get("id").map(String::as_str), Some("42"));
        assert_eq!(m.on_match, vec!["load-user"]);
    }

    #[test]
    fn multiple_params_all_captured_in_order() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/posts/:year/:month/:slug", &["load-post"]));
        let m = reg
            .match_url("https://blog.test/posts/2026/04/onward")
            .unwrap();
        assert_eq!(m.params.get("year").map(String::as_str), Some("2026"));
        assert_eq!(m.params.get("month").map(String::as_str), Some("04"));
        assert_eq!(m.params.get("slug").map(String::as_str), Some("onward"));
    }

    #[test]
    fn wildcard_matches_any_single_segment() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/*/edit", &["edit-whatever"]));
        assert!(reg.match_url("https://site.com/users/edit").is_some());
        assert!(reg.match_url("https://site.com/posts/edit").is_some());
        assert!(reg.match_url("https://site.com/users").is_none());
    }

    #[test]
    fn segment_count_mismatch_returns_none() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/users/:id", &["load-user"]));
        assert!(reg.match_url("https://site.com/users").is_none());
        assert!(reg.match_url("https://site.com/users/42/edit").is_none());
    }

    #[test]
    fn query_and_fragment_stripped_from_matching() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/search", &["run-search"]));
        assert!(reg.match_url("https://site.com/search?q=rust").is_some());
        assert!(reg.match_url("https://site.com/search#results").is_some());
    }

    #[test]
    fn root_path_matches_only_itself() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/", &["load-homepage"]));
        assert!(reg.match_url("https://example.com/").is_some());
        assert!(reg.match_url("https://example.com").is_some()); // no path = empty
        assert!(reg.match_url("https://example.com/about").is_none());
    }

    #[test]
    fn first_inserted_wins_on_overlap() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/users/me", &["load-me"]));
        reg.insert(route("/users/:id", &["load-user"]));
        let m = reg.match_url("https://site.com/users/me").unwrap();
        assert_eq!(m.on_match, vec!["load-me"]);
        let m = reg.match_url("https://site.com/users/other").unwrap();
        assert_eq!(m.on_match, vec!["load-user"]);
    }

    #[test]
    fn bindings_survive_match() {
        let mut reg = RouteRegistry::new();
        reg.insert(route_bind(
            "/users/:id",
            &[("current-user-id", "id")],
            &["load-user"],
        ));
        let m = reg.match_url("https://site.com/users/42").unwrap();
        assert_eq!(
            m.bindings,
            vec![("current-user-id".to_string(), "id".to_string())]
        );
        assert_eq!(m.params.get("id").map(String::as_str), Some("42"));
    }

    #[test]
    fn plain_path_input_matches_too() {
        let mut reg = RouteRegistry::new();
        reg.insert(route("/a/:b", &["x"]));
        let m = reg.match_url("/a/value").unwrap();
        assert_eq!(m.params.get("b").map(String::as_str), Some("value"));
    }

    #[test]
    fn extract_path_handles_various_shapes() {
        assert_eq!(extract_path("https://site.com/users/42"), "/users/42");
        assert_eq!(extract_path("https://site.com/users/42?x=1"), "/users/42");
        assert_eq!(
            extract_path("https://site.com/users/42#section"),
            "/users/42"
        );
        assert_eq!(extract_path("/users/42"), "/users/42");
        assert_eq!(extract_path("https://site.com"), "");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_route_specs() {
        let src = r#"
            (defroute :pattern "/users/:id"
                      :bind (("user-id" "id"))
                      :on-match ("load-user" "auto-reader-mode"))
            (defroute :name "blog-post"
                      :pattern "/posts/:year/:month/:slug"
                      :bind (("post-year" "year")
                             ("post-month" "month")
                             ("post-slug" "slug"))
                      :on-match ("load-post"))
            (defroute :pattern "/"
                      :on-match ("load-homepage"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].pattern, "/users/:id");
        assert_eq!(
            specs[0].bind,
            vec![("user-id".to_string(), "id".to_string())]
        );
        assert_eq!(specs[1].name.as_deref(), Some("blog-post"));
        assert_eq!(specs[1].bind.len(), 3);
        assert!(specs[2].bind.is_empty());
    }
}
