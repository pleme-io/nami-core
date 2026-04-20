//! `(defshare-target)` — declarative share destinations.
//!
//! Absorbs iOS/Android share sheets, Chrome Web Share API, Firefox
//! Send To..., and Edge Share. A share target is a named destination
//! + handler — mailto, clipboard, a store entry, a `{url}`-templated
//! HTTP POST, or an MCP tool invocation.
//!
//! ```lisp
//! (defshare-target :name    "slack"
//!                  :label   "Share to Slack"
//!                  :kind    :http-post
//!                  :url     "https://hooks.slack.com/services/…"
//!                  :body-template "{\"text\": \"{url}\"}")
//!
//! (defshare-target :name    "clipboard"
//!                  :label   "Copy link"
//!                  :kind    :clipboard)
//!
//! (defshare-target :name    "archive"
//!                  :label   "Save to Wayback"
//!                  :kind    :redirect
//!                  :url-template "https://web.archive.org/save/{url}")
//!
//! (defshare-target :name    "notes"
//!                  :label   "Append to notes"
//!                  :kind    :storage
//!                  :storage "reading-list")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// How to handle the share action.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShareKind {
    /// Write the shared URL to the system clipboard.
    Clipboard,
    /// Append a `{url}/{title}` entry to the named (defstorage) store.
    Storage,
    /// Navigate to `url_template` with `{url}` / `{title}` substituted.
    Redirect,
    /// POST to `url` with `body_template` JSON payload.
    HttpPost,
    /// Invoke an MCP tool by name with a predefined arg shape.
    Mcp,
    /// Open a `mailto:` URL.
    Email,
}

impl Default for ShareKind {
    fn default() -> Self {
        Self::Clipboard
    }
}

/// One share-target destination.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defshare-target"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShareTargetSpec {
    pub name: String,
    /// Display label in the share sheet.
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub kind: ShareKind,
    /// URL for HttpPost / Email destinations.
    #[serde(default)]
    pub url: Option<String>,
    /// Template URL for Redirect targets. Supports `{url}`, `{title}`.
    #[serde(default)]
    pub url_template: Option<String>,
    /// JSON body template for HttpPost. `{url}` / `{title}` replaced.
    #[serde(default)]
    pub body_template: Option<String>,
    /// Storage name for Storage targets.
    #[serde(default)]
    pub storage: Option<String>,
    /// MCP tool name for Mcp targets.
    #[serde(default)]
    pub mcp_tool: Option<String>,
    /// Runtime toggle.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

/// Payload handed to a share target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SharePayload {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

impl ShareTargetSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("share target name is empty".into());
        }
        if self.label.trim().is_empty() {
            return Err(format!(
                "share target '{}' has empty :label",
                self.name
            ));
        }
        match self.kind {
            ShareKind::Clipboard | ShareKind::Email => Ok(()),
            ShareKind::Redirect => {
                if self.url_template.is_none() {
                    return Err(format!(
                        "redirect target '{}' requires :url-template",
                        self.name
                    ));
                }
                Ok(())
            }
            ShareKind::HttpPost => {
                if self.url.is_none() {
                    return Err(format!(
                        "http-post target '{}' requires :url",
                        self.name
                    ));
                }
                Ok(())
            }
            ShareKind::Storage => {
                if self.storage.is_none() {
                    return Err(format!(
                        "storage target '{}' requires :storage",
                        self.name
                    ));
                }
                Ok(())
            }
            ShareKind::Mcp => {
                if self.mcp_tool.is_none() {
                    return Err(format!(
                        "mcp target '{}' requires :mcp-tool",
                        self.name
                    ));
                }
                Ok(())
            }
        }
    }

    /// Materialize the URL (Redirect targets).
    #[must_use]
    pub fn rendered_url(&self, payload: &SharePayload) -> Option<String> {
        let template = self.url_template.as_deref()?;
        Some(render_template(template, payload))
    }

    /// Materialize the body (HttpPost targets).
    #[must_use]
    pub fn rendered_body(&self, payload: &SharePayload) -> Option<String> {
        let template = self.body_template.as_deref()?;
        Some(render_template(template, payload))
    }
}

fn render_template(template: &str, payload: &SharePayload) -> String {
    let title = payload.title.as_deref().unwrap_or("");
    let text = payload.text.as_deref().unwrap_or("");
    template
        .replace("{url}", &payload.url)
        .replace("{title}", title)
        .replace("{text}", text)
}

