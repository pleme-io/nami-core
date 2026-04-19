//! `(defwasm-agent)` — Lisp-declared WASM scrapers wired into the
//! substrate pipeline.
//!
//! ```lisp
//! (defwasm-agent :name "recipe-extractor"
//!                :wasm "recipe.wasm"
//!                :on "page-load"
//!                :max-fuel 5000000
//!                :allow-stdout #t)
//! ```
//!
//! At navigate time, the runtime resolves `:wasm` (relative paths
//! against `$XDG_CONFIG_HOME/namimado/wasm/`, absolute paths used
//! verbatim), reads the bytes, and runs through `WasmHost::run_agent`
//! with the caller-supplied `WasmAgentContext` (carrying the
//! read-only DOM snapshot + output accumulator).
//!
//! Trigger gating lives at the `on` field (typically `"page-load"`).
//! Predicate gating (`:when`) is reserved for a later session —
//! the same predicate evaluator the regular `(defagent)` uses would
//! plug in here.
//!
//! ## Relation to (defagent)
//!
//! `(defagent)` composes Lisp-authored transforms into plans. That's
//! for in-process reshaping of the DOM. `(defwasm-agent)` runs a
//! *precompiled* `.wasm` module against a read-only DOM snapshot to
//! extract data — different responsibility, sandboxed execution.
//! Both can coexist on the same page.

use serde::{Deserialize, Serialize};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// One agent declaration authored in Lisp.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defwasm-agent"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WasmAgentSpec {
    pub name: String,
    /// Path to the compiled `.wasm` module. Relative paths resolve
    /// against the runtime's wasm-agents directory
    /// (`$XDG_CONFIG_HOME/namimado/wasm/`); absolute paths are used
    /// verbatim.
    pub wasm: String,
    /// Trigger event — currently only `"page-load"` is honored by
    /// the namimado pipeline; other strings are accepted + ignored.
    pub on: String,
    /// Fuel budget for this run. `None` defers to the host default
    /// (`WasmCaps::default().max_fuel`).
    #[serde(default)]
    pub max_fuel: Option<u64>,
    /// Capture stdout. Off by default — most scrapers emit via
    /// `nami.emit` and don't need stdout.
    #[serde(default)]
    pub allow_stdout: bool,
    #[serde(default)]
    pub description: Option<String>,
}

/// Registry of agents, indexed by name.
#[derive(Debug, Clone, Default)]
pub struct WasmAgentRegistry {
    specs: Vec<WasmAgentSpec>,
}

impl WasmAgentRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: WasmAgentSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = WasmAgentSpec>) {
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

    /// Specs matching a trigger event name (plus the empty match
    /// shortcut — specs with `:on ""` fire on every trigger).
    pub fn for_trigger<'a>(&'a self, trigger: &'a str) -> impl Iterator<Item = &'a WasmAgentSpec> {
        self.specs
            .iter()
            .filter(move |s| s.on == trigger || s.on.is_empty())
    }

    pub fn specs(&self) -> &[WasmAgentSpec] {
        &self.specs
    }
}

/// One agent's outcome.
#[derive(Debug, Clone)]
pub struct WasmAgentReport {
    pub name: String,
    pub wasm_path: String,
    /// `Ok(output_bytes)` when the run completed; `Err(msg)` when
    /// loading or execution failed. Errors are tolerated (logged
    /// upstream) not panics.
    pub outcome: Result<Vec<u8>, String>,
    pub fuel_used: u64,
    pub duration_ms: u128,
}

impl WasmAgentReport {
    #[must_use]
    pub fn output_as_str(&self) -> String {
        match &self.outcome {
            Ok(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            Err(e) => format!("<err: {e}>"),
        }
    }

    #[must_use]
    pub fn ok(&self) -> bool {
        self.outcome.is_ok()
    }
}

/// Compile a Lisp source of `(defwasm-agent …)` forms into specs.
#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<WasmAgentSpec>, String> {
    tatara_lisp::compile_typed::<WasmAgentSpec>(src)
        .map_err(|e| format!("failed to compile defwasm-agent forms: {e}"))
}

/// Registration hook so workspace coherence checkers see the keyword.
#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<WasmAgentSpec>();
}

