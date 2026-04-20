//! `(defllm-provider)` — declarative LLM provider + pluggable trait.
//!
//! The AI foundation for nami. Every AI-flavored DSL in nami-core
//! (`defsummarize`, `defchat-with-page`, `defllm-completion`)
//! dispatches through the [`LlmProvider`] trait; real engines plug
//! in as trait-object impls behind feature flags. Same pattern as
//! [`crate::js_runtime::JsRuntime`].
//!
//! Supported provider kinds at the spec level:
//! - OpenAI-compatible REST (OpenAI, OpenRouter, Together, Groq,
//!   Fireworks, local Ollama, LM Studio, vLLM, llama.cpp server)
//! - Anthropic (Claude)
//! - Gemini (Google AI)
//! - Local `ollama` CLI
//! - pleme-io kurage (MCP-driven agent fleet)
//! - Mcp — arbitrary MCP tool that implements a generate schema
//! - Stub — the [`EchoProvider`] bundled here for tests + when no
//!   real provider is wired, mirrors input + records captured prompts
//!
//! ```lisp
//! (defllm-provider :name     "claude"
//!                  :kind     :anthropic
//!                  :endpoint "https://api.anthropic.com"
//!                  :model    "claude-opus-4-7"
//!                  :max-tokens 4096
//!                  :auth-env "ANTHROPIC_API_KEY")
//!
//! (defllm-provider :name  "local"
//!                  :kind  :ollama
//!                  :model "llama3.1:70b")
//!
//! (defllm-provider :name  "router"
//!                  :kind  :openai-compatible
//!                  :endpoint "https://openrouter.ai/api/v1"
//!                  :model    "anthropic/claude-opus-4")
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Upstream LLM protocol.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LlmKind {
    /// OpenAI REST shape — also serves OpenRouter, Together AI,
    /// Groq, Fireworks, vLLM, llama.cpp server, LM Studio, Ollama's
    /// OpenAI-compat endpoint.
    OpenAiCompatible,
    /// Anthropic Messages API.
    Anthropic,
    /// Google AI Gemini REST.
    Gemini,
    /// Local `ollama` CLI.
    Ollama,
    /// pleme-io Kurage — cloud agent fleet.
    Kurage,
    /// Arbitrary MCP tool that implements a generate schema.
    Mcp,
    /// Built-in echo provider — safe fallback for tests + when no
    /// real provider is wired.
    Stub,
}

impl Default for LlmKind {
    fn default() -> Self {
        Self::Stub
    }
}

/// Provider declaration.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defllm-provider"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderSpec {
    pub name: String,
    #[serde(default)]
    pub kind: LlmKind,
    /// REST endpoint (ignored by Ollama/Stub/Mcp).
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Model identifier at the upstream.
    pub model: String,
    /// Max tokens in a single response. 0 = provider default.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Generation temperature. Clamped to `[0.0, 2.0]` at call time.
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Env var name holding the API key. Resolved at call time —
    /// not stored in the spec, never serialized.
    #[serde(default)]
    pub auth_env: Option<String>,
    /// MCP tool name when `kind == Mcp`.
    #[serde(default)]
    pub mcp_tool: Option<String>,
    /// Max requests per minute. `0` = unlimited.
    #[serde(default)]
    pub rate_limit_per_minute: u32,
    /// Timeout in seconds per request.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_max_tokens() -> u32 {
    4096
}
fn default_temperature() -> f32 {
    0.7
}
fn default_timeout() -> u32 {
    60
}

impl LlmProviderSpec {
    #[must_use]
    pub fn clamped_temperature(&self) -> f32 {
        self.temperature.clamp(0.0, 2.0)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("llm provider name is empty".into());
        }
        if self.model.trim().is_empty() {
            return Err(format!(
                "llm provider '{}' has empty :model",
                self.name
            ));
        }
        match self.kind {
            LlmKind::OpenAiCompatible | LlmKind::Anthropic | LlmKind::Gemini => {
                if self.endpoint.is_none() && !matches!(self.kind, LlmKind::Anthropic) {
                    // Anthropic defaults to https://api.anthropic.com
                    return Err(format!(
                        "{:?} provider '{}' requires :endpoint",
                        self.kind, self.name
                    ));
                }
            }
            LlmKind::Mcp => {
                if self.mcp_tool.is_none() {
                    return Err(format!(
                        "mcp provider '{}' requires :mcp-tool",
                        self.name
                    ));
                }
            }
            LlmKind::Ollama | LlmKind::Kurage | LlmKind::Stub => {}
        }
        Ok(())
    }
}

