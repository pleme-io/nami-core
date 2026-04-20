//! `(defjs-runtime)` — declarative JavaScript runtime specs + trait
//! surface for pluggable engines.
//!
//! **Foundation lift J1**: Lisp-native substrate meets web-compat.
//! Authoring happens in tatara-lisp; runtime execution happens through
//! a pluggable [`JsRuntime`] trait. The initial implementation is
//! [`MicroEval`] — a 50-line arithmetic + string-concat expression
//! evaluator that proves the dispatch pipeline works end-to-end.
//! Real engines (Boa, rquickjs) slot into the same trait behind
//! feature flags, same pattern as signatures.
//!
//! ```lisp
//! (defjs-runtime
//!   :name           "sandbox"
//!   :fuel-limit     10000000
//!   :memory-limit   "16 MB"
//!   :capabilities   (dom-read storage-read fetch-allowed-hosts)
//!   :allowed-hosts  ("*://*.example.com/*"
//!                    "https://api.example.com/*")
//!   :description    "Default sandboxed eval for (defboost :js)")
//! ```
//!
//! Three kinds of values the eval pipeline moves around:
//!   - source text (user-authored JS, possibly from a boost payload)
//!   - call context (vars + host bindings + fuel)
//!   - result (primitive value OR structured JSON OR error)
//!
//! The capability enum is the security root. A runtime that doesn't
//! advertise `DomWrite` must reject any host call that would mutate
//! the DOM; one that doesn't advertise `FetchAllowedHosts` must
//! reject fetch() calls against every URL.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

// ─── capability surface ──────────────────────────────────────────

/// Host-API capability grants. The runtime enforces these at each
/// host-binding call; a missing capability produces [`EvalError::PermissionDenied`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Read-only DOM access (querySelector, textContent).
    DomRead,
    /// DOM mutation (setAttribute, appendChild).
    DomWrite,
    /// Read from (defstorage)-declared stores the host exposes.
    StorageRead,
    /// Write to (defstorage)-declared stores.
    StorageWrite,
    /// `fetch()` calls against hosts listed in `allowed_hosts`.
    FetchAllowedHosts,
    /// Emit notifications via tsuuchi.
    Notify,
    /// Read the clipboard.
    ClipboardRead,
    /// Write to the clipboard.
    ClipboardWrite,
    /// Stdout-style logging (inspector console).
    Console,
}

// ─── DSL ─────────────────────────────────────────────────────────

/// Declarative JS runtime profile.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defjs-runtime"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JsRuntimeSpec {
    pub name: String,
    /// Instruction / fuel budget per eval call. `0` = unlimited
    /// (unsafe outside curated code).
    #[serde(default = "default_fuel")]
    pub fuel_limit: u64,
    /// Upper bound on guest linear-memory size in bytes. `0` =
    /// unlimited.
    #[serde(default = "default_memory_limit")]
    pub memory_limit_bytes: u64,
    /// Granted host-API capabilities.
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// WebExtensions-style glob patterns for fetch(). Required
    /// when [`Capability::FetchAllowedHosts`] is granted.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_fuel() -> u64 {
    10_000_000
}
fn default_memory_limit() -> u64 {
    16 * 1024 * 1024
}

impl JsRuntimeSpec {
    /// Built-in sandbox profile — DomRead + Console, 10M fuel, 16 MB mem.
    #[must_use]
    pub fn default_profile() -> Self {
        Self {
            name: "default".into(),
            fuel_limit: default_fuel(),
            memory_limit_bytes: default_memory_limit(),
            capabilities: vec![Capability::DomRead, Capability::Console],
            allowed_hosts: vec![],
            description: Some("Default sandbox — DOM read + console, no fetch.".into()),
        }
    }

