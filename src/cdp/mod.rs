//! Chrome DevTools Protocol — server-side method dispatch core.
//!
//! The pleme-io absorption of Obscura's CDP server, **standing on the
//! [`BrowserEngine`](crate::engine::BrowserEngine) seam**: a CDP client
//! (Puppeteer / Playwright / curupira) drives *any* pleme-io engine — Servo,
//! wry, or the null engine — through one typed [`dispatch`]. Where Obscura
//! hand-binds CDP to its V8 engine, here the protocol binds to the trait, so
//! the same CDP surface works for every engine behind the seam.
//!
//! This module is the pure protocol + dispatch core — fully testable with a
//! mock engine, no transport. The WebSocket endpoint (`ws://…:9222/devtools/
//! browser`) is a thin host-side wrapper (follow-up): parse a frame into
//! [`CdpRequest`], call [`dispatch`], serialize [`CdpOutcome::to_frame`].
//!
//! Engine gaps surface as typed CDP errors, never silent successes — e.g.
//! `Runtime.evaluate` against an engine with no JS host returns a CDP error
//! naming the engine, mirroring the [`EngineError`](crate::engine::EngineError)
//! contract.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::{BrowserEngine, EngineError};

/// A CDP request frame: `{ "id", "method", "params" }`.
#[derive(Debug, Clone, Deserialize)]
pub struct CdpRequest {
    pub id: i64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl CdpRequest {
    /// Parse a raw JSON-RPC frame into a request.
    ///
    /// # Errors
    /// Returns the `serde_json` error if the frame is not a valid CDP request.
    pub fn parse(frame: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(frame)
    }
}

/// The result of dispatching a CDP method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CdpOutcome {
    /// A `result` payload.
    Result(Value),
    /// An `error` payload — `code` follows JSON-RPC conventions
    /// (`-32601` method-not-found, `-32602` invalid-params, `-32000`
    /// engine/server error).
    Error { code: i64, message: String },
}

impl CdpOutcome {
    /// Serialize to the full CDP wire frame for `id`. The frame is built as a
    /// typed `serde_json::Value` (TYPED EMISSION — never string-concatenated).
    #[must_use]
    pub fn to_frame(&self, id: i64) -> Value {
        match self {
            CdpOutcome::Result(v) => json!({ "id": id, "result": v }),
            CdpOutcome::Error { code, message } => {
                json!({ "id": id, "error": { "code": code, "message": message } })
            }
        }
    }

    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self, CdpOutcome::Error { .. })
    }
}

/// Dispatch one CDP method against `engine`.
///
/// Implemented domains (extend by adding match arms — each maps to a
/// `BrowserEngine` call): `Browser.getVersion`, `Page.navigate`,
/// `Runtime.evaluate`, `Target.getTargets`. Unknown methods return the CDP
/// standard `-32601 method not found`.
#[must_use]
pub fn dispatch(req: &CdpRequest, engine: &mut dyn BrowserEngine) -> CdpOutcome {
    match req.method.as_str() {
        "Browser.getVersion" => CdpOutcome::Result(json!({
            "protocolVersion": "1.3",
            "product": "pleme-io-namimado",
            "revision": "0",
            "userAgent": "",
            "jsVersion": "",
            // pleme-io extension: which engine is actually backing this session.
            "plemeEngine": engine.name(),
        })),

        "Page.navigate" => match req.params.get("url").and_then(Value::as_str) {
            Some(u) => match url::Url::parse(u) {
                Ok(parsed) => {
                    engine.navigate(&parsed);
                    CdpOutcome::Result(json!({ "frameId": "0", "loaderId": "0" }))
                }
                Err(e) => invalid_params(&e.to_string()),
            },
            None => invalid_params("missing params.url"),
        },

        "Runtime.evaluate" => match req.params.get("expression").and_then(Value::as_str) {
            Some(expr) => match engine.evaluate_js(expr) {
                Ok(()) => CdpOutcome::Result(json!({ "result": { "type": "undefined" } })),
                Err(EngineError::Unsupported { engine: eng, .. }) => CdpOutcome::Error {
                    code: -32000,
                    message: {
                        // serde-bound message; engine name is a &'static str.
                        let mut m = String::from(eng);
                        m.push_str(" engine has no JavaScript host");
                        m
                    },
                },
                Err(e) => CdpOutcome::Error {
                    code: -32000,
                    message: e.to_string(),
                },
            },
            None => invalid_params("missing params.expression"),
        },

        "Target.getTargets" => CdpOutcome::Result(json!({
            "targetInfos": [{
                "targetId": "0",
                "type": "page",
                "title": "",
                "url": "",
                "attached": true,
            }]
        })),

        other => CdpOutcome::Error {
            code: -32601,
            message: {
                let mut m = String::from("method not found: ");
                m.push_str(other);
                m
            },
        },
    }
}

