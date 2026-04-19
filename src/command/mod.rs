//! `(defcommand)` + `(defbind)` — declarative commands and keybindings.
//!
//! Absorbs Vivaldi command-chains, Arc toolbar-shortcut authoring,
//! Chrome `commands` API, and Firefox `browser.commands` into the
//! substrate pattern. A command is a named unit of behavior; a binding
//! ties a key chord to a command (optionally gated by host or state).
//!
//! ```lisp
//! (defcommand :name "toggle-reader"
//!             :description "Flip the current tab into reader mode."
//!             :action "reader:toggle")
//!
//! (defcommand :name "capture-as-note"
//!             :description "Save the page's reader view to my notes store."
//!             :body "(set-state \"last-capture\" (reader-text))")
//!
//! (defbind :key "Cmd+Shift+R"  :command "toggle-reader")
//! (defbind :key "Cmd+Shift+N"  :command "capture-as-note"
//!          :when "(not (secret-host))")
//! ```
//!
//! **Action vs body:** `:action` selects a built-in verb (fast, no
//! Lisp eval needed — `navigate:URL`, `reload`, `reader:toggle`,
//! `extensions:toggle:<name>`, etc.). `:body` holds a tatara-lisp
//! expression run via tatara-eval when the eval feature is on. A
//! command may specify one or the other; both is an error at compile.
//!
//! **When-gates:** `:when` on a binding names a `(defpredicate)` or
//! a host-glob pattern — the binding only fires when the predicate
//! evaluates true (or host matches). Missing `:when` = "always".

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One command — a named unit of behavior invokable by key, menu,
/// MCP tool, or HTTP POST.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defcommand"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Built-in verb to invoke — interpretation is substrate-local.
    /// Examples: `reload`, `reader:toggle`, `navigate:https://…`,
    /// `extensions:enable:<name>`, `storage:clear:<store>`.
    /// Mutually exclusive with `body`.
    #[serde(default)]
    pub action: Option<String>,
    /// Raw tatara-lisp body evaluated when the command fires. Requires
    /// the `eval` feature to execute; with `eval` off it's a no-op
    /// with a warning. Mutually exclusive with `action`. Authored as
    /// `:body "(…)"` in Lisp (plain keyword — `:do` is reserved).
    #[serde(default)]
    pub body: Option<String>,
    /// Optional default key chord — surfaced in the command palette
    /// as the canonical shortcut even if no `(defbind)` exists yet.
    #[serde(default)]
    pub default_key: Option<String>,
}

/// One key binding. Supports multi-key Vim-style sequences and
/// modal dispatch so a single registry can carry normal/insert/
/// visual bindings side-by-side.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defbind"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BindSpec {
    /// Key chord or Vim-style sequence. Single chord:
    /// `"Cmd+Shift+R"`, `"Alt+F4"`, `"F5"`. Multi-key sequence:
    /// space-separated chords — `"g g"`, `"d d"`, `"leader f p"`.
    /// Modifiers are case-insensitive and aliased
    /// (cmd/command/super, ctrl/control, alt/opt/option).
    pub key: String,
    /// Name of the command to invoke. Must match a `(defcommand :name …)`.
    pub command: String,
    /// Modal scope — `"normal"`, `"insert"`, `"visual"`, `"command"`,
    /// or `"any"` (default). Multiple modes: comma-separated
    /// (`"normal,visual"`). Modes are strings, not enums, so Lisp
    /// authors can introduce their own (e.g. `"leader"`).
    #[serde(default)]
    pub mode: Option<String>,
    /// Optional gate — predicate name, host glob, or bare `always`.
    /// `(when "github-host")` → only fires when the host matches the
    /// named pattern. Absent → always fires.
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Structural validation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl CommandSpec {
    /// Exactly one of `action` / `body` must be set; `name` must be
    /// non-empty.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError("command name is empty".into()));
        }
        match (self.action.is_some(), self.body.is_some()) {
            (true, true) => Err(ValidationError(format!(
                "command '{}' declares both :action and :do",
                self.name
            ))),
            (false, false) => Err(ValidationError(format!(
                "command '{}' declares neither :action nor :do",
                self.name
            ))),
            _ => Ok(()),
        }
    }

    /// True if the command uses a built-in verb (vs Lisp body).
    #[must_use]
    pub fn is_action(&self) -> bool {
        self.action.is_some()
    }
}

