//! `(defsummarize)` — declarative page summarization.
//!
//! Pairs with `(defreader)`: reader simplifies, summarize compresses.
//! Each profile declares the LLM provider, scope of content, style,
//! and length. The trait-based [`crate::llm::LlmProvider`] handles
//! the actual model call.
//!
//! ```lisp
//! (defsummarize :name          "tl-dr"
//!               :provider      "claude"
//!               :scope         :reader-text
//!               :style         :bullets
//!               :max-words     150
//!               :include-code  #f)
//!
//! (defsummarize :name          "long-form"
//!               :provider      "claude"
//!               :scope         :whole-page
//!               :style         :paragraph
//!               :max-words     500
//!               :include-code  #t)
//! ```

use crate::llm::{LlmCall, LlmError, LlmMessage, LlmProvider, LlmProviderSpec, LlmResponse};
use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Source content for summarization.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SummarizeScope {
    /// Whole-page HTML stripped to text.
    WholePage,
    /// `(defreader)` simplified output — typically better signal.
    ReaderText,
    /// User-highlighted selection only.
    Selection,
}

impl Default for SummarizeScope {
    fn default() -> Self {
        Self::ReaderText
    }
}

/// Output style.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SummarizeStyle {
    /// One-paragraph dense summary.
    Paragraph,
    /// Bullet-point list.
    Bullets,
    /// Single-sentence TL;DR.
    Sentence,
    /// Key-point + supporting detail tree.
    Outline,
    /// Question-and-answer format for study.
    QnA,
}

impl Default for SummarizeStyle {
    fn default() -> Self {
        Self::Bullets
    }
}

/// Summarization profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defsummarize"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummarizeSpec {
    pub name: String,
    /// `(defllm-provider)` name.
    pub provider: String,
    #[serde(default)]
    pub scope: SummarizeScope,
    #[serde(default)]
    pub style: SummarizeStyle,
    /// Upper bound on output length. Enforced at post-hoc clamp time
    /// since most models honor it only loosely.
    #[serde(default = "default_max_words")]
    pub max_words: u32,
    /// Preserve fenced code blocks verbatim in the summary.
    #[serde(default = "default_include_code")]
    pub include_code: bool,
    /// Language of the summary. Defaults to the source language
    /// when the provider can detect it; otherwise falls back to
    /// English. Overrides: `"en"`, `"ja"`, `"pt-BR"`, etc.
    #[serde(default)]
    pub language: Option<String>,
    /// Optional custom instructions appended to the system prompt.
    #[serde(default)]
    pub extra_instructions: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_max_words() -> u32 {
    200
}
fn default_include_code() -> bool {
    false
}

impl SummarizeSpec {
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            provider: "default".into(),
            scope: SummarizeScope::ReaderText,
            style: SummarizeStyle::Bullets,
            max_words: 200,
            include_code: false,
            language: None,
            extra_instructions: None,
            description: Some("Default TL;DR — bullets, 200 words, reader scope.".into()),
        }
    }

    /// Build the system prompt based on the profile's style + knobs.
    #[must_use]
    pub fn system_prompt(&self) -> String {
        let style_hint = match self.style {
            SummarizeStyle::Paragraph => "Respond with one dense paragraph.",
            SummarizeStyle::Bullets => "Respond with a bullet list.",
            SummarizeStyle::Sentence => "Respond with exactly one sentence.",
            SummarizeStyle::Outline => "Respond with a hierarchical outline.",
            SummarizeStyle::QnA => "Respond as Q&A pairs useful for study.",
        };
        let code_hint = if self.include_code {
            "Preserve fenced code blocks verbatim if present."
        } else {
            "Omit code blocks; describe them in prose if relevant."
        };
        let lang_hint = self
            .language
            .as_deref()
            .map(|l| format!(" Respond in {l}."))
            .unwrap_or_default();
        let extra = self
            .extra_instructions
            .as_deref()
            .map(|e| format!("\n\n{e}"))
            .unwrap_or_default();
        format!(
            "You are a precise summarizer. Keep the output under {} words. {style_hint} {code_hint}{lang_hint}{extra}",
            self.max_words
        )
    }

    /// Build the `LlmCall` for a given source text.
    #[must_use]
    pub fn build_call(&self, source: &str) -> LlmCall {
        LlmCall {
            messages: vec![
                LlmMessage {
                    role: "system".into(),
                    content: self.system_prompt(),
                },
                LlmMessage {
                    role: "user".into(),
                    content: format!("Summarize the following:\n\n{source}"),
                },
            ],
            temperature: Some(0.3),
            max_tokens: Some(self.max_words * 6),
            stop: vec![],
            metadata: Default::default(),
        }
    }

    /// Drive a provider to produce a summary.
    pub fn run(
        &self,
        provider: &dyn LlmProvider,
        provider_spec: &LlmProviderSpec,
        source: &str,
    ) -> Result<LlmResponse, LlmError> {
        let call = self.build_call(source);
        provider.generate(provider_spec, &call)
    }
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct SummarizeRegistry {
    specs: Vec<SummarizeSpec>,
}

