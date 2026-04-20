//! `(defconsole-rule)` — declarative console output filter / highlight.
//!
//! Absorbs Chrome DevTools console filters + Firefox DevTools sidebar
//! filters + log-level color themes across every browser. Each rule
//! matches a log severity + regex pattern + source host, then either
//! recolors, prefixes, captures to storage, or outright drops the
//! message. Composes with (defstorage) for durable capture and
//! (definspector) for live display.
//!
//! ```lisp
//! (defconsole-rule :name     "error-red"
//!                  :level    :error
//!                  :pattern  ""
//!                  :color    "#ff5555"
//!                  :prefix   "[ERR] ")
//!
//! (defconsole-rule :name     "suppress-gdpr"
//!                  :pattern  "gdpr|cookie policy"
//!                  :action   :drop)
//!
//! (defconsole-rule :name     "capture-analytics"
//!                  :host     "*://*.google-analytics.com/*"
//!                  :action   :capture
//!                  :capture-store "analytics-log")
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Log severity — mirrors `console.log` / `info` / `warn` / `error`
/// plus `debug` and `table` for completeness.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Debug,
    Info,
    Log,
    Warn,
    Error,
    Table,
    /// Any level — useful for pattern-only rules.
    Any,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Any
    }
}

/// What happens when a rule fires.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConsoleAction {
    /// Display as-is (possibly with color/prefix overrides).
    Display,
    /// Suppress the message entirely — never shows in the console.
    Drop,
    /// Display + append a copy to `capture_store`.
    Capture,
    /// Display + emit a `(defstate)` set-state event for dashboards.
    Emit,
}

impl Default for ConsoleAction {
    fn default() -> Self {
        Self::Display
    }
}

/// Console rule.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defconsole-rule"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleRuleSpec {
    pub name: String,
    /// Log severity the rule applies to.
    #[serde(default)]
    pub level: LogLevel,
    /// Regex pattern matched against the full message. Empty =
    /// match any (pairs with level-only rules).
    #[serde(default)]
    pub pattern: String,
    /// Host glob the rule is scoped to. Empty / `"*"` = every host.
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub action: ConsoleAction,
    /// Override foreground color (CSS hex). None = default severity color.
    #[serde(default)]
    pub color: Option<String>,
    /// Override background color.
    #[serde(default)]
    pub background: Option<String>,
    /// Literal string prefixed to the output.
    #[serde(default)]
    pub prefix: Option<String>,
    /// Storage namespace for Capture action.
    #[serde(default)]
    pub capture_store: Option<String>,
    /// State cell for Emit action.
    #[serde(default)]
    pub emit_state: Option<String>,
    /// Ignore case when matching `pattern`.
    #[serde(default = "default_case_insensitive")]
    pub case_insensitive: bool,
    /// Treat `pattern` as a literal substring rather than regex.
    #[serde(default)]
    pub literal: bool,
    /// Disabled rules stay declared but skip dispatch.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_host() -> String {
    "*".into()
}
fn default_enabled() -> bool {
    true
}
fn default_case_insensitive() -> bool {
    true
}