fn invalid_params(detail: &str) -> CdpOutcome {
    let mut m = String::from("invalid params: ");
    m.push_str(detail);
    CdpOutcome::Error {
        code: -32602,
        message: m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ContentRect;
    use url::Url;

    /// A recording engine: logs navigations, and `evaluate_js` is configurable
    /// (Some(()) ⇒ a JS host exists; None ⇒ default `Unsupported`).
    #[derive(Default)]
    struct RecEngine {
        navigated: Vec<String>,
        has_js: bool,
    }

    impl BrowserEngine for RecEngine {
        fn name(&self) -> &'static str {
            "rec"
        }
        fn navigate(&mut self, url: &Url) {
            self.navigated.push(url.to_string());
        }
        fn resize(&mut self, _rect: ContentRect) {}
        fn evaluate_js(&mut self, _s: &str) -> Result<(), EngineError> {
            if self.has_js {
                Ok(())
            } else {
                Err(EngineError::Unsupported {
                    what: "evaluate_js",
                    engine: "rec",
                })
            }
        }
        fn renders_pixels(&self) -> bool {
            true
        }
    }

    fn req(id: i64, method: &str, params: Value) -> CdpRequest {
        CdpRequest {
            id,
            method: method.into(),
            params,
        }
    }

    #[test]
    fn browser_get_version_reports_engine() {
        let mut e = RecEngine::default();
        let out = dispatch(&req(1, "Browser.getVersion", Value::Null), &mut e);
        match out {
            CdpOutcome::Result(v) => {
                assert_eq!(v["protocolVersion"], "1.3");
                assert_eq!(v["plemeEngine"], "rec");
            }
            CdpOutcome::Error { .. } => panic!("expected result"),
        }
    }

    #[test]
    fn page_navigate_drives_engine() {
        let mut e = RecEngine::default();
        let out = dispatch(
            &req(2, "Page.navigate", json!({ "url": "https://example.com/" })),
            &mut e,
        );
        assert!(!out.is_error());
        assert_eq!(e.navigated, vec!["https://example.com/"]);
    }

    #[test]
    fn page_navigate_missing_url_is_invalid_params() {
        let mut e = RecEngine::default();
        let out = dispatch(&req(3, "Page.navigate", json!({})), &mut e);
        assert_eq!(
            out,
            CdpOutcome::Error {
                code: -32602,
                message: "invalid params: missing params.url".into()
            }
        );
        assert!(e.navigated.is_empty());
    }

    #[test]
    fn page_navigate_bad_url_is_invalid_params() {
        let mut e = RecEngine::default();
        let out = dispatch(
            &req(4, "Page.navigate", json!({ "url": "not a url" })),
            &mut e,
        );
        assert!(matches!(out, CdpOutcome::Error { code: -32602, .. }));
    }

    #[test]
    fn runtime_evaluate_without_js_host_is_typed_error() {
        let mut e = RecEngine { has_js: false, ..Default::default() };
        let out = dispatch(
            &req(5, "Runtime.evaluate", json!({ "expression": "1+1" })),
            &mut e,
        );
        match out {
            CdpOutcome::Error { code, message } => {
                assert_eq!(code, -32000);
                assert!(message.contains("no JavaScript host"), "got: {message}");
            }
            CdpOutcome::Result(_) => panic!("expected a typed error, not a silent success"),
        }
    }

    #[test]
    fn runtime_evaluate_with_js_host_succeeds() {
        let mut e = RecEngine { has_js: true, ..Default::default() };
        let out = dispatch(
            &req(6, "Runtime.evaluate", json!({ "expression": "1+1" })),
            &mut e,
        );
        assert!(!out.is_error());
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let mut e = RecEngine::default();
        let out = dispatch(&req(7, "Fantasy.doThing", Value::Null), &mut e);
        assert!(matches!(out, CdpOutcome::Error { code: -32601, .. }));
    }

    #[test]
    fn target_get_targets_returns_one_page() {
        let mut e = RecEngine::default();
        let out = dispatch(&req(8, "Target.getTargets", Value::Null), &mut e);
        match out {
            CdpOutcome::Result(v) => {
                assert_eq!(v["targetInfos"][0]["type"], "page");
            }
            CdpOutcome::Error { .. } => panic!("expected result"),
        }
    }

    #[test]
    fn to_frame_shapes_result_and_error() {
        let r = CdpOutcome::Result(json!({ "ok": true })).to_frame(9);
        assert_eq!(r, json!({ "id": 9, "result": { "ok": true } }));
        let e = CdpOutcome::Error {
            code: -32601,
            message: "method not found: X".into(),
        }
        .to_frame(10);
        assert_eq!(
            e,
            json!({ "id": 10, "error": { "code": -32601, "message": "method not found: X" } })
        );
    }

    #[test]
    fn request_parses_from_wire_frame() {
        let r = CdpRequest::parse(r#"{"id":11,"method":"Page.navigate","params":{"url":"https://a.io/"}}"#)
            .unwrap();
        assert_eq!(r.id, 11);
        assert_eq!(r.method, "Page.navigate");
        assert_eq!(r.params["url"], "https://a.io/");
    }

    #[test]
    fn request_params_default_when_absent() {
        let r = CdpRequest::parse(r#"{"id":12,"method":"Browser.getVersion"}"#).unwrap();
        assert_eq!(r.params, Value::Null);
    }
}
