//! Lisp-declared thin HTTP / GraphQL client.
//!
//! A [`QuerySpec`] declares an HTTP request (any method, any body
//! shape) whose response populates a named state cell. This is the
//! data-inbound half of the modern-web data-flow story:
//!
//! ```lisp
//! ; REST
//! (defquery :name "current-user"
//!           :endpoint "https://api.example.com/me"
//!           :method "GET"
//!           :into "user")
//!
//! ; GraphQL
//! (defquery :name "list-posts"
//!           :endpoint "https://api.example.com/graphql"
//!           :method "POST"
//!           :headers (("content-type" "application/json"))
//!           :body "{\"query\":\"{ posts { id title author { name } } }\"}"
//!           :into "posts")
//!
//! ; Quota/cache hint (semantics up to the runtime):
//! (defquery :name "public-holidays"
//!           :endpoint "https://date.nager.at/api/v3/PublicHolidays/2026/US"
//!           :method "GET"
//!           :into "holidays"
//!           :cache-ttl-secs 86400)
//! ```
//!
//! Transport-agnostic. `nami-core` defines the spec + a
//! [`Fetcher`] trait; a runtime (nami browser, a test harness, an
//! MCP server) provides the actual HTTP impl. Keeps nami-core slim
//! and lets us swap in tor-routed / proxy-aware / mocked fetchers
//! without library changes.
//!
//! Responses are parsed as JSON (falling back to a plain string if
//! parsing fails) and written into the configured `:into` state
//! cell. After `run_query` returns, a `defderived` computation or a
//! component template can read `(@ user)` / `(@ posts)` / etc. and
//! get typed data.

use crate::store::StateStore;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One header entry — serializable as `(:name "content-type" :value
/// "application/json")` or the more ergonomic positional tuple
/// `("content-type" "application/json")` depending on the tatara-lisp
/// surface. We take the simpler positional form.
pub type HeaderPair = (String, String);

/// A named query declaration.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defquery"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuerySpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// URL to request. V1 accepts a literal string; future versions
    /// could allow `(: EXPR)` to interpolate state cells.
    pub endpoint: String,
    /// HTTP method (GET / POST / PUT / etc.). Defaults to GET when
    /// empty.
    #[serde(default)]
    pub method: String,
    /// Raw request body — typically a GraphQL JSON payload or a
    /// JSON-serialized form object. V1 is a literal string.
    #[serde(default)]
    pub body: Option<String>,
    /// Request headers. Either one pair per entry or many — all sent.
    #[serde(default)]
    pub headers: Vec<HeaderPair>,
    /// Name of the state cell to write the parsed response into. If
    /// parsing as JSON fails, the raw body text is written as a
    /// string value.
    pub into: String,
    /// Hint to any caching layer (TTL in seconds). Pure data — the
    /// fetcher decides whether to honor it.
    #[serde(default)]
    pub cache_ttl_secs: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl QuerySpec {
    #[must_use]
    pub fn method_or_default(&self) -> &str {
        if self.method.is_empty() {
            "GET"
        } else {
            &self.method
        }
    }
}

/// Transport abstraction. A real browser plugs in reqwest / fetch
/// API / whatever; tests inject a mock.
pub trait Fetcher {
    /// Perform one HTTP request. The implementation should handle
    /// TLS, redirects, timeouts — nami-core doesn't care.
    fn fetch(
        &self,
        url: &str,
        method: &str,
        body: Option<&str>,
        headers: &[HeaderPair],
    ) -> Result<String, String>;
}

/// Registry of queries by name.
#[derive(Debug, Clone, Default)]
pub struct QueryRegistry {
    specs: Vec<QuerySpec>,
}

impl QueryRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: QuerySpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = QuerySpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&QuerySpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Run a named query via the given fetcher, writing the result
    /// into the configured `:into` state cell.
    pub fn run(
        &self,
        name: &str,
        fetcher: &dyn Fetcher,
        store: &StateStore,
    ) -> Result<QueryReport, String> {
        let spec = self
            .get(name)
            .ok_or_else(|| format!("unknown query: {name}"))?;
        run_spec(spec, fetcher, store)
    }
}

/// Per-query outcome.
#[derive(Debug, Clone)]
pub struct QueryReport {
    pub query: String,
    pub into: String,
    pub bytes: usize,
    pub parsed_json: bool,
}

