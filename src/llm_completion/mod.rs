//! `(defllm-completion)` — LLM-backed inline completion.
//!
//! Absorbs Arc's AI URL-bar completion, Edge's Copilot-in-compose
//! suggestions, and Gmail's Smart Compose. Each profile declares
//! *where* completions fire (URL bar / form inputs / contenteditable),
//! which LLM provider to use, how aggressively to prefill, and which
//! hosts are in-scope.
//!
//! ```lisp
//! (defllm-completion :name         "omnibox"
//!                    :provider     "claude"
//!                    :trigger      :url-bar
//!                    :min-chars    3
//!                    :debounce-ms  200
//!                    :max-suggestions 3
//!                    :temperature  0.4)
//!
//! (defllm-completion :name         "compose"
//!                    :provider     "claude"
//!                    :trigger      :contenteditable
//!                    :host-gated   ("*://mail.*/*" "*://*.notion.so/*")
//!                    :min-chars    20
//!                    :max-tokens   60
//!                    :temperature  0.2)
//! ```

use crate::llm::{LlmCall, LlmError, LlmMessage, LlmProvider, LlmProviderSpec, LlmResponse};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Where to fire completions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CompletionTrigger {
    /// Inside the URL bar / omnibox — prefill URLs or queries.
    UrlBar,
    /// `<input type=text>` / `<textarea>`.
    FormInput,
    /// `contenteditable` DOM — rich-text composers (Notion, Gmail).
    Contenteditable,
    /// Inline-code buffers (Monaco, CodeMirror, ProseMirror code blocks).
    CodeBuffer,
}

impl Default for CompletionTrigger {
    fn default() -> Self {
        Self::FormInput
    }
}

/// Completion profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defllm-completion"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LlmCompletionSpec {
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub trigger: CompletionTrigger,
    /// Minimum typed characters before the profile fires. Reduces
    /// provider traffic and false starts.
    #[serde(default = "default_min_chars")]
    pub min_chars: u32,
    /// Debounce delay. Fresh keystrokes reset the timer.
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u32,
    /// Max suggestions to surface. UrlBar + FormInput typically
    /// want 1-5; Contenteditable wants 1 inline ghost.
    #[serde(default = "default_max_suggestions")]
    pub max_suggestions: u32,
    /// Max tokens in a single completion.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Override provider temperature (lower = more deterministic).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Host globs this profile fires on. Empty = all hosts.
    #[serde(default)]
    pub host_gated: Vec<String>,
    /// Explicit opt-out hosts (wins over host_gated).
    #[serde(default)]
    pub blocked_hosts: Vec<String>,
    /// System prompt shaping the completion style.
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_min_chars() -> u32 {
    3
}
fn default_debounce_ms() -> u32 {
    200
}
fn default_max_suggestions() -> u32 {
    1
}
fn default_max_tokens() -> u32 {
    60
}
fn default_temperature() -> f32 {
    0.3
}
fn default_enabled() -> bool {
    true
}
fn default_system_prompt() -> String {
    "Continue the user's text naturally. Return only the continuation, no commentary, no quotes."
        .into()
}

impl LlmCompletionSpec {
    #[must_use]
    pub fn default_url_bar() -> Self {
        Self {
            name: "omnibox".into(),
            provider: "default".into(),
            trigger: CompletionTrigger::UrlBar,
            min_chars: 3,
            debounce_ms: 200,
            max_suggestions: 3,
            max_tokens: 40,
            temperature: 0.4,
            host_gated: vec![],
            blocked_hosts: vec![],
            system_prompt:
                "Suggest URLs or web search queries matching the user's prefix. Return one per line, no numbering.".into(),
            enabled: true,
            description: Some("URL-bar LLM completion.".into()),
        }
    }