impl SummarizeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: SummarizeSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = SummarizeSpec>) {
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
    pub fn specs(&self) -> &[SummarizeSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SummarizeSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<SummarizeSpec>, String> {
    tatara_lisp::compile_typed::<SummarizeSpec>(src)
        .map_err(|e| format!("failed to compile defsummarize forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<SummarizeSpec>();
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
    fn default_profile_targets_reader_text_bullets() {
        let s = SummarizeSpec::default_profile();
        assert_eq!(s.scope, SummarizeScope::ReaderText);
        assert_eq!(s.style, SummarizeStyle::Bullets);
        assert_eq!(s.max_words, 200);
    }

    #[test]
    fn system_prompt_reflects_style() {
        for (style, token) in [
            (SummarizeStyle::Paragraph, "paragraph"),
            (SummarizeStyle::Bullets, "bullet"),
            (SummarizeStyle::Sentence, "one sentence"),
            (SummarizeStyle::Outline, "outline"),
            (SummarizeStyle::QnA, "Q&A"),
        ] {
            let s = SummarizeSpec {
                style,
                ..SummarizeSpec::default_profile()
            };
            let prompt = s.system_prompt();
            assert!(
                prompt.to_lowercase().contains(&token.to_lowercase()),
                "style {style:?} should mention {token:?} in prompt: {prompt}"
            );
        }
    }

    #[test]
    fn system_prompt_mentions_max_words() {
        let s = SummarizeSpec {
            max_words: 73,
            ..SummarizeSpec::default_profile()
        };
        assert!(s.system_prompt().contains("73"));
    }

    #[test]
    fn system_prompt_honors_language() {
        let s = SummarizeSpec {
            language: Some("ja".into()),
            ..SummarizeSpec::default_profile()
        };
        assert!(s.system_prompt().contains("ja"));
    }

    #[test]
    fn system_prompt_appends_extra_instructions() {
        let s = SummarizeSpec {
            extra_instructions: Some("Avoid jargon.".into()),
            ..SummarizeSpec::default_profile()
        };
        assert!(s.system_prompt().contains("Avoid jargon"));
    }

    #[test]
    fn code_hint_flips_with_include_code() {
        let without = SummarizeSpec::default_profile().system_prompt();
        assert!(without.contains("Omit"));
        let with_code = SummarizeSpec {
            include_code: true,
            ..SummarizeSpec::default_profile()
        };
        let p = with_code.system_prompt();
        assert!(p.contains("Preserve"));
    }

    #[test]
    fn build_call_wraps_source_text() {
        let s = SummarizeSpec::default_profile();
        let call = s.build_call("Article body here.");
        assert_eq!(call.messages.len(), 2);
        assert_eq!(call.messages[0].role, "system");
        assert_eq!(call.messages[1].role, "user");
        assert!(call.messages[1].content.contains("Article body here"));
        assert_eq!(call.temperature, Some(0.3));
    }

    #[test]
    fn run_through_echo_provider_roundtrips() {
        let s = SummarizeSpec::default_profile();
        let provider = EchoProvider::default();
        let spec = provider_spec();
        let resp = s.run(&provider, &spec, "hello world").unwrap();
        assert!(resp.content.contains("hello world"));
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = SummarizeRegistry::new();
        reg.insert(SummarizeSpec::default_profile());
        reg.insert(SummarizeSpec {
            max_words: 50,
            ..SummarizeSpec::default_profile()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].max_words, 50);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_summarize_form() {
        let src = r#"
            (defsummarize :name      "tl-dr"
                          :provider  "claude"
                          :scope     "reader-text"
                          :style     "bullets"
                          :max-words 150
                          :include-code #f)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "tl-dr");
        assert_eq!(s.scope, SummarizeScope::ReaderText);
        assert_eq!(s.style, SummarizeStyle::Bullets);
        assert_eq!(s.max_words, 150);
    }
}
