//! `(defchat-with-page)` — conversational Q&A over page contents.
//!
//! Absorbs Arc Browser's AI chat, Microsoft Copilot-in-Edge, Brave
//! Leo, Firefox AI sidebar. The profile declares the LLM provider,
//! how page context is built (whole DOM, reader-simplified, selection,
//! or RAG chunks), conversation-memory strategy, and storage for
//! chat history.
//!
//! ```lisp
//! (defchat-with-page :name         "default"
//!                    :provider     "claude"
//!                    :context      :reader
//!                    :history      :per-tab
//!                    :storage      "chat-history"
//!                    :max-context-tokens 120000
//!                    :rag-enabled #f)
//!
//! (defchat-with-page :name         "research"
//!                    :provider     "claude"
//!                    :context      :rag
//!                    :history      :per-space
//!                    :rag-enabled  #t
//!                    :rag-chunk-size 512)
//! ```

use crate::llm::{LlmCall, LlmError, LlmMessage, LlmProvider, LlmProviderSpec, LlmResponse};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Strategy for stitching page content into the LLM context.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ContextStrategy {
    /// Entire text_content() of the DOM.
    WholeDom,
    /// `(defreader)` simplified view — highest signal-to-tokens.
    Reader,
    /// User selection / highlight only.
    Selection,
    /// Chunk-and-retrieve via embeddings (RAG). Requires
    /// `rag_enabled`; the retrieval step lives in the host.
    Rag,
    /// No page context — plain chat.
    None,
}

impl Default for ContextStrategy {
    fn default() -> Self {
        Self::Reader
    }
}

/// How conversation memory is scoped.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryScope {
    /// One conversation per browser tab.
    PerTab,
    /// One conversation per declared `(defspace)`.
    PerSpace,
    /// Globally shared across the whole session.
    Global,
    /// No persistence — each question is one-shot.
    Ephemeral,
}

impl Default for HistoryScope {
    fn default() -> Self {
        Self::PerTab
    }
}

/// Chat profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defchat-with-page"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatSpec {
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub context: ContextStrategy,
    #[serde(default)]
    pub history: HistoryScope,
    /// (defstorage) namespace for chat transcripts.
    #[serde(default = "default_storage")]
    pub storage: String,
    /// Total token budget the host will respect when building context.
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: u32,
    /// Keep the last N turns explicitly (separate from the token cap).
    /// `0` = no limit (token cap still applies).
    #[serde(default = "default_keep_last_turns")]
    pub keep_last_turns: u32,
    /// RAG toggle — when true, `context = Rag` is selectable AND the
    /// host is expected to embed + retrieve.
    #[serde(default)]
    pub rag_enabled: bool,
    /// Chunk size for RAG in tokens. Common values 256–1024.
    #[serde(default = "default_rag_chunk_size")]
    pub rag_chunk_size: u32,
    /// System prompt prefix injected before every message.
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_storage() -> String {
    "chat-history".into()
}
fn default_max_context_tokens() -> u32 {
    100_000
}
fn default_keep_last_turns() -> u32 {
    20
}
fn default_rag_chunk_size() -> u32 {
    512
}
fn default_system_prompt() -> String {
    "You are a browser-side assistant. Answer questions about the page the user is viewing. Cite quoted spans where useful.".into()
}