    #[must_use]
    pub fn has_capability(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Does this runtime allow fetching `url`? Only true when the
    /// `FetchAllowedHosts` cap is granted AND the URL's host matches
    /// one of the `allowed_hosts` globs.
    #[must_use]
    pub fn allows_fetch(&self, url: &str) -> bool {
        if !self.has_capability(Capability::FetchAllowedHosts) {
            return false;
        }
        // Let the extension module's matcher do the glob work.
        self.allowed_hosts
            .iter()
            .any(|g| crate::extension::glob_match_host(g, url))
    }
}

/// Registry of runtime profiles.
#[derive(Debug, Clone, Default)]
pub struct JsRuntimeRegistry {
    specs: Vec<JsRuntimeSpec>,
}

impl JsRuntimeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: JsRuntimeSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = JsRuntimeSpec>) {
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
    pub fn specs(&self) -> &[JsRuntimeSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&JsRuntimeSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

// ─── runtime surface ─────────────────────────────────────────────

/// Input to a single eval call.
#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    /// Variables accessible from the script as bare identifiers.
    /// Primitives only in MicroEval; real engines widen this.
    pub vars: HashMap<String, Value>,
    /// Optional origin URL (for fetch gating + logging).
    pub origin: Option<String>,
}

/// A primitive JS-ish value. Real engines widen to full values; this
/// shape is sufficient for boost-style "return an overlay style" use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}

impl Value {
    #[must_use]
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::String(s) => !s.is_empty(),
            Self::Array(_) | Self::Object(_) => true,
        }
    }

    #[must_use]
    pub fn to_display_string(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(b) => b.to_string(),
            Self::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e16 {
                    format!("{}", *n as i64)
                } else {
                    n.to_string()
                }
            }
            Self::String(s) => s.clone(),
            Self::Array(_) | Self::Object(_) => {
                serde_json::to_string(self).unwrap_or_default()
            }
        }
    }
}

/// Eval outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionResult {
    pub value: Value,
    /// Fuel consumed. Implementation-defined unit; use the same
    /// "instructions" convention across engines so profiles are
    /// comparable.
    pub fuel_used: u64,
    /// Best-effort peak memory in bytes. 0 when the runtime
    /// doesn't track.
    pub memory_peak: u64,
    /// Log lines captured via [`Capability::Console`].
    pub logs: Vec<String>,
}