impl ConsoleRuleSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("console rule name is empty".into());
        }
        match self.action {
            ConsoleAction::Capture if self.capture_store.is_none() => {
                return Err(format!(
                    "console rule '{}' :capture action needs :capture-store",
                    self.name
                ));
            }
            ConsoleAction::Emit if self.emit_state.is_none() => {
                return Err(format!(
                    "console rule '{}' :emit action needs :emit-state",
                    self.name
                ));
            }
            _ => {}
        }
        if !self.literal && !self.pattern.is_empty() {
            if let Err(e) = regex::RegexBuilder::new(&self.pattern)
                .case_insensitive(self.case_insensitive)
                .build()
            {
                return Err(format!(
                    "console rule '{}' has invalid regex: {e}",
                    self.name
                ));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn matches_host(&self, host: &str) -> bool {
        crate::extension::host_pattern_matches(&self.host, host)
    }

    #[must_use]
    pub fn matches_level(&self, level: LogLevel) -> bool {
        matches!(self.level, LogLevel::Any) || self.level == level
    }

    /// Test whether the rule fires on `(level, host, message)`.
    #[must_use]
    pub fn fires_on(&self, level: LogLevel, host: &str, message: &str) -> bool {
        if !self.enabled {
            return false;
        }
        if !self.matches_level(level) {
            return false;
        }
        if !self.matches_host(host) {
            return false;
        }
        if self.pattern.is_empty() {
            return true;
        }
        if self.literal {
            if self.case_insensitive {
                return message
                    .to_ascii_lowercase()
                    .contains(&self.pattern.to_ascii_lowercase());
            }
            return message.contains(&self.pattern);
        }
        // Regex path — already validated in validate(); errors fall
        // through to false.
        match regex::RegexBuilder::new(&self.pattern)
            .case_insensitive(self.case_insensitive)
            .build()
        {
            Ok(re) => re.is_match(message),
            Err(_) => false,
        }
    }
}

/// One evaluated dispatch result — what the UI / capture layer acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleOutcome<'a> {
    pub rule: &'a ConsoleRuleSpec,
    pub action: ConsoleAction,
    pub color: Option<&'a str>,
    pub background: Option<&'a str>,
    pub prefix: Option<&'a str>,
    pub capture_store: Option<&'a str>,
    pub emit_state: Option<&'a str>,
}

/// Registry.
#[derive(Debug, Clone, Default)]
pub struct ConsoleRuleRegistry {
    specs: Vec<ConsoleRuleSpec>,
}