impl ChatSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            provider: "default".into(),
            context: ContextStrategy::Reader,
            history: HistoryScope::PerTab,
            storage: default_storage(),
            max_context_tokens: default_max_context_tokens(),
            keep_last_turns: default_keep_last_turns(),
            rag_enabled: false,
            rag_chunk_size: default_rag_chunk_size(),
            system_prompt: default_system_prompt(),
            description: Some("Default chat-with-page — Reader + per-tab.".into()),
        }
    }

    /// Build an `LlmCall` from a history of prior turns + current
    /// question + optional page context. Caller assembles the
    /// context per the `context` strategy; this just stitches
    /// everything into a messages array.
    #[must_use]
    pub fn build_call(
        &self,
        page_context: Option<&str>,
        history: &[LlmMessage],
        question: &str,
    ) -> LlmCall {
        let mut messages: Vec<LlmMessage> = Vec::new();
        messages.push(LlmMessage {
            role: "system".into(),
            content: self.system_prompt.clone(),
        });
        if let Some(ctx) = page_context {
            if !ctx.is_empty() {
                messages.push(LlmMessage {
                    role: "system".into(),
                    content: format!("Current page context:\n\n{ctx}"),
                });
            }
        }
        // Apply keep_last_turns cap.
        let tail_start = if self.keep_last_turns > 0
            && history.len() > self.keep_last_turns as usize * 2
        {
            history.len() - self.keep_last_turns as usize * 2
        } else {
            0
        };
        messages.extend(history[tail_start..].iter().cloned());
        messages.push(LlmMessage {
            role: "user".into(),
            content: question.to_owned(),
        });
        LlmCall {
            messages,
            temperature: Some(0.4),
            max_tokens: None,
            stop: vec![],
            metadata: Default::default(),
        }
    }

    /// Drive a provider, returning its response. Caller writes the
    /// response back into history.
    pub fn run(
        &self,
        provider: &dyn LlmProvider,
        provider_spec: &LlmProviderSpec,
        page_context: Option<&str>,
        history: &[LlmMessage],
        question: &str,
    ) -> Result<LlmResponse, LlmError> {
        let call = self.build_call(page_context, history, question);
        provider.generate(provider_spec, &call)
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ChatRegistry {
    specs: Vec<ChatSpec>,
}

impl ChatRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ChatSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ChatSpec>) {
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
    pub fn specs(&self) -> &[ChatSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ChatSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ChatSpec>, String> {
    tatara_lisp::compile_typed::<ChatSpec>(src)
        .map_err(|e| format!("failed to compile defchat-with-page forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ChatSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{EchoProvider, LlmKind, LlmMessage, LlmProviderSpec};

    fn provider_spec() -> LlmProviderSpec {
        LlmProviderSpec {
            name: "stub".into(),
            kind: LlmKind::Stub,
            endpoint: None,
            model: "stub-1".into(),
            max_tokens: 1000,
            temperature: 0.7,
            auth_env: None,
            mcp_tool: None,
            rate_limit_per_minute: 0,
            timeout_seconds: 60,
            description: None,
        }
    }

    #[test]
    fn default_profile_is_reader_per_tab() {
        let s = ChatSpec::default_profile();
        assert_eq!(s.context, ContextStrategy::Reader);
        assert_eq!(s.history, HistoryScope::PerTab);
    }

    #[test]
    fn build_call_injects_system_prompt_and_context() {
        let s = ChatSpec::default_profile();
        let call = s.build_call(Some("page body"), &[], "what is this about?");
        assert!(call.messages[0].role == "system");
        assert!(call.messages[0].content.contains("browser-side assistant"));
        assert!(call.messages.iter().any(|m| m.content.contains("page body")));
        assert_eq!(call.messages.last().unwrap().role, "user");
    }

    #[test]
    fn build_call_skips_empty_page_context() {
        let s = ChatSpec::default_profile();
        let call = s.build_call(Some(""), &[], "hi");
        // Only system + user, no page-context wrapper.
        assert_eq!(call.messages.len(), 2);
    }

    #[test]
    fn build_call_honors_keep_last_turns_cap() {
        let s = ChatSpec {
            keep_last_turns: 2,
            ..ChatSpec::default_profile()
        };
        // Feed 10 turns (20 messages).
        let history: Vec<LlmMessage> = (0..20)
            .map(|i| LlmMessage {
                role: if i % 2 == 0 { "user".into() } else { "assistant".into() },
                content: format!("turn {i}"),
            })
            .collect();
        let call = s.build_call(None, &history, "latest");
        // Messages: system (1) + last 4 messages + user. 4 from cap of 2 turns × 2 roles.
        let history_in_call = call
            .messages
            .iter()
            .filter(|m| m.content.starts_with("turn "))
            .count();
        assert_eq!(history_in_call, 4);
    }

    #[test]
    fn keep_last_turns_zero_keeps_everything() {
        let s = ChatSpec {
            keep_last_turns: 0,
            ..ChatSpec::default_profile()
        };
        let history: Vec<LlmMessage> = (0..6)
            .map(|i| LlmMessage {
                role: "user".into(),
                content: format!("t{i}"),
            })
            .collect();
        let call = s.build_call(None, &history, "q");
        // All 6 + system + user.
        assert_eq!(call.messages.len(), 8);
    }

    #[test]
    fn run_through_echo_provider_roundtrips() {
        let s = ChatSpec::default_profile();
        let provider = EchoProvider::default();
        let spec = provider_spec();
        let resp = s
            .run(&provider, &spec, Some("source text"), &[], "what is this?")
            .unwrap();
        assert!(resp.content.contains("what is this?"));
    }

    #[test]
    fn context_strategy_roundtrips_through_serde() {
        for c in [
            ContextStrategy::WholeDom,
            ContextStrategy::Reader,
            ContextStrategy::Selection,
            ContextStrategy::Rag,
            ContextStrategy::None,
        ] {
            let s = ChatSpec {
                context: c,
                ..ChatSpec::default_profile()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ChatSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.context, c);
        }
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = ChatRegistry::new();
        reg.insert(ChatSpec::default_profile());
        reg.insert(ChatSpec {
            rag_enabled: true,
            ..ChatSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert!(reg.specs()[0].rag_enabled);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_chat_form() {
        let src = r#"
            (defchat-with-page :name "research"
                               :provider "claude"
                               :context "rag"
                               :history "per-space"
                               :storage "chat-history"
                               :max-context-tokens 120000
                               :rag-enabled #t
                               :rag-chunk-size 512)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "research");
        assert_eq!(s.context, ContextStrategy::Rag);
        assert_eq!(s.history, HistoryScope::PerSpace);
        assert!(s.rag_enabled);
        assert_eq!(s.rag_chunk_size, 512);
    }
}