    #[must_use]
    pub fn default_compose() -> Self {
        Self {
            name: "compose".into(),
            provider: "default".into(),
            trigger: CompletionTrigger::Contenteditable,
            min_chars: 20,
            debounce_ms: 400,
            max_suggestions: 1,
            max_tokens: 80,
            temperature: 0.2,
            host_gated: vec![],
            blocked_hosts: vec![],
            system_prompt: default_system_prompt(),
            enabled: true,
            description: Some("Gmail-Smart-Compose-style inline completion.".into()),
        }
    }

    #[must_use]
    pub fn clamped_temperature(&self) -> f32 {
        self.temperature.clamp(0.0, 2.0)
    }

    /// Whether this profile applies to a trigger source + host pair.
    #[must_use]
    pub fn fires_on(&self, trigger: CompletionTrigger, host: &str) -> bool {
        if !self.enabled || self.trigger != trigger {
            return false;
        }
        if self
            .blocked_hosts
            .iter()
            .any(|g| crate::extension::glob_match_host(g, host))
        {
            return false;
        }
        if self.host_gated.is_empty() {
            return true;
        }
        self.host_gated
            .iter()
            .any(|g| crate::extension::glob_match_host(g, host))
    }

    /// Does the profile trigger on `typed.chars().count() >= min_chars`?
    #[must_use]
    pub fn meets_minimum(&self, typed: &str) -> bool {
        typed.chars().count() >= self.min_chars as usize
    }

    /// Build the LlmCall for a given prefix.
    #[must_use]
    pub fn build_call(&self, prefix: &str) -> LlmCall {
        LlmCall {
            messages: vec![
                LlmMessage {
                    role: "system".into(),
                    content: self.system_prompt.clone(),
                },
                LlmMessage {
                    role: "user".into(),
                    content: prefix.to_owned(),
                },
            ],
            temperature: Some(self.clamped_temperature()),
            max_tokens: Some(self.max_tokens),
            stop: vec!["\n\n".into()],
            metadata: Default::default(),
        }
    }