impl BindSpec {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.key.trim().is_empty() {
            return Err(ValidationError("binding has empty :key".into()));
        }
        if self.command.trim().is_empty() {
            return Err(ValidationError(format!(
                "binding '{}' has empty :command",
                self.key
            )));
        }
        // Canonical key representation — normalize + structural check.
        let canon = canonicalize_sequence(&self.key);
        if canon.is_empty() {
            return Err(ValidationError(format!(
                "binding key '{}' did not tokenize",
                self.key
            )));
        }
        Ok(())
    }

    /// Lowercase, modifiers alphabetized, chords space-separated.
    /// Used for O(1) lookup + de-dupe.
    #[must_use]
    pub fn canonical_key(&self) -> String {
        canonicalize_sequence(&self.key)
    }

    /// Parsed mode list. Empty → `["any"]`.
    #[must_use]
    pub fn modes(&self) -> Vec<String> {
        match self.mode.as_deref() {
            None | Some("") => vec!["any".to_owned()],
            Some(s) => s
                .split(',')
                .map(|m| m.trim().to_ascii_lowercase())
                .filter(|m| !m.is_empty())
                .collect(),
        }
    }

    /// Does this binding fire in `mode`? Bindings with no mode (→ "any")
    /// fire in every mode.
    #[must_use]
    pub fn matches_mode(&self, mode: &str) -> bool {
        let mode = mode.to_ascii_lowercase();
        self.modes()
            .iter()
            .any(|m| m == "any" || m == &mode)
    }
}

/// Canonicalize a full sequence — space-separated list of chords.
/// Each chord is individually canonicalized, then re-joined with
/// single spaces. `"G G"` → `"g g"`, `"  Cmd+R "` → `"cmd+r"`.
#[must_use]
pub fn canonicalize_sequence(input: &str) -> String {
    let chords: Vec<String> = input
        .split_whitespace()
        .map(canonicalize_key)
        .filter(|c| !c.is_empty())
        .collect();
    chords.join(" ")
}

/// Registry of commands, keyed by name.
#[derive(Debug, Clone, Default)]
pub struct CommandRegistry {
    specs: Vec<CommandSpec>,
}

impl CommandRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: CommandSpec) -> Result<(), ValidationError> {
        spec.validate()?;
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
        Ok(())
    }

    /// Best-effort insert — invalid specs are dropped with a
    /// warning (mirrors how blocker/normalize handle bad rules).
    pub fn extend(&mut self, specs: impl IntoIterator<Item = CommandSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defcommand '{}' rejected: {}", s.name, e.0);
            }
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CommandSpec> {
        self.specs.iter().find(|s| s.name == name)
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
    pub fn specs(&self) -> &[CommandSpec] {
        &self.specs
    }

    /// Every command name in insertion order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.specs.iter().map(|s| s.name.clone()).collect()
    }
}

/// Registry of key bindings, indexed by canonical chord string for
/// O(1) lookup during key-event dispatch.
#[derive(Debug, Clone, Default)]
pub struct BindRegistry {
    specs: Vec<BindSpec>,
}