/// Errors an evaluator may return.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    /// Source wouldn't parse.
    Parse(String),
    /// Fuel exhausted.
    OutOfFuel { limit: u64 },
    /// Memory cap exceeded.
    OutOfMemory { limit_bytes: u64 },
    /// Host call violated the capability grant.
    PermissionDenied(Capability),
    /// Runtime-level error (division by zero, type error, host throw).
    Runtime(String),
    /// The feature flag for a real engine isn't enabled and the
    /// MicroEval fallback doesn't cover this construct.
    Unsupported(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(m) => write!(f, "parse error: {m}"),
            Self::OutOfFuel { limit } => write!(f, "out of fuel (limit {limit})"),
            Self::OutOfMemory { limit_bytes } => {
                write!(f, "out of memory (limit {limit_bytes})")
            }
            Self::PermissionDenied(c) => write!(f, "permission denied: {c:?}"),
            Self::Runtime(m) => write!(f, "runtime error: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Pluggable runtime surface. Engines implement this; the substrate
/// doesn't care whether the guts are a micro-evaluator, Boa, or a
/// wasmtime-hosted QuickJS.
pub trait JsRuntime: std::fmt::Debug + Send + Sync {
    /// Evaluate `source` under `spec` + `ctx`. Must honor fuel +
    /// memory + capability gates declared on `spec`.
    fn eval(
        &self,
        source: &str,
        spec: &JsRuntimeSpec,
        ctx: &EvalContext,
    ) -> Result<ExecutionResult, EvalError>;

    /// Human-friendly engine name for diagnostics + typescape.
    fn engine_name(&self) -> &'static str;
}

// ─── MicroEval — proof-of-pipeline ───────────────────────────────

/// Minimal evaluator — arithmetic (+ − × ÷ %) over f64 literals and
/// identifiers bound in the eval context, plus string concatenation
/// via `+`, plus bare identifier lookup. Not a JavaScript interpreter;
/// it's the simplest thing that proves dispatch + fuel + permissions
/// work end-to-end. Real engine integration plugs into the same
/// [`JsRuntime`] trait.
#[derive(Debug, Clone, Default)]
pub struct MicroEval;

impl JsRuntime for MicroEval {
    fn eval(
        &self,
        source: &str,
        spec: &JsRuntimeSpec,
        ctx: &EvalContext,
    ) -> Result<ExecutionResult, EvalError> {
        let mut fuel = 0u64;
        let (value, logs) = micro_eval_expr(source.trim(), spec, ctx, &mut fuel)?;
        Ok(ExecutionResult {
            value,
            fuel_used: fuel,
            memory_peak: source.len() as u64,
            logs,
        })
    }

    fn engine_name(&self) -> &'static str {
        "micro-eval"
    }
}

fn tick(fuel: &mut u64, spec: &JsRuntimeSpec) -> Result<(), EvalError> {
    *fuel += 1;
    if spec.fuel_limit > 0 && *fuel > spec.fuel_limit {
        return Err(EvalError::OutOfFuel {
            limit: spec.fuel_limit,
        });
    }
    Ok(())
}

fn micro_eval_expr(
    src: &str,
    spec: &JsRuntimeSpec,
    ctx: &EvalContext,
    fuel: &mut u64,
) -> Result<(Value, Vec<String>), EvalError> {
    let mut parser = MicroParser::new(src);
    let v = parser.expr(spec, ctx, fuel)?;
    parser.skip_ws();
    if !parser.eof() {
        return Err(EvalError::Parse(format!(
            "trailing input at offset {}",
            parser.pos
        )));
    }
    // MicroEval doesn't actually do console.log — logs empty. Left
    // in the signature so the richer-engine implementations don't
    // need a new return shape.
    Ok((v, Vec::new()))
}

struct MicroParser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> MicroParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(
        &mut self,
        spec: &JsRuntimeSpec,
        ctx: &EvalContext,
        fuel: &mut u64,
    ) -> Result<Value, EvalError> {
        tick(fuel, spec)?;
        let mut lhs = self.term(spec, ctx, fuel)?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some(b'+') => b'+',
                Some(b'-') => b'-',
                _ => break,
            };
            self.pos += 1;
            let rhs = self.term(spec, ctx, fuel)?;
            lhs = apply_additive(op, lhs, rhs)?;
        }
        Ok(lhs)
    }

    fn term(
        &mut self,
        spec: &JsRuntimeSpec,
        ctx: &EvalContext,
        fuel: &mut u64,
    ) -> Result<Value, EvalError> {
        tick(fuel, spec)?;
        let mut lhs = self.atom(spec, ctx, fuel)?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some(b'*') => b'*',
                Some(b'/') => b'/',
                Some(b'%') => b'%',
                _ => break,
            };
            self.pos += 1;
            let rhs = self.atom(spec, ctx, fuel)?;
            lhs = apply_multiplicative(op, lhs, rhs)?;
        }
        Ok(lhs)
    }

    fn atom(
        &mut self,
        spec: &JsRuntimeSpec,
        ctx: &EvalContext,
        fuel: &mut u64,
    ) -> Result<Value, EvalError> {
        self.skip_ws();
        match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                let v = self.expr(spec, ctx, fuel)?;
                if !self.eat(b')') {
                    return Err(EvalError::Parse("unclosed paren".into()));
                }
                Ok(v)
            }
            Some(b'"') => {
                self.pos += 1;
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if c == b'"' {
                        break;
                    }
                    self.pos += 1;
                }
                if self.peek() != Some(b'"') {
                    return Err(EvalError::Parse("unterminated string".into()));
                }
                let s =
                    std::str::from_utf8(&self.src[start..self.pos])
                        .map_err(|e| EvalError::Parse(e.to_string()))?
                        .to_owned();
                self.pos += 1;
                Ok(Value::String(s))
            }
            Some(c) if c.is_ascii_digit() || c == b'-' => {
                let start = self.pos;
                if c == b'-' {
                    self.pos += 1;
                }
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() || c == b'.' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let slice = std::str::from_utf8(&self.src[start..self.pos])
                    .map_err(|e| EvalError::Parse(e.to_string()))?;
                let n: f64 = slice.parse().map_err(|e: std::num::ParseFloatError| {
                    EvalError::Parse(e.to_string())
                })?;
                Ok(Value::Number(n))
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let name = std::str::from_utf8(&self.src[start..self.pos])
                    .map_err(|e| EvalError::Parse(e.to_string()))?;
                match name {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    "null" => Ok(Value::Null),
                    _ => ctx.vars.get(name).cloned().ok_or_else(|| {
                        EvalError::Runtime(format!("undefined identifier: {name}"))
                    }),
                }
            }
            Some(c) => Err(EvalError::Parse(format!(
                "unexpected char {:?} at offset {}",
                c as char, self.pos
            ))),
            None => Err(EvalError::Parse("unexpected end of input".into())),
        }
    }
}