/// Execute one spec against one fetcher. Writes the response into
/// `store` at the spec's `:into` cell.
pub fn run_spec(
    spec: &QuerySpec,
    fetcher: &dyn Fetcher,
    store: &StateStore,
) -> Result<QueryReport, String> {
    let body = spec.body.as_deref();
    let raw = fetcher.fetch(
        &spec.endpoint,
        spec.method_or_default(),
        body,
        &spec.headers,
    )?;
    let bytes = raw.len();
    let (value, parsed_json) = match serde_json::from_str::<JsonValue>(&raw) {
        Ok(v) => (v, true),
        Err(_) => (JsonValue::String(raw), false),
    };
    store.set(&spec.into, value);
    Ok(QueryReport {
        query: spec.name.clone(),
        into: spec.into.clone(),
        bytes,
        parsed_json,
    })
}

/// Run every query in registration order. Errors are collected per-
/// query; the batch continues so later queries can still populate
/// their cells.
pub fn run_all(
    registry: &QueryRegistry,
    fetcher: &dyn Fetcher,
    store: &StateStore,
) -> Vec<Result<QueryReport, String>> {
    registry
        .specs
        .iter()
        .map(|s| run_spec(s, fetcher, store))
        .collect()
}

/// Compile a Lisp document of `(defquery …)` forms.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<QuerySpec>, String> {
    tatara_lisp::compile_typed::<QuerySpec>(src).map_err(|e| format!("{e}"))
}