impl BindRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: BindSpec) -> Result<(), ValidationError> {
        spec.validate()?;
        let canon = spec.canonical_key();
        let modes = spec.modes();
        // Two bindings collide only if they share both the canonical
        // sequence AND overlap on any mode — so `g g` in `normal` can
        // coexist with `g g` in `visual` side-by-side.
        self.specs.retain(|s| {
            let same_chord = canonicalize_sequence(&s.key) == canon;
            let existing_modes = s.modes();
            let mode_overlap = existing_modes
                .iter()
                .any(|em| modes.iter().any(|nm| nm == em || em == "any" || nm == "any"));
            !(same_chord && mode_overlap)
        });
        self.specs.push(spec);
        Ok(())
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = BindSpec>) {
        for s in specs {
            if let Err(e) = self.insert(s.clone()) {
                tracing::warn!("defbind '{}' rejected: {}", s.key, e.0);
            }
        }
    }

    /// Look up the binding for a chord or sequence in `mode`. The
    /// lookup normalizes the input. `mode="any"` matches bindings in
    /// any mode; a specific mode only matches bindings scoped to it
    /// or to `any`.
    #[must_use]
    pub fn resolve(&self, input: &str, mode: &str) -> Option<&BindSpec> {
        let canon = canonicalize_sequence(input);
        self.specs.iter().find(|s| {
            canonicalize_sequence(&s.key) == canon && s.matches_mode(mode)
        })
    }

    /// `Match` state for a typed-so-far sequence — either resolved
    /// (complete match), prefix (waiting for more keys), or miss.
    #[must_use]
    pub fn match_sequence(&self, typed: &str, mode: &str) -> SequenceMatch<'_> {
        let canon = canonicalize_sequence(typed);
        if canon.is_empty() {
            return SequenceMatch::Miss;
        }
        // Exact hit wins outright.
        for s in &self.specs {
            if canonicalize_sequence(&s.key) == canon && s.matches_mode(mode) {
                return SequenceMatch::Complete(s);
            }
        }
        // Otherwise check for prefixing — a longer spec that starts
        // with our typed-so-far sequence. Use space boundaries to
        // avoid false positives like `g` prefixing `gg`'s first char.
        let prefix = format!("{canon} ");
        if self.specs.iter().any(|s| {
            canonicalize_sequence(&s.key).starts_with(&prefix) && s.matches_mode(mode)
        }) {
            return SequenceMatch::Prefix;
        }
        SequenceMatch::Miss
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
    pub fn specs(&self) -> &[BindSpec] {
        &self.specs
    }

    /// Chord strings in canonical form.
    #[must_use]
    pub fn chords(&self) -> Vec<String> {
        self.specs
            .iter()
            .map(|s| canonicalize_sequence(&s.key))
            .collect()
    }
}

/// Result of matching a typed-so-far sequence against the bind registry.
#[derive(Debug, PartialEq)]
pub enum SequenceMatch<'a> {
    /// Exact hit — dispatch the command.
    Complete(&'a BindSpec),
    /// Valid prefix — more keys expected.
    Prefix,
    /// Not a prefix, not a match — cancel the sequence.
    Miss,
}

/// Normalize a chord string: lowercase, alphabetize modifiers, use
/// canonical tokens (`cmd` → `cmd`, `command` → `cmd`, `super` → `cmd`;
/// `opt`/`option` → `alt`; `ctrl`/`control` → `ctrl`; `shift` → `shift`).
/// Returns empty string if the chord has no non-modifier key.
#[must_use]
pub fn canonicalize_key(input: &str) -> String {
    let mut mods: Vec<&'static str> = Vec::new();
    let mut main: Option<String> = None;

    for raw in input.split('+') {
        let t = raw.trim().to_ascii_lowercase();
        if t.is_empty() {
            continue;
        }
        match t.as_str() {
            "cmd" | "command" | "super" | "meta" => {
                if !mods.contains(&"cmd") {
                    mods.push("cmd");
                }
            }
            "ctrl" | "control" => {
                if !mods.contains(&"ctrl") {
                    mods.push("ctrl");
                }
            }
            "alt" | "opt" | "option" => {
                if !mods.contains(&"alt") {
                    mods.push("alt");
                }
            }
            "shift" => {
                if !mods.contains(&"shift") {
                    mods.push("shift");
                }
            }
            _ => {
                main = Some(t);
            }
        }
    }

    let Some(main) = main else {
        return String::new();
    };
    mods.sort_unstable();
    let mut out = String::new();
    for m in mods {
        out.push_str(m);
        out.push('+');
    }
    out.push_str(&main);
    out
}