    pub fn run(
        &self,
        provider: &dyn LlmProvider,
        provider_spec: &LlmProviderSpec,
        prefix: &str,
    ) -> Result<LlmResponse, LlmError> {
        let call = self.build_call(prefix);
        provider.generate(provider_spec, &call)
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct LlmCompletionRegistry {
    specs: Vec<LlmCompletionSpec>,
}

impl LlmCompletionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: LlmCompletionSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = LlmCompletionSpec>) {
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
    pub fn specs(&self) -> &[LlmCompletionSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&LlmCompletionSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// First profile that fires for a (trigger, host) pair.
    #[must_use]
    pub fn resolve(
        &self,
        trigger: CompletionTrigger,
        host: &str,
    ) -> Option<&LlmCompletionSpec> {
        self.specs.iter().find(|s| s.fires_on(trigger, host))
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<LlmCompletionSpec>, String> {
    tatara_lisp::compile_typed::<LlmCompletionSpec>(src)
        .map_err(|e| format!("failed to compile defllm-completion forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<LlmCompletionSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{EchoProvider, LlmKind, LlmProviderSpec};

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
    fn default_url_bar_and_compose_differ_on_trigger() {
        let a = LlmCompletionSpec::default_url_bar();
        let b = LlmCompletionSpec::default_compose();
        assert_eq!(a.trigger, CompletionTrigger::UrlBar);
        assert_eq!(b.trigger, CompletionTrigger::Contenteditable);
    }

    #[test]
    fn fires_on_respects_trigger_enabled_host_gate_and_blocklist() {
        let s = LlmCompletionSpec {
            host_gated: vec!["*://mail.*/*".into()],
            blocked_hosts: vec!["*://mail.evil.com/*".into()],
            ..LlmCompletionSpec::default_compose()
        };
        assert!(s.fires_on(CompletionTrigger::Contenteditable, "mail.google.com"));
        assert!(!s.fires_on(CompletionTrigger::Contenteditable, "other.com"));
        assert!(!s.fires_on(CompletionTrigger::UrlBar, "mail.google.com"));
        assert!(!s.fires_on(CompletionTrigger::Contenteditable, "mail.evil.com"));

        let disabled = LlmCompletionSpec {
            enabled: false,
            ..LlmCompletionSpec::default_compose()
        };
        assert!(!disabled.fires_on(CompletionTrigger::Contenteditable, "any.com"));
    }

    #[test]
    fn meets_minimum_uses_char_count() {
        let s = LlmCompletionSpec {
            min_chars: 5,
            ..LlmCompletionSpec::default_url_bar()
        };
        assert!(!s.meets_minimum("abcd"));
        assert!(s.meets_minimum("abcde"));
        // Multi-byte chars.
        assert!(s.meets_minimum("αβγδε"));
    }

    #[test]
    fn clamped_temperature_respects_bounds() {
        assert_eq!(
            LlmCompletionSpec {
                temperature: 99.0,
                ..LlmCompletionSpec::default_url_bar()
            }
            .clamped_temperature(),
            2.0
        );
        assert_eq!(
            LlmCompletionSpec {
                temperature: -1.0,
                ..LlmCompletionSpec::default_url_bar()
            }
            .clamped_temperature(),
            0.0
        );
    }

    #[test]
    fn build_call_wraps_prefix_with_system_prompt() {
        let s = LlmCompletionSpec::default_url_bar();
        let call = s.build_call("rust async");
        assert_eq!(call.messages.len(), 2);
        assert_eq!(call.messages[0].role, "system");
        assert_eq!(call.messages[1].content, "rust async");
        assert_eq!(call.max_tokens, Some(40));
        assert!(call.stop.iter().any(|s| s == "\n\n"));
    }

    #[test]
    fn run_through_echo_provider() {
        let s = LlmCompletionSpec::default_compose();
        let provider = EchoProvider::default();
        let pspec = provider_spec();
        let resp = s.run(&provider, &pspec, "Dear Jane,").unwrap();
        assert!(resp.content.contains("Dear Jane"));
    }

    #[test]
    fn registry_resolve_first_matching_profile() {
        let mut reg = LlmCompletionRegistry::new();
        reg.insert(LlmCompletionSpec::default_url_bar());
        reg.insert(LlmCompletionSpec::default_compose());
        let hit = reg
            .resolve(CompletionTrigger::UrlBar, "example.com")
            .unwrap();
        assert_eq!(hit.trigger, CompletionTrigger::UrlBar);
    }

    #[test]
    fn resolve_returns_none_when_nothing_fires() {
        let mut reg = LlmCompletionRegistry::new();
        reg.insert(LlmCompletionSpec {
            host_gated: vec!["*://mail.*/*".into()],
            ..LlmCompletionSpec::default_compose()
        });
        assert!(reg
            .resolve(CompletionTrigger::Contenteditable, "other.com")
            .is_none());
    }

    #[test]
    fn trigger_roundtrips_through_serde() {
        for t in [
            CompletionTrigger::UrlBar,
            CompletionTrigger::FormInput,
            CompletionTrigger::Contenteditable,
            CompletionTrigger::CodeBuffer,
        ] {
            let s = LlmCompletionSpec {
                trigger: t,
                ..LlmCompletionSpec::default_url_bar()
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: LlmCompletionSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.trigger, t);
        }
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = LlmCompletionRegistry::new();
        reg.insert(LlmCompletionSpec::default_url_bar());
        reg.insert(LlmCompletionSpec {
            min_chars: 10,
            ..LlmCompletionSpec::default_url_bar()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].min_chars, 10);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_llm_completion_form() {
        let src = r#"
            (defllm-completion :name "compose"
                               :provider "claude"
                               :trigger "contenteditable"
                               :min-chars 20
                               :debounce-ms 400
                               :max-suggestions 1
                               :max-tokens 80
                               :temperature 0.2
                               :host-gated ("*://mail.*/*"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.trigger, CompletionTrigger::Contenteditable);
        assert_eq!(s.min_chars, 20);
        assert_eq!(s.max_tokens, 80);
    }
}