/// Register the `defquery` keyword.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<QuerySpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    /// Mock fetcher that returns canned responses keyed by URL.
    struct MockFetcher {
        responses: RefCell<Vec<(String, Result<String, String>)>>,
        calls: RefCell<Vec<(String, String, Option<String>, Vec<HeaderPair>)>>,
    }

    impl MockFetcher {
        fn new() -> Self {
            Self {
                responses: RefCell::new(Vec::new()),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn on(&self, url: &str, response: Result<String, String>) -> &Self {
            self.responses
                .borrow_mut()
                .push((url.to_string(), response));
            self
        }
    }

    impl Fetcher for MockFetcher {
        fn fetch(
            &self,
            url: &str,
            method: &str,
            body: Option<&str>,
            headers: &[HeaderPair],
        ) -> Result<String, String> {
            self.calls.borrow_mut().push((
                url.to_string(),
                method.to_string(),
                body.map(str::to_owned),
                headers.to_vec(),
            ));
            for (u, r) in self.responses.borrow().iter() {
                if u == url {
                    return r.clone();
                }
            }
            Err(format!("mock: no canned response for {url}"))
        }
    }

    fn qspec(name: &str, endpoint: &str, into: &str) -> QuerySpec {
        QuerySpec {
            name: name.into(),
            description: None,
            endpoint: endpoint.into(),
            method: String::new(),
            body: None,
            headers: vec![],
            into: into.into(),
            cache_ttl_secs: None,
            tags: vec![],
        }
    }

    #[test]
    fn get_request_parses_json_into_state() {
        let fetcher = MockFetcher::new();
        fetcher.on(
            "https://api.test/me",
            Ok(r#"{"name":"Alice","id":1}"#.to_string()),
        );
        let store = StateStore::new();
        let mut reg = QueryRegistry::new();
        reg.insert(qspec("me", "https://api.test/me", "user"));
        let report = reg.run("me", &fetcher, &store).unwrap();
        assert!(report.parsed_json);
        assert_eq!(store.get("user"), Some(json!({"name": "Alice", "id": 1})));
    }

    #[test]
    fn non_json_response_stored_as_string() {
        let fetcher = MockFetcher::new();
        fetcher.on("https://test/text", Ok("plain text response".into()));
        let store = StateStore::new();
        let mut reg = QueryRegistry::new();
        reg.insert(qspec("txt", "https://test/text", "content"));
        let report = reg.run("txt", &fetcher, &store).unwrap();
        assert!(!report.parsed_json);
        assert_eq!(store.get("content"), Some(json!("plain text response")));
    }

    #[test]
    fn default_method_is_get() {
        let fetcher = MockFetcher::new();
        fetcher.on("https://test/x", Ok("null".into()));
        let store = StateStore::new();
        let mut reg = QueryRegistry::new();
        reg.insert(qspec("q", "https://test/x", "v"));
        reg.run("q", &fetcher, &store).unwrap();
        let calls = fetcher.calls.borrow();
        assert_eq!(calls[0].1, "GET");
    }

    #[test]
    fn post_with_body_and_headers_sent() {
        let fetcher = MockFetcher::new();
        fetcher.on("https://test/gql", Ok(r#"{"data":{}}"#.into()));
        let store = StateStore::new();
        let mut reg = QueryRegistry::new();
        let mut spec = qspec("gql", "https://test/gql", "data");
        spec.method = "POST".into();
        spec.body = Some(r#"{"query":"{ _ }"}"#.into());
        spec.headers = vec![("content-type".into(), "application/json".into())];
        reg.insert(spec);
        reg.run("gql", &fetcher, &store).unwrap();
        let calls = fetcher.calls.borrow();
        assert_eq!(calls[0].1, "POST");
        assert_eq!(calls[0].2.as_deref(), Some(r#"{"query":"{ _ }"}"#));
        assert_eq!(calls[0].3.len(), 1);
        assert_eq!(calls[0].3[0].0, "content-type");
    }

    #[test]
    fn unknown_query_errors() {
        let fetcher = MockFetcher::new();
        let store = StateStore::new();
        let reg = QueryRegistry::new();
        assert!(reg.run("ghost", &fetcher, &store).is_err());
    }

    #[test]
    fn fetcher_error_propagates() {
        let fetcher = MockFetcher::new();
        fetcher.on("https://test/x", Err("connection refused".into()));
        let store = StateStore::new();
        let mut reg = QueryRegistry::new();
        reg.insert(qspec("q", "https://test/x", "v"));
        let err = reg.run("q", &fetcher, &store).unwrap_err();
        assert!(err.contains("connection refused"));
        // State cell unchanged on error.
        assert!(store.get("v").is_none());
    }

    #[test]
    fn run_all_continues_through_failures() {
        let fetcher = MockFetcher::new();
        fetcher
            .on("https://test/ok", Ok(r#"{"hello":"world"}"#.into()))
            .on("https://test/boom", Err("500".into()))
            .on("https://test/after", Ok(r#"[1,2,3]"#.into()));
        let store = StateStore::new();
        let mut reg = QueryRegistry::new();
        reg.insert(qspec("a", "https://test/ok", "a"));
        reg.insert(qspec("b", "https://test/boom", "b"));
        reg.insert(qspec("c", "https://test/after", "c"));
        let results = run_all(&reg, &fetcher, &store);
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        assert!(results[2].is_ok());
        // Even though b failed, c ran.
        assert!(store.get("a").is_some());
        assert!(store.get("b").is_none());
        assert_eq!(store.get("c"), Some(json!([1, 2, 3])));
    }

    #[test]
    fn query_then_derived_is_the_data_flow() {
        use crate::derived::{DerivedRegistry, DerivedSpec};

        // 1. Query fills `user` state cell.
        let fetcher = MockFetcher::new();
        fetcher.on(
            "https://api/me",
            Ok(r#"{"name":"Alice","posts":42}"#.into()),
        );
        let store = StateStore::new();
        let mut queries = QueryRegistry::new();
        queries.insert(qspec("me", "https://api/me", "user"));
        queries.run("me", &fetcher, &store).unwrap();

        // 2. Derived reads from `user` (as a whole JSON object).
        //    For V1 we don't have field access in tatara-eval,
        //    but we can at least confirm the store has what
        //    downstream consumers expect.
        assert_eq!(
            store.get("user"),
            Some(json!({"name": "Alice", "posts": 42}))
        );

        // 3. A separate state cell holds an integer we can
        //    derive off of.
        store.set("posts", json!(42));
        #[cfg(feature = "eval")]
        {
            let mut derived = DerivedRegistry::new();
            derived.insert(DerivedSpec {
                name: "posts-plus-one".into(),
                description: None,
                inputs: vec!["posts".into()],
                compute: "(+ posts 1)".into(),
            });
            assert_eq!(
                derived.evaluate("posts-plus-one", &store).unwrap(),
                json!(43)
            );
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn lisp_round_trip_query_specs() {
        let src = r#"
            (defquery :name "current-user"
                      :endpoint "https://api.example.com/me"
                      :method "GET"
                      :into "user")
            (defquery :name "list-posts"
                      :endpoint "https://api.example.com/graphql"
                      :method "POST"
                      :body "{\"query\":\"{ posts { id } }\"}"
                      :headers (("content-type" "application/json"))
                      :into "posts"
                      :cache-ttl-secs 300
                      :tags ("content" "list"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].method_or_default(), "GET");
        assert_eq!(specs[1].method, "POST");
        assert_eq!(specs[1].cache_ttl_secs, Some(300));
        assert_eq!(specs[1].headers.len(), 1);
        assert_eq!(specs[1].tags, vec!["content", "list"]);
    }
}