#[cfg(feature = "lisp")]
pub fn compile_commands(src: &str) -> Result<Vec<CommandSpec>, String> {
    tatara_lisp::compile_typed::<CommandSpec>(src)
        .map_err(|e| format!("failed to compile defcommand forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn compile_binds(src: &str) -> Result<Vec<BindSpec>, String> {
    tatara_lisp::compile_typed::<BindSpec>(src)
        .map_err(|e| format!("failed to compile defbind forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<CommandSpec>();
    tatara_lisp::domain::register::<BindSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(name: &str, action: Option<&str>, body: Option<&str>) -> CommandSpec {
        CommandSpec {
            name: name.into(),
            description: None,
            action: action.map(str::to_owned),
            body: body.map(str::to_owned),
            default_key: None,
        }
    }

    fn bind(key: &str, command: &str) -> BindSpec {
        BindSpec {
            key: key.into(),
            command: command.into(),
            mode: None,
            when: None,
            description: None,
        }
    }

    fn bind_mode(key: &str, command: &str, mode: &str) -> BindSpec {
        BindSpec {
            key: key.into(),
            command: command.into(),
            mode: Some(mode.into()),
            when: None,
            description: None,
        }
    }

    #[test]
    fn canonicalize_sorts_modifiers() {
        assert_eq!(canonicalize_key("Shift+Cmd+R"), "cmd+shift+r");
        assert_eq!(canonicalize_key("Alt+Ctrl+Shift+Cmd+K"), "alt+cmd+ctrl+shift+k");
        assert_eq!(canonicalize_key("Ctrl+/"), "ctrl+/");
    }

    #[test]
    fn canonicalize_aliases_modifier_names() {
        assert_eq!(canonicalize_key("Command+X"), "cmd+x");
        assert_eq!(canonicalize_key("Option+X"), "alt+x");
        assert_eq!(canonicalize_key("Super+X"), "cmd+x");
        assert_eq!(canonicalize_key("Control+X"), "ctrl+x");
    }

    #[test]
    fn canonicalize_bare_key_has_no_modifiers() {
        assert_eq!(canonicalize_key("F5"), "f5");
        assert_eq!(canonicalize_key("Escape"), "escape");
    }

    #[test]
    fn canonicalize_rejects_modifier_only_chord() {
        assert_eq!(canonicalize_key("Cmd+Shift"), "");
        assert_eq!(canonicalize_key(""), "");
    }

    #[test]
    fn command_validate_requires_exactly_one_body() {
        assert!(cmd("a", Some("reload"), None).validate().is_ok());
        assert!(cmd("a", None, Some("(echo 1)")).validate().is_ok());
        // Both set → error.
        assert!(cmd("a", Some("reload"), Some("(echo 1)"))
            .validate()
            .is_err());
        // Neither set → error.
        assert!(cmd("a", None, None).validate().is_err());
    }

    #[test]
    fn command_validate_rejects_empty_name() {
        let c = cmd("", Some("reload"), None);
        assert!(c.validate().is_err());
    }

    #[test]
    fn bind_validate_requires_key_and_command() {
        assert!(bind("Cmd+R", "reload").validate().is_ok());
        assert!(bind("", "reload").validate().is_err());
        assert!(bind("Cmd+R", "").validate().is_err());
    }

    #[test]
    fn bind_validate_rejects_modifier_only_key() {
        assert!(bind("Cmd+Shift", "x").validate().is_err());
    }

    #[test]
    fn command_registry_dedupes_by_name() {
        let mut reg = CommandRegistry::new();
        reg.insert(cmd("x", Some("reload"), None)).unwrap();
        reg.insert(cmd("x", Some("navigate:https://example.com"), None))
            .unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(
            reg.get("x").unwrap().action.as_deref(),
            Some("navigate:https://example.com")
        );
    }

    #[test]
    fn bind_registry_dedupes_by_canonical_chord() {
        let mut reg = BindRegistry::new();
        reg.insert(bind("Cmd+R", "a")).unwrap();
        // Equivalent chord — different casing and modifier ordering.
        reg.insert(bind("R+CMD", "b")).unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.resolve("cmd+r", "any").unwrap().command, "b");
    }

    #[test]
    fn bind_resolve_ignores_input_normalization() {
        let mut reg = BindRegistry::new();
        reg.insert(bind("Cmd+Shift+R", "reload")).unwrap();
        assert_eq!(
            reg.resolve("shift+cmd+r", "any").unwrap().command,
            "reload"
        );
        assert_eq!(reg.resolve("Cmd+Shift+R", "any").unwrap().command, "reload");
        assert!(reg.resolve("Cmd+R", "any").is_none());
    }

    #[test]
    fn bind_modes_default_to_any() {
        let b = bind("Cmd+R", "reload");
        assert_eq!(b.modes(), vec!["any"]);
        assert!(b.matches_mode("normal"));
        assert!(b.matches_mode("insert"));
    }

    #[test]
    fn bind_modes_split_on_comma() {
        let b = bind_mode("j", "cursor:down", "normal,visual");
        assert_eq!(b.modes(), vec!["normal", "visual"]);
        assert!(b.matches_mode("normal"));
        assert!(b.matches_mode("visual"));
        assert!(!b.matches_mode("insert"));
    }

    #[test]
    fn same_chord_different_modes_coexist() {
        let mut reg = BindRegistry::new();
        reg.insert(bind_mode("j", "cursor:down", "normal")).unwrap();
        reg.insert(bind_mode("j", "select:extend-down", "visual"))
            .unwrap();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.resolve("j", "normal").unwrap().command, "cursor:down");
        assert_eq!(
            reg.resolve("j", "visual").unwrap().command,
            "select:extend-down"
        );
        // Insert mode has neither — both were scoped.
        assert!(reg.resolve("j", "insert").is_none());
    }

    #[test]
    fn same_chord_any_mode_replaces_specific() {
        let mut reg = BindRegistry::new();
        reg.insert(bind_mode("j", "old", "normal")).unwrap();
        // Inserting a new binding on the same chord with mode=any
        // should replace it, since "any" overlaps with "normal".
        reg.insert(bind("j", "new")).unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn multi_key_sequence_canonicalizes() {
        assert_eq!(canonicalize_sequence("g g"), "g g");
        assert_eq!(canonicalize_sequence("G G"), "g g");
        assert_eq!(canonicalize_sequence("  leader   f   p  "), "leader f p");
        assert_eq!(canonicalize_sequence("Cmd+R  Cmd+R"), "cmd+r cmd+r");
    }

    #[test]
    fn match_sequence_reports_prefix_vs_complete_vs_miss() {
        let mut reg = BindRegistry::new();
        reg.insert(bind("g g", "scroll:top")).unwrap();
        reg.insert(bind("g e", "scroll:end-word")).unwrap();
        reg.insert(bind("d d", "delete:line")).unwrap();

        // "g" alone — prefix of both `g g` and `g e`.
        assert!(matches!(
            reg.match_sequence("g", "any"),
            SequenceMatch::Prefix
        ));
        // "g g" — complete.
        match reg.match_sequence("g g", "any") {
            SequenceMatch::Complete(b) => assert_eq!(b.command, "scroll:top"),
            other => panic!("expected Complete, got {other:?}"),
        }
        // "d" alone — prefix of `d d`.
        assert!(matches!(
            reg.match_sequence("d", "any"),
            SequenceMatch::Prefix
        ));
        // "x" — neither prefix nor match.
        assert!(matches!(
            reg.match_sequence("x", "any"),
            SequenceMatch::Miss
        ));
        // "g x" — not a prefix (no sequence starts with that).
        assert!(matches!(
            reg.match_sequence("g x", "any"),
            SequenceMatch::Miss
        ));
    }

    #[test]
    fn match_sequence_respects_mode_scope() {
        let mut reg = BindRegistry::new();
        reg.insert(bind_mode("g g", "scroll:top", "normal"))
            .unwrap();
        // In normal mode — it's a hit.
        assert!(matches!(
            reg.match_sequence("g g", "normal"),
            SequenceMatch::Complete(_)
        ));
        // In insert mode — `g` isn't even a prefix.
        assert!(matches!(
            reg.match_sequence("g", "insert"),
            SequenceMatch::Miss
        ));
    }

    #[test]
    fn extend_drops_invalid_commands_silently() {
        let mut reg = CommandRegistry::new();
        reg.extend(vec![
            cmd("ok", Some("reload"), None),
            cmd("bad", None, None), // invalid
        ]);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.names(), vec!["ok"]);
    }

    #[test]
    fn extend_drops_invalid_binds_silently() {
        let mut reg = BindRegistry::new();
        reg.extend(vec![
            bind("Cmd+R", "reload"),
            bind("", "broken"), // invalid
        ]);
        assert_eq!(reg.len(), 1);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_command_with_action() {
        let src = r#"
            (defcommand :name "reload"
                        :description "Reload the current tab."
                        :action "reload")
        "#;
        let specs = compile_commands(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "reload");
        assert_eq!(specs[0].action.as_deref(), Some("reload"));
        assert!(specs[0].body.is_none());
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_command_with_body() {
        let src = r#"
            (defcommand :name "log"
                        :body "(echo \"hi\")")
        "#;
        let specs = compile_commands(src).unwrap();
        assert_eq!(specs[0].body.as_deref(), Some("(echo \"hi\")"));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_bind_round_trips() {
        let src = r#"
            (defbind :key "Cmd+Shift+R"
                     :command "reload"
                     :when "always")
        "#;
        let specs = compile_binds(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].key, "Cmd+Shift+R");
        assert_eq!(specs[0].command, "reload");
        assert_eq!(specs[0].when.as_deref(), Some("always"));
    }
}