impl ConsoleRuleRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: ConsoleRuleSpec) -> Result<(), String> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = ConsoleRuleSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defconsole-rule '{}' rejected: {}", s.name, e);
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
    pub fn specs(&self) -> &[ConsoleRuleSpec] {
        &self.specs
    }

    /// Every rule that fires on `(level, host, message)`. Rules
    /// evaluate in insertion order; callers with priority needs
    /// can look at the Vec ordering directly.
    #[must_use]
    pub fn matching(
        &self,
        level: LogLevel,
        host: &str,
        message: &str,
    ) -> Vec<RuleOutcome<'_>> {
        self.specs
            .iter()
            .filter(|s| s.fires_on(level, host, message))
            .map(|s| RuleOutcome {
                rule: s,
                action: s.action,
                color: s.color.as_deref(),
                background: s.background.as_deref(),
                prefix: s.prefix.as_deref(),
                capture_store: s.capture_store.as_deref(),
                emit_state: s.emit_state.as_deref(),
            })
            .collect()
    }
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<ConsoleRuleSpec>, String> {
    tatara_lisp::compile_typed::<ConsoleRuleSpec>(src)
        .map_err(|e| format!("failed to compile defconsole-rule forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<ConsoleRuleSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, pattern: &str) -> ConsoleRuleSpec {
        ConsoleRuleSpec {
            name: name.into(),
            level: LogLevel::Any,
            pattern: pattern.into(),
            host: "*".into(),
            action: ConsoleAction::Display,
            color: None,
            background: None,
            prefix: None,
            capture_store: None,
            emit_state: None,
            case_insensitive: true,
            literal: false,
            enabled: true,
            description: None,
        }
    }

    #[test]
    fn validate_rejects_empty_name() {
        let s = ConsoleRuleSpec {
            name: String::new(),
            ..sample("x", "")
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_capture_without_store() {
        let s = ConsoleRuleSpec {
            action: ConsoleAction::Capture,
            ..sample("x", "")
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_emit_without_state() {
        let s = ConsoleRuleSpec {
            action: ConsoleAction::Emit,
            ..sample("x", "")
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_invalid_regex() {
        let s = sample("x", "[unclosed");
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_accepts_invalid_regex_when_literal() {
        let s = ConsoleRuleSpec {
            literal: true,
            ..sample("x", "[unclosed")
        };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn fires_on_respects_level_host_and_pattern() {
        let s = ConsoleRuleSpec {
            level: LogLevel::Error,
            pattern: "crash".into(),
            host: "*://*.example.com/*".into(),
            ..sample("x", "")
        };
        assert!(s.fires_on(LogLevel::Error, "shop.example.com", "crash detected"));
        assert!(!s.fires_on(LogLevel::Warn, "shop.example.com", "crash detected"));
        assert!(!s.fires_on(LogLevel::Error, "evil.com", "crash detected"));
        assert!(!s.fires_on(LogLevel::Error, "shop.example.com", "nothing"));
    }

    #[test]
    fn fires_on_any_level_matches_everything() {
        let s = sample("x", "");
        assert!(s.fires_on(LogLevel::Error, "anywhere.com", "hi"));
        assert!(s.fires_on(LogLevel::Info, "anywhere.com", "hi"));
        assert!(s.fires_on(LogLevel::Debug, "anywhere.com", "hi"));
    }

    #[test]
    fn fires_on_case_insensitive_by_default() {
        let s = sample("x", "ERROR");
        assert!(s.fires_on(LogLevel::Any, "*", "something error occurred"));
    }

    #[test]
    fn fires_on_case_sensitive_opt_in() {
        let s = ConsoleRuleSpec {
            case_insensitive: false,
            ..sample("x", "Error")
        };
        assert!(!s.fires_on(LogLevel::Any, "*", "something error occurred"));
        assert!(s.fires_on(LogLevel::Any, "*", "something Error occurred"));
    }

    #[test]
    fn fires_on_literal_substring_mode() {
        let s = ConsoleRuleSpec {
            literal: true,
            ..sample("x", "[err]")
        };
        assert!(s.fires_on(LogLevel::Any, "*", "prefix [err] suffix"));
        assert!(!s.fires_on(LogLevel::Any, "*", "nothing"));
    }

    #[test]
    fn disabled_rule_never_fires() {
        let s = ConsoleRuleSpec {
            enabled: false,
            ..sample("x", "")
        };
        assert!(!s.fires_on(LogLevel::Any, "*", "anything"));
    }

    #[test]
    fn registry_insert_validates_and_dedupes() {
        let mut reg = ConsoleRuleRegistry::new();
        assert!(reg
            .insert(sample("", ""))
            .is_err());
        reg.insert(sample("rule", "")).unwrap();
        reg.insert(ConsoleRuleSpec {
            color: Some("#ff0000".into()),
            ..sample("rule", "")
        })
        .unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].color.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn matching_returns_every_rule_that_fires() {
        let mut reg = ConsoleRuleRegistry::new();
        reg.insert(ConsoleRuleSpec {
            color: Some("#ff5555".into()),
            ..sample("err-red", "")
        })
        .unwrap();
        reg.insert(ConsoleRuleSpec {
            level: LogLevel::Error,
            pattern: "crash".into(),
            action: ConsoleAction::Capture,
            capture_store: Some("crashes".into()),
            ..sample("capture-crash", "")
        })
        .unwrap();
        let hits = reg.matching(LogLevel::Error, "*", "crash detected");
        assert_eq!(hits.len(), 2);
        // Both ordered as inserted.
        assert_eq!(hits[0].rule.name, "err-red");
        assert_eq!(hits[1].action, ConsoleAction::Capture);
    }

    #[test]
    fn level_and_action_roundtrip_through_serde() {
        for level in [
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Log,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Table,
            LogLevel::Any,
        ] {
            let s = ConsoleRuleSpec {
                level,
                ..sample("x", "")
            };
            let json = serde_json::to_string(&s).unwrap();
            let back: ConsoleRuleSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(back.level, level);
        }
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_console_rule_form() {
        let src = r#"
            (defconsole-rule :name "suppress-gdpr"
                             :pattern "gdpr|cookie policy"
                             :action "drop"
                             :case-insensitive #t
                             :enabled #t)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "suppress-gdpr");
        assert_eq!(s.action, ConsoleAction::Drop);
        // Case-insensitive matching — "gdpr" pattern hits "GDPR".
        assert!(s.fires_on(LogLevel::Info, "*", "GDPR compliance notice"));
    }
}