/// Execute every spec whose `:on` matches `trigger` against the
/// supplied `WasmHost` + context. Load failures for individual
/// modules don't stop the pass — each spec's report records its
/// own outcome. The caller provides a closure `resolve_bytes` that
/// turns the `:wasm` path into a `Vec<u8>` (namimado uses that to
/// enforce its `~/.config/namimado/wasm/` prefix policy).
#[cfg(feature = "wasm")]
pub fn run(
    registry: &WasmAgentRegistry,
    trigger: &str,
    host: &crate::wasm::WasmHost,
    cx_template: &crate::wasm::WasmAgentContext,
    mut resolve_bytes: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Vec<WasmAgentReport> {
    registry
        .for_trigger(trigger)
        .map(|spec| run_one(spec, host, cx_template, &mut resolve_bytes))
        .collect()
}

#[cfg(feature = "wasm")]
fn run_one(
    spec: &WasmAgentSpec,
    host: &crate::wasm::WasmHost,
    cx_template: &crate::wasm::WasmAgentContext,
    resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> WasmAgentReport {
    let bytes = match resolve(&spec.wasm) {
        Ok(b) => b,
        Err(e) => {
            return WasmAgentReport {
                name: spec.name.clone(),
                wasm_path: spec.wasm.clone(),
                outcome: Err(format!("resolve: {e}")),
                fuel_used: 0,
                duration_ms: 0,
            };
        }
    };
    let caps = crate::wasm::WasmCaps {
        allow_stdout: spec.allow_stdout,
        max_fuel: spec.max_fuel.or(Some(10_000_000)),
        ..crate::wasm::WasmCaps::default()
    };
    let t0 = std::time::Instant::now();
    let result = host.run_agent(&bytes, caps, cx_template.clone());
    let duration_ms = t0.elapsed().as_millis();
    match result {
        Ok(out) => WasmAgentReport {
            name: spec.name.clone(),
            wasm_path: spec.wasm.clone(),
            outcome: Ok(out.output),
            fuel_used: out.base.fuel_used,
            duration_ms,
        },
        Err(e) => WasmAgentReport {
            name: spec.name.clone(),
            wasm_path: spec.wasm.clone(),
            outcome: Err(e.to_string()),
            fuel_used: 0,
            duration_ms,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_one_form() {
        let src = r#"
            (defwasm-agent :name "extractor"
                           :wasm "extractor.wasm"
                           :on "page-load"
                           :max-fuel 5000000)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "extractor");
        assert_eq!(specs[0].wasm, "extractor.wasm");
        assert_eq!(specs[0].on, "page-load");
        assert_eq!(specs[0].max_fuel, Some(5_000_000));
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_rejects_malformed_source() {
        let src = r#"(defwasm-agent :name "x")"#; // missing required :wasm / :on
        let err = compile(src);
        assert!(err.is_err() || err.unwrap().is_empty());
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = WasmAgentRegistry::new();
        reg.insert(WasmAgentSpec {
            name: "x".into(),
            wasm: "a.wasm".into(),
            on: "page-load".into(),
            max_fuel: None,
            allow_stdout: false,
            description: None,
        });
        reg.insert(WasmAgentSpec {
            name: "x".into(),
            wasm: "b.wasm".into(),
            on: "page-load".into(),
            max_fuel: None,
            allow_stdout: false,
            description: None,
        });
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.specs()[0].wasm, "b.wasm");
    }

    #[test]
    fn for_trigger_matches_exact_and_empty() {
        let mut reg = WasmAgentRegistry::new();
        reg.insert(WasmAgentSpec {
            name: "on-load".into(),
            wasm: "a.wasm".into(),
            on: "page-load".into(),
            max_fuel: None,
            allow_stdout: false,
            description: None,
        });
        reg.insert(WasmAgentSpec {
            name: "always".into(),
            wasm: "b.wasm".into(),
            on: String::new(),
            max_fuel: None,
            allow_stdout: false,
            description: None,
        });
        reg.insert(WasmAgentSpec {
            name: "on-nav".into(),
            wasm: "c.wasm".into(),
            on: "navigate-start".into(),
            max_fuel: None,
            allow_stdout: false,
            description: None,
        });
        let fired: Vec<_> = reg
            .for_trigger("page-load")
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(fired, vec!["on-load", "always"]);
    }

    #[cfg(all(feature = "wasm", feature = "lisp"))]
    #[test]
    fn run_executes_one_agent_and_collects_output() {
        use crate::wasm::{WasmAgentContext, WasmCaps, WasmHost};
        let wasm_bytes = wat::parse_str(
            r#"(module
                 (import "nami" "emit" (func $emit (param i32 i32) (result i32)))
                 (memory (export "memory") 1)
                 (data (i32.const 0) "hello")
                 (func (export "_start")
                   (drop (call $emit (i32.const 0) (i32.const 5)))))"#,
        )
        .unwrap();

        let mut reg = WasmAgentRegistry::new();
        reg.insert(WasmAgentSpec {
            name: "hello".into(),
            wasm: "virtual/hello.wasm".into(),
            on: "page-load".into(),
            max_fuel: None,
            allow_stdout: false,
            description: None,
        });

        let host = WasmHost::new().unwrap();
        let cx = WasmAgentContext::default();
        // The resolver closure is how namimado will plug in its path
        // policy. For the test we just hand back pre-compiled bytes.
        let reports = run(&reg, "page-load", &host, &cx, |_path| {
            Ok(wasm_bytes.clone())
        });
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(r.name, "hello");
        assert!(r.ok(), "expected ok outcome, got {:?}", r.outcome);
        assert_eq!(r.output_as_str(), "hello");
        assert!(r.fuel_used > 0);
    }

    #[cfg(all(feature = "wasm"))]
    #[test]
    fn run_surfaces_resolve_errors_without_panicking() {
        use crate::wasm::{WasmAgentContext, WasmHost};
        let mut reg = WasmAgentRegistry::new();
        reg.insert(WasmAgentSpec {
            name: "ghost".into(),
            wasm: "nonexistent.wasm".into(),
            on: "page-load".into(),
            max_fuel: None,
            allow_stdout: false,
            description: None,
        });
        let host = WasmHost::new().unwrap();
        let reports = run(&reg, "page-load", &host, &WasmAgentContext::default(), |_| {
            Err("file not found".into())
        });
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].ok());
        assert!(reports[0].output_as_str().contains("resolve"));
    }

    #[cfg(all(feature = "wasm"))]
    #[test]
    fn run_skips_agents_whose_trigger_does_not_match() {
        use crate::wasm::{WasmAgentContext, WasmHost};
        let mut reg = WasmAgentRegistry::new();
        reg.insert(WasmAgentSpec {
            name: "page-load-only".into(),
            wasm: "x.wasm".into(),
            on: "page-load".into(),
            max_fuel: None,
            allow_stdout: false,
            description: None,
        });
        let host = WasmHost::new().unwrap();
        let reports = run(&reg, "some-other-trigger", &host, &WasmAgentContext::default(), |_| {
            panic!("resolver shouldn't be called")
        });
        assert!(reports.is_empty());
    }
}