fn apply_additive(op: u8, lhs: Value, rhs: Value) -> Result<Value, EvalError> {
    match op {
        b'+' => match (&lhs, &rhs) {
            // Number + Number
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            // String + String → concat
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{a}{b}"))),
            // JS-style: Number + String → string coercion
            (Value::Number(_), Value::String(_)) | (Value::String(_), Value::Number(_)) => {
                Ok(Value::String(format!(
                    "{}{}",
                    lhs.to_display_string(),
                    rhs.to_display_string()
                )))
            }
            _ => Err(EvalError::Runtime(format!(
                "bad operands for +: {lhs:?}, {rhs:?}"
            ))),
        },
        b'-' => match (&lhs, &rhs) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a - b)),
            _ => Err(EvalError::Runtime(format!(
                "bad operands for -: {lhs:?}, {rhs:?}"
            ))),
        },
        _ => unreachable!(),
    }
}

fn apply_multiplicative(op: u8, lhs: Value, rhs: Value) -> Result<Value, EvalError> {
    let (Value::Number(a), Value::Number(b)) = (&lhs, &rhs) else {
        return Err(EvalError::Runtime(format!(
            "bad operands for binary op: {lhs:?}, {rhs:?}"
        )));
    };
    match op {
        b'*' => Ok(Value::Number(a * b)),
        b'/' => {
            if *b == 0.0 {
                Err(EvalError::Runtime("division by zero".into()))
            } else {
                Ok(Value::Number(a / b))
            }
        }
        b'%' => {
            if *b == 0.0 {
                Err(EvalError::Runtime("modulo by zero".into()))
            } else {
                Ok(Value::Number(a % b))
            }
        }
        _ => unreachable!(),
    }
}

