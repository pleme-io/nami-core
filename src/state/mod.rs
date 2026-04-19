//! Embedded-state extraction.
//!
//! Modern SPAs ship a snapshot of the page's state inside a
//! `<script type="application/json">` or `<script id="__NEXT_DATA__">`
//! tag so the client can hydrate without another round-trip. Those
//! blobs are structured data sitting in plain sight — pull them into
//! Lisp space alongside the DOM.
//!
//! Also extracts JSON-LD metadata (`<script type="application/ld+json">`)
//! which is where many sites publish their canonical structured data
//! for search engines.

use crate::dom::{Document, NodeData};
use serde::Serialize;
use serde_json::Value;

/// Kind of embedded state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateKind {
    /// `<script type="application/json">` — generic page state.
    ApplicationJson,
    /// `<script type="application/ld+json">` — schema.org metadata.
    JsonLd,
    /// `<script id="__NEXT_DATA__">` — Next.js hydration payload.
    NextData,
    /// `<script id="__NUXT_DATA__">` — Nuxt hydration payload.
    NuxtData,
    /// `window.__remixContext = {...}` pattern.
    RemixContext,
    /// `<script id="gatsby-data">` or equivalent.
    GatsbyData,
}

/// One state blob found in the document.
#[derive(Debug, Clone, Serialize)]
pub struct StateBlob {
    pub kind: StateKind,
    /// The `id` attribute of the script tag, if any.
    pub id: Option<String>,
    /// Either a parsed JSON value, or the raw text if parse failed.
    pub value: Option<Value>,
    /// When JSON parsing failed, the raw text so users can still inspect.
    pub raw: Option<String>,
    /// Approximate size of the payload in bytes (uncompressed).
    pub bytes: usize,
}

/// Find every embedded state payload in the document.
#[must_use]
pub fn extract(doc: &Document) -> Vec<StateBlob> {
    let mut out = Vec::new();
    for node in doc.root.descendants() {
        let NodeData::Element(el) = &node.data else {
            continue;
        };
        if el.tag != "script" {
            continue;
        }

        let ty = el.get_attribute("type").unwrap_or("").to_ascii_lowercase();
        let id = el.get_attribute("id").map(str::to_owned);

        let kind = if id.as_deref() == Some("__NEXT_DATA__") {
            StateKind::NextData
        } else if id.as_deref() == Some("__NUXT_DATA__") {
            StateKind::NuxtData
        } else if matches!(id.as_deref(), Some(s) if s.starts_with("gatsby-")) {
            StateKind::GatsbyData
        } else if ty == "application/ld+json" {
            StateKind::JsonLd
        } else if ty == "application/json" {
            StateKind::ApplicationJson
        } else {
            continue;
        };

        let raw = node.text_content();
        let bytes = raw.len();
        let parsed: Option<Value> = serde_json::from_str(raw.trim()).ok();
        let raw_slot = if parsed.is_none() { Some(raw) } else { None };

        out.push(StateBlob {
            kind,
            id,
            value: parsed,
            raw: raw_slot,
            bytes,
        });
    }

    // Also scrape for inline `window.__remixContext = {…}` scripts.
    for node in doc.root.descendants() {
        let NodeData::Element(el) = &node.data else {
            continue;
        };
        if el.tag != "script" {
            continue;
        }
        let body = node.text_content();
        if let Some(payload) = extract_remix_context(&body) {
            let bytes = payload.len();
            let parsed = serde_json::from_str::<Value>(&payload).ok();
            let raw_slot = if parsed.is_none() {
                Some(payload)
            } else {
                None
            };
            out.push(StateBlob {
                kind: StateKind::RemixContext,
                id: None,
                value: parsed,
                raw: raw_slot,
                bytes,
            });
        }
    }

    out
}

/// Very small targeted parser: find `window.__remixContext = ` and
/// grab the balanced JSON object or array that follows.
fn extract_remix_context(body: &str) -> Option<String> {
    let marker = "window.__remixContext";
    let idx = body.find(marker)?;
    let after = &body[idx + marker.len()..];
    let eq = after.find('=')?;
    let rest = after[eq + 1..].trim_start();
    balanced_json_prefix(rest).map(str::to_owned)
}

/// Return the longest prefix of `s` that is a syntactically-balanced
/// JSON value, or None if it doesn't start with `{` or `[`.
fn balanced_json_prefix(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (open, close) = match bytes[0] {
        b'{' => (b'{', b'}'),
        b'[' => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_next_data() {
        let doc = Document::parse(
            r#"<html><head>
                <script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{}},"page":"/"}</script>
               </head></html>"#,
        );
        let blobs = extract(&doc);
        let next = blobs
            .iter()
            .find(|b| b.kind == StateKind::NextData)
            .expect("next data");
        assert!(next.value.is_some());
        let v = next.value.as_ref().unwrap();
        assert_eq!(v["page"], "/");
    }

    #[test]
    fn extracts_generic_application_json() {
        let doc = Document::parse(
            r#"<html><body><script type="application/json" id="initial">{"a":1,"b":[2,3]}</script></body></html>"#,
        );
        let blobs = extract(&doc);
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].kind, StateKind::ApplicationJson);
        assert_eq!(blobs[0].id.as_deref(), Some("initial"));
        assert_eq!(blobs[0].value.as_ref().unwrap()["a"], 1);
    }

    #[test]
    fn extracts_json_ld_metadata() {
        let doc = Document::parse(
            r#"<html><head><script type="application/ld+json">
               {"@context":"https://schema.org","@type":"Article","name":"hi"}
               </script></head></html>"#,
        );
        let blobs = extract(&doc);
        let ld = blobs
            .iter()
            .find(|b| b.kind == StateKind::JsonLd)
            .expect("json-ld");
        assert_eq!(ld.value.as_ref().unwrap()["@type"], "Article");
    }

    #[test]
    fn extracts_remix_context() {
        let doc = Document::parse(
            r#"<html><body><script>
               window.__remixContext = {"routes":{"root":{}},"state":{"loaderData":{}}};
               </script></body></html>"#,
        );
        let blobs = extract(&doc);
        let remix = blobs
            .iter()
            .find(|b| b.kind == StateKind::RemixContext)
            .expect("remix context");
        assert!(remix.value.is_some());
    }

    #[test]
    fn malformed_json_preserved_as_raw() {
        let doc = Document::parse(
            r#"<html><head><script id="__NEXT_DATA__" type="application/json">{"oops": not valid</script></head></html>"#,
        );
        let blobs = extract(&doc);
        assert_eq!(blobs[0].kind, StateKind::NextData);
        assert!(blobs[0].value.is_none());
        assert!(blobs[0].raw.is_some());
    }

    #[test]
    fn ignores_plain_js_scripts() {
        let doc = Document::parse(r#"<html><head><script>console.log('x')</script></head></html>"#);
        let blobs = extract(&doc);
        assert!(blobs.is_empty());
    }

    #[test]
    fn balanced_json_prefix_handles_nested() {
        assert_eq!(
            super::balanced_json_prefix(r#"{"a":{"b":1}} rest"#),
            Some(r#"{"a":{"b":1}}"#)
        );
        assert_eq!(
            super::balanced_json_prefix("[1,[2,3],4];"),
            Some("[1,[2,3],4]")
        );
    }

    #[test]
    fn balanced_json_prefix_handles_strings_with_braces() {
        assert_eq!(
            super::balanced_json_prefix(r#"{"s":"}}}"} rest"#),
            Some(r#"{"s":"}}}"}"#)
        );
    }
}