/// Registry of share targets.
#[derive(Debug, Clone, Default)]
pub struct ShareRegistry {
    specs: Vec<ShareTargetSpec>,
}

impl ShareRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ShareTargetSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ShareTargetSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defshare-target '{}' rejected: {}", s.name, e);
            }
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
    pub fn specs(&self) -> &[ShareTargetSpec] {
        &self.specs
    }

    #[must_use]
    pub fn enabled(&self) -> Vec<&ShareTargetSpec> {
        self.specs.iter().filter(|s| s.enabled).collect()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ShareTargetSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ShareTargetSpec>, String> {
    tatara_lisp::compile_typed::<ShareTargetSpec>(src)
        .map_err(|e| format!("failed to compile defshare-target forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ShareTargetSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, kind: ShareKind) -> ShareTargetSpec {
        ShareTargetSpec {
            name: name.into(),
            label: format!("label for {name}"),
            icon: None,
            kind,
            url: None,
            url_template: None,
            body_template: None,
            storage: None,
            mcp_tool: None,
            enabled: true,
            description: None,
        }
    }

    fn payload() -> SharePayload {
        SharePayload {
            url: "https://example.com/post?id=1".into(),
            title: Some("Hello".into()),
            text: None,
        }
    }

    #[test]
    fn clipboard_validates_with_no_extras() {
        assert!(sample("cb", ShareKind::Clipboard).validate().is_ok());
    }

    #[test]
    fn redirect_requires_url_template() {
        let s = sample("archive", ShareKind::Redirect);
        assert!(s.validate().is_err());
        let ok = ShareTargetSpec {
            url_template: Some("https://web.archive.org/save/{url}".into()),
            ..s
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn http_post_requires_url() {
        let s = sample("slack", ShareKind::HttpPost);
        assert!(s.validate().is_err());
        let ok = ShareTargetSpec {
            url: Some("https://hook".into()),
            ..s
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn storage_requires_name() {
        assert!(sample("notes", ShareKind::Storage).validate().is_err());
    }

    #[test]
    fn mcp_requires_tool_name() {
        assert!(sample("k", ShareKind::Mcp).validate().is_err());
    }

    #[test]
    fn rendered_url_substitutes_placeholders() {
        let s = ShareTargetSpec {
            url_template: Some("https://web.archive.org/save/{url}".into()),
            ..sample("archive", ShareKind::Redirect)
        };
        let out = s.rendered_url(&payload()).unwrap();
        assert_eq!(
            out,
            "https://web.archive.org/save/https://example.com/post?id=1"
        );
    }

    #[test]
    fn rendered_body_substitutes_url_and_title() {
        let s = ShareTargetSpec {
            body_template: Some(r#"{"url":"{url}","title":"{title}"}"#.into()),
            ..sample("slack", ShareKind::HttpPost)
        };
        let out = s.rendered_body(&payload()).unwrap();
        assert!(out.contains("https://example.com/post?id=1"));
        assert!(out.contains("Hello"));
    }

    #[test]
    fn registry_insert_validates() {
        let mut reg = ShareRegistry::new();
        assert!(reg
            .insert(sample("archive", ShareKind::Redirect))
            .is_err());
        assert!(reg.insert(sample("cb", ShareKind::Clipboard)).is_ok());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = ShareRegistry::new();
        reg.insert(sample("cb", ShareKind::Clipboard)).unwrap();
        let updated = ShareTargetSpec {
            label: "Renamed".into(),
            ..sample("cb", ShareKind::Clipboard)
        };
        reg.insert(updated).unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].label, "Renamed");
    }

    #[test]
    fn enabled_filters_paused() {
        let mut reg = ShareRegistry::new();
        reg.insert(sample("a", ShareKind::Clipboard)).unwrap();
        let mut b = sample("b", ShareKind::Clipboard);
        b.enabled = false;
        reg.insert(b).unwrap();
        let e = reg.enabled();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "a");
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_share_form() {
        let src = r#"
            (defshare-target :name    "archive"
                             :label   "Save to Wayback"
                             :kind    "redirect"
                             :url-template "https://web.archive.org/save/{url}")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "archive");
        assert_eq!(s.kind, ShareKind::Redirect);
        assert!(s.url_template.is_some());
    }
}