// ─── Lisp compile + register ─────────────────────────────────────

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<JsRuntimeSpec>, String> {
    tatara_lisp::compile_typed::<JsRuntimeSpec>(src)
        .map_err(|e| format!("failed to compile defjs-runtime forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<JsRuntimeSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_spec() -> JsRuntimeSpec {
        JsRuntimeSpec::default_profile()
    }

    #[test]
    fn spec_has_capability_roundtrip() {
        let s = default_spec();
        assert!(s.has_capability(Capability::DomRead));
        assert!(s.has_capability(Capability::Console));
        assert!(!s.has_capability(Capability::DomWrite));
    }

    #[test]
    fn allows_fetch_requires_both_cap_and_host_glob() {
        let s = JsRuntimeSpec {
            capabilities: vec![Capability::FetchAllowedHosts],
            allowed_hosts: vec!["*://*.example.com/*".into()],
            ..JsRuntimeSpec::default_profile()
        };
        assert!(s.allows_fetch("blog.example.com"));
        assert!(!s.allows_fetch("evil.com"));
        // Without the cap, nothing is allowed.
        let no_cap = JsRuntimeSpec {
            capabilities: vec![],
            allowed_hosts: vec!["*".into()],
            ..JsRuntimeSpec::default_profile()
        };
        assert!(!no_cap.allows_fetch("anywhere.com"));
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = JsRuntimeRegistry::new();
        reg.insert(default_spec());
        reg.insert(JsRuntimeSpec {
            fuel_limit: 1,
            ..default_spec()
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("default").unwrap().fuel_limit, 1);
    }

    #[test]
    fn value_is_truthy_follows_js_semantics() {
        assert!(!Value::Null.is_truthy());
        assert!(!Value::Bool(false).is_truthy());
        assert!(Value::Bool(true).is_truthy());
        assert!(!Value::Number(0.0).is_truthy());
        assert!(Value::Number(1.0).is_truthy());
        assert!(!Value::String(String::new()).is_truthy());
        assert!(Value::String("x".into()).is_truthy());
        assert!(Value::Array(vec![]).is_truthy());
        assert!(Value::Object(HashMap::new()).is_truthy());
    }

    #[test]
    fn value_to_display_formats_integers_without_fractional() {
        assert_eq!(Value::Number(42.0).to_display_string(), "42");
        assert_eq!(Value::Number(3.14).to_display_string(), "3.14");
        assert_eq!(Value::Bool(true).to_display_string(), "true");
    }

    // ── MicroEval ───────────────────────────────────────────────

    #[test]
    fn micro_eval_arithmetic() {
        let e = MicroEval;
        let s = default_spec();
        let c = EvalContext::default();
        let r = e.eval("1 + 2 * 3", &s, &c).unwrap();
        assert_eq!(r.value, Value::Number(7.0));
    }

    #[test]
    fn micro_eval_parentheses() {
        let e = MicroEval;
        let r = e.eval("(1 + 2) * 3", &default_spec(), &EvalContext::default()).unwrap();
        assert_eq!(r.value, Value::Number(9.0));
    }

    #[test]
    fn micro_eval_string_concat() {
        let e = MicroEval;
        let r = e
            .eval(r#""hello" + " " + "world""#, &default_spec(), &EvalContext::default())
            .unwrap();
        assert_eq!(r.value, Value::String("hello world".into()));
    }

    #[test]
    fn micro_eval_string_number_coerces() {
        let e = MicroEval;
        let r = e.eval(r#""n=" + 42"#, &default_spec(), &EvalContext::default()).unwrap();
        assert_eq!(r.value, Value::String("n=42".into()));
    }

    #[test]
    fn micro_eval_identifier_lookup() {
        let e = MicroEval;
        let mut c = EvalContext::default();
        c.vars.insert("x".into(), Value::Number(10.0));
        let r = e.eval("x * x + 1", &default_spec(), &c).unwrap();
        assert_eq!(r.value, Value::Number(101.0));
    }

    #[test]
    fn micro_eval_booleans_and_null() {
        let e = MicroEval;
        let r = e.eval("true", &default_spec(), &EvalContext::default()).unwrap();
        assert_eq!(r.value, Value::Bool(true));
        let r2 = e.eval("null", &default_spec(), &EvalContext::default()).unwrap();
        assert_eq!(r2.value, Value::Null);
    }

    #[test]
    fn micro_eval_division_by_zero() {
        let e = MicroEval;
        let err = e
            .eval("1 / 0", &default_spec(), &EvalContext::default())
            .unwrap_err();
        assert!(matches!(err, EvalError::Runtime(ref m) if m.contains("division")));
    }

    #[test]
    fn micro_eval_out_of_fuel() {
        let e = MicroEval;
        let tight = JsRuntimeSpec {
            fuel_limit: 2,
            ..default_spec()
        };
        let err = e
            .eval("1 + 2 + 3 + 4 + 5", &tight, &EvalContext::default())
            .unwrap_err();
        assert!(matches!(err, EvalError::OutOfFuel { limit: 2 }));
    }

    #[test]
    fn micro_eval_zero_fuel_is_unlimited() {
        let e = MicroEval;
        let s = JsRuntimeSpec {
            fuel_limit: 0,
            ..default_spec()
        };
        let r = e.eval("1+2+3+4+5+6+7+8+9+10", &s, &EvalContext::default()).unwrap();
        assert_eq!(r.value, Value::Number(55.0));
    }

    #[test]
    fn micro_eval_undefined_identifier_is_runtime_error() {
        let e = MicroEval;
        let err = e
            .eval("x + 1", &default_spec(), &EvalContext::default())
            .unwrap_err();
        assert!(matches!(err, EvalError::Runtime(ref m) if m.contains("undefined")));
    }

    #[test]
    fn micro_eval_reports_fuel_used() {
        let e = MicroEval;
        let r = e.eval("1 + 2 + 3", &default_spec(), &EvalContext::default()).unwrap();
        assert!(r.fuel_used > 0);
    }

    #[test]
    fn micro_eval_returns_engine_name() {
        assert_eq!(MicroEval.engine_name(), "micro-eval");
    }

    #[test]
    fn micro_eval_trailing_garbage_is_parse_error() {
        let e = MicroEval;
        let err = e
            .eval("1 + 2 }", &default_spec(), &EvalContext::default())
            .unwrap_err();
        assert!(matches!(err, EvalError::Parse(_)));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_runtime_form() {
        let src = r#"
            (defjs-runtime
              :name           "sandbox"
              :fuel-limit     5000000
              :memory-limit-bytes 8388608
              :capabilities   (dom-read console storage-read)
              :allowed-hosts  ("*://*.example.com/*"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        let s = &specs[0];
        assert_eq!(s.name, "sandbox");
        assert_eq!(s.fuel_limit, 5_000_000);
        assert!(s.has_capability(Capability::DomRead));
        assert!(s.has_capability(Capability::StorageRead));
        assert!(s.has_capability(Capability::Console));
    }
}