/// Provider registry.
#[derive(Debug, Clone, Default)]
pub struct LlmProviderRegistry {
    specs: Vec<LlmProviderSpec>,
}

impl LlmProviderRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: LlmProviderSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = LlmProviderSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defllm-provider '{}' rejected: {}", s.name, e);
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
    pub fn specs(&self) -> &[LlmProviderSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&LlmProviderSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

// ─── runtime trait ───────────────────────────────────────────────

/// One message in a chat-shaped request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LlmMessage {
    /// `system` | `user` | `assistant`.
    pub role: String,
    pub content: String,
}

/// Input to one call.
#[derive(Debug, Clone, Default)]
pub struct LlmCall {
    pub messages: Vec<LlmMessage>,
    /// Override provider temperature when Some.
    pub temperature: Option<f32>,
    /// Override provider max_tokens when Some.
    pub max_tokens: Option<u32>,
    /// Stop sequences.
    pub stop: Vec<String>,
    /// Key/value metadata propagated through to providers that
    /// support it (tracing ids, user hints).
    pub metadata: HashMap<String, String>,
}

/// Completion outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmResponse {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub model: String,
    pub stopped: StopReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    StopSequence,
    MaxTokens,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    InvalidSpec(String),
    Network(String),
    RateLimited,
    AuthFailure,
    Unsupported(String),
    Parse(String),
    Timeout,
    Other(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSpec(m) => write!(f, "invalid spec: {m}"),
            Self::Network(m) => write!(f, "network error: {m}"),
            Self::RateLimited => write!(f, "rate limited"),
            Self::AuthFailure => write!(f, "authentication failed"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
            Self::Parse(m) => write!(f, "parse error: {m}"),
            Self::Timeout => write!(f, "timeout"),
            Self::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for LlmError {}

/// Pluggable provider trait. Real engines implement this; the
/// substrate dispatches summarize / chat / completion calls through
/// it without caring whether the backend is HTTP, CLI, or in-process.
pub trait LlmProvider: std::fmt::Debug + Send + Sync {
    fn generate(
        &self,
        spec: &LlmProviderSpec,
        call: &LlmCall,
    ) -> Result<LlmResponse, LlmError>;

    fn engine_name(&self) -> &'static str;
}

/// Trivial echo provider — always returns the concatenated content
/// of every `user` message. Useful for testing the dispatch
/// pipeline; real builds swap it out for an HTTP-backed impl.
#[derive(Debug, Clone, Default)]
pub struct EchoProvider {
    /// When Some, records every prompt the provider sees (for tests).
    pub trace: Option<Arc<Mutex<Vec<Vec<LlmMessage>>>>>,
}

impl LlmProvider for EchoProvider {
    fn generate(
        &self,
        spec: &LlmProviderSpec,
        call: &LlmCall,
    ) -> Result<LlmResponse, LlmError> {
        if let Some(t) = &self.trace {
            if let Ok(mut v) = t.lock() {
                v.push(call.messages.clone());
            }
        }
        let body = call
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(LlmResponse {
            content: format!("echo: {body}"),
            input_tokens: body.split_whitespace().count() as u32,
            output_tokens: (body.len() as u32 / 4).max(1),
            model: spec.model.clone(),
            stopped: StopReason::EndTurn,
        })
    }

    fn engine_name(&self) -> &'static str {
        "echo"
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<LlmProviderSpec>, String> {
    tatara_lisp::compile_typed::<LlmProviderSpec>(src)
        .map_err(|e| format!("failed to compile defllm-provider forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<LlmProviderSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LlmProviderSpec {
        LlmProviderSpec {
            name: "claude".into(),
            kind: LlmKind::Anthropic,
            endpoint: None,
            model: "claude-opus-4-7".into(),
            max_tokens: 4096,
            temperature: 0.7,
            auth_env: Some("ANTHROPIC_API_KEY".into()),
            mcp_tool: None,
            rate_limit_per_minute: 60,
            timeout_seconds: 60,
            description: None,
        }
    }

    #[test]
    fn validate_requires_non_empty_name_and_model() {
        assert!(LlmProviderSpec {
            name: String::new(),
            ..sample()
        }
        .validate()
        .is_err());
        assert!(LlmProviderSpec {
            model: String::new(),
            ..sample()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn validate_openai_compat_requires_endpoint() {
        let s = LlmProviderSpec {
            kind: LlmKind::OpenAiCompatible,
            endpoint: None,
            ..sample()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_anthropic_accepts_empty_endpoint() {
        // Anthropic has a well-known default.
        assert!(sample().validate().is_ok());
    }

    #[test]
    fn validate_mcp_requires_tool_name() {
        let s = LlmProviderSpec {
            kind: LlmKind::Mcp,
            mcp_tool: None,
            ..sample()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_ollama_and_stub_need_nothing() {
        for kind in [LlmKind::Ollama, LlmKind::Stub, LlmKind::Kurage] {
            let s = LlmProviderSpec {
                kind,
                endpoint: None,
                ..sample()
            };
            assert!(
                s.validate().is_ok(),
                "{kind:?} should validate with minimal form"
            );
        }
    }

    #[test]
    fn temperature_clamps_to_valid_range() {
        assert_eq!(
            LlmProviderSpec {
                temperature: 99.0,
                ..sample()
            }
            .clamped_temperature(),
            2.0
        );
        assert_eq!(
            LlmProviderSpec {
                temperature: -1.0,
                ..sample()
            }
            .clamped_temperature(),
            0.0
        );
    }

    #[test]
    fn registry_insert_validates_and_dedupes() {
        let mut reg = LlmProviderRegistry::new();
        assert!(reg
            .insert(LlmProviderSpec {
                model: String::new(),
                ..sample()
            })
            .is_err());
        reg.insert(sample()).unwrap();
        reg.insert(LlmProviderSpec {
            max_tokens: 8192,
            ..sample()
        })
        .unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].max_tokens, 8192);
    }

    #[test]
    fn echo_provider_concatenates_user_messages() {
        let spec = sample();
        let provider = EchoProvider::default();
        let call = LlmCall {
            messages: vec![
                LlmMessage {
                    role: "system".into(),
                    content: "be helpful".into(),
                },
                LlmMessage {
                    role: "user".into(),
                    content: "hello".into(),
                },
                LlmMessage {
                    role: "user".into(),
                    content: "world".into(),
                },
            ],
            ..Default::default()
        };
        let resp = provider.generate(&spec, &call).unwrap();
        assert!(resp.content.contains("hello"));
        assert!(resp.content.contains("world"));
        assert!(!resp.content.contains("be helpful")); // system role excluded
        assert_eq!(resp.stopped, StopReason::EndTurn);
    }

    #[test]
    fn echo_provider_trace_records_prompts() {
        let provider = EchoProvider {
            trace: Some(Arc::new(Mutex::new(Vec::new()))),
        };
        let spec = sample();
        let call = LlmCall {
            messages: vec![LlmMessage {
                role: "user".into(),
                content: "x".into(),
            }],
            ..Default::default()
        };
        provider.generate(&spec, &call).unwrap();
        provider.generate(&spec, &call).unwrap();
        let captured = provider.trace.as_ref().unwrap().lock().unwrap();
        assert_eq!(captured.len(), 2);
    }

    #[test]
    fn llm_kind_roundtrips_through_serde() {
        for k in [
            LlmKind::OpenAiCompatible,
            LlmKind::Anthropic,
            LlmKind::Gemini,
            LlmKind::Ollama,
            LlmKind::Kurage,
            LlmKind::Mcp,
            LlmKind::Stub,
        ] {
            let s = LlmProviderSpec {
                kind: k,
                ..sample()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: LlmProviderSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.kind, k);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_provider_form() {
        let src = r#"
            (defllm-provider :name "claude"
                             :kind "anthropic"
                             :model "claude-opus-4-7"
                             :max-tokens 4096
                             :temperature 0.7
                             :auth-env "ANTHROPIC_API_KEY")
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.kind, LlmKind::Anthropic);
        assert_eq!(s.model, "claude-opus-4-7");
    }
}
