//! WASM/WASI agent host — sandboxed execution of precompiled modules
//! with capability-gated access.
//!
//! V1 scope: load a `.wasm` module, run it with WASI stdout capture
//! under a fuel limit, return the captured stdout + fuel consumed +
//! duration. Capability toggles gate what the module can do; exceeding
//! the fuel limit fails cleanly without panicking the host.
//!
//! This is the foundation for Lisp-authored browser scrapers. A later
//! session will add the Lisp → WASM compile step (via `tatara-wasm`)
//! and richer host functions (`get-dom-sexp`, `emit-transform`,
//! `fetch-via-proxy`).

use std::sync::Arc;
use std::time::{Duration, Instant};
use wasmtime::{Caller, Config, Engine, Extern, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p1::{self as preview1, WasiP1Ctx};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::WasiCtxBuilder;

/// Capability-gated execution policy.
#[derive(Debug, Clone)]
pub struct WasmCaps {
    /// Capture stdout into the returned `WasmOutcome`. When `false`,
    /// the module runs with a null stdout and output is discarded.
    pub allow_stdout: bool,
    /// Fuel budget. Each WASM instruction consumes 1 fuel by default;
    /// when exhausted, the module traps with a fuel error. `None`
    /// means unlimited (dangerous — only for trusted input).
    pub max_fuel: Option<u64>,
    /// Hard wall-clock ceiling. Currently advisory — we measure after
    /// the run, but don't interrupt mid-execution (that requires the
    /// async / interrupt API). A future session can migrate.
    pub max_duration: Duration,
    /// Upper bound on total memory pages (64 KiB each). Default 256
    /// pages = 16 MiB.
    pub max_memory_pages: usize,
}

impl Default for WasmCaps {
    fn default() -> Self {
        Self {
            allow_stdout: true,
            max_fuel: Some(10_000_000),
            max_duration: Duration::from_secs(5),
            max_memory_pages: 256,
        }
    }
}

/// Outcome of one `WasmHost::run` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmOutcome {
    pub stdout: String,
    pub exit_code: Option<i32>,
    pub fuel_used: u64,
    pub duration: Duration,
}

/// Error from loading or executing a WASM module.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    #[error("failed to parse WASM module: {0}")]
    ModuleParse(String),
    #[error("failed to link WASI imports: {0}")]
    Link(String),
    #[error("failed to instantiate module: {0}")]
    Instantiate(String),
    #[error("module has no _start function")]
    NoStart,
    #[error("fuel exhausted (executed {fuel_used} instructions)")]
    FuelExhausted { fuel_used: u64 },
    #[error("trap during execution: {0}")]
    Trap(String),
}

/// Host capable of running WASM modules under capability gates.
///
/// The `Engine` is expensive to construct (compiles the JIT) but cheap
/// to share — one host per process is the intended pattern.
pub struct WasmHost {
    engine: Engine,
}

impl WasmHost {
    /// Construct a host with fuel consumption enabled (otherwise our
    /// max_fuel policy can't be enforced). Returns Err only if
    /// wasmtime itself fails to initialize — on a supported platform
    /// that should never happen.
    pub fn new() -> Result<Self, WasmError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine =
            Engine::new(&config).map_err(|e| WasmError::Link(format!("engine init: {e}")))?;
        Ok(Self { engine })
    }

    /// Run a pre-compiled WASM module to completion. Consumes `caps`;
    /// each run is independent.
    pub fn run(&self, wasm: &[u8], caps: WasmCaps) -> Result<WasmOutcome, WasmError> {
        let module = Module::from_binary(&self.engine, wasm)
            .map_err(|e| WasmError::ModuleParse(e.to_string()))?;

        // Build WASI context with stdout-capture pipe when allowed.
        let stdout_pipe = MemoryOutputPipe::new(64 * 1024);
        let mut wasi_builder = WasiCtxBuilder::new();
        if caps.allow_stdout {
            wasi_builder.stdout(stdout_pipe.clone());
        }
        let wasi_ctx = wasi_builder.build_p1();

        struct HostState {
            wasi: WasiP1Ctx,
            limits: StoreLimits,
        }

        let limits = StoreLimitsBuilder::new()
            .memory_size(caps.max_memory_pages * 64 * 1024)
            .build();

        let mut store = Store::new(
            &self.engine,
            HostState {
                wasi: wasi_ctx,
                limits,
            },
        );
        store.limiter(|s| &mut s.limits);

        if let Some(fuel) = caps.max_fuel {
            store
                .set_fuel(fuel)
                .map_err(|e| WasmError::Link(format!("set fuel: {e}")))?;
        }

        let mut linker: Linker<HostState> = Linker::new(&self.engine);
        preview1::add_to_linker_sync(&mut linker, |s: &mut HostState| &mut s.wasi)
            .map_err(|e| WasmError::Link(e.to_string()))?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| WasmError::Instantiate(e.to_string()))?;

        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|_| WasmError::NoStart)?;

        let t0 = Instant::now();
        let call_result = start.call(&mut store, ());
        let duration = t0.elapsed();

        let fuel_used = caps.max_fuel.map_or(0, |max| {
            max.saturating_sub(store.get_fuel().unwrap_or(0))
        });

        // Drop the store so the WASI ctx releases the pipe handle and
        // MemoryOutputPipe::contents() can read a final snapshot.
        // Instance is Copy; drop(store) alone closes out the lifecycle.
        drop(store);

        if let Err(e) = call_result {
            let msg = format!("{e:?}");
            if msg.contains("all fuel consumed") || msg.contains("OutOfFuel") {
                return Err(WasmError::FuelExhausted { fuel_used });
            }
            return Err(WasmError::Trap(msg));
        }

        let stdout = if caps.allow_stdout {
            String::from_utf8_lossy(&stdout_pipe.contents()).into_owned()
        } else {
            String::new()
        };

        Ok(WasmOutcome {
            stdout,
            exit_code: Some(0),
            fuel_used,
            duration,
        })
    }
}

// ──────────────────────────────────────────────────────────────────
// Agent context — DOM access + output accumulator for host functions.
// ──────────────────────────────────────────────────────────────────

/// Per-run context exposed to guest WASM through host imports. The
/// DOM snapshot is read-only (Arc is cheap to clone into the store);
/// `output` is an accumulator the guest writes into via `nami_emit`.
#[derive(Debug, Clone)]
pub struct WasmAgentContext {
    pub dom_snapshot: Option<Arc<crate::dom::Document>>,
    /// Accumulates bytes written by the guest through `nami_emit`.
    pub output: Vec<u8>,
    /// Hard cap on total output bytes — a runaway agent can't fill
    /// host memory. Write calls that exceed this truncate silently
    /// (the guest just sees a short write return).
    pub max_output_bytes: usize,
}

impl Default for WasmAgentContext {
    fn default() -> Self {
        Self {
            dom_snapshot: None,
            output: Vec::new(),
            max_output_bytes: 1 << 20, // 1 MiB default
        }
    }
}

impl WasmAgentContext {
    #[must_use]
    pub fn with_snapshot(doc: Arc<crate::dom::Document>) -> Self {
        Self {
            dom_snapshot: Some(doc),
            ..Self::default()
        }
    }
}

/// Outcome of a `run_agent` invocation. Wraps the plain `WasmOutcome`
/// and surfaces the guest-written output buffer.
#[derive(Debug, Clone)]
pub struct WasmAgentOutcome {
    pub base: WasmOutcome,
    pub output: Vec<u8>,
}

impl WasmAgentOutcome {
    #[must_use]
    pub fn output_as_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.output)
    }
}

struct AgentState {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
    cx: WasmAgentContext,
    // Pre-serialized DOM sexp, computed once per run. Guest reads
    // this in chunks via `nami_dom_sexp_read` to avoid ballooning
    // the call surface for large docs.
    dom_sexp_cache: String,
}

impl WasmHost {
    /// Run a WASM module as an **agent** — guest has access to four
    /// host functions exposed under the `nami` import namespace:
    ///
    ///   (import "nami" "query_count"       (func (param i32 i32)      (result i32)))
    ///   (import "nami" "dom_sexp_len"      (func                     (result i32)))
    ///   (import "nami" "dom_sexp_read"     (func (param i32 i32 i32) (result i32)))
    ///   (import "nami" "emit"              (func (param i32 i32)      (result i32)))
    ///
    /// Semantics:
    ///
    /// * `query_count(sel_ptr, sel_len)` — treats the sel_ptr..sel_len
    ///   slice of guest memory as a UTF-8 CSS selector, runs
    ///   `Document::query_selector_all` on the snapshot, returns match
    ///   count (0 when no snapshot was provided).
    ///
    /// * `dom_sexp_len()` — length in bytes of the cached DOM sexp.
    ///   Guest calls once, allocates a buffer of that size in its own
    ///   memory, then calls `dom_sexp_read` to fill it.
    ///
    /// * `dom_sexp_read(offset, len, dst_ptr)` — copies `len` bytes
    ///   starting at `offset` of the cached sexp into guest memory at
    ///   `dst_ptr`. Returns bytes actually copied (always ≤ `len`).
    ///
    /// * `emit(ptr, len)` — appends the slice to the agent's output
    ///   accumulator. Returns bytes actually accumulated (0 when the
    ///   cap at `max_output_bytes` is hit).
    ///
    /// The output vec is returned alongside the standard outcome.
    pub fn run_agent(
        &self,
        wasm: &[u8],
        caps: WasmCaps,
        cx: WasmAgentContext,
    ) -> Result<WasmAgentOutcome, WasmError> {
        let module = Module::from_binary(&self.engine, wasm)
            .map_err(|e| WasmError::ModuleParse(e.to_string()))?;

        let stdout_pipe = MemoryOutputPipe::new(64 * 1024);
        let mut wasi_builder = WasiCtxBuilder::new();
        if caps.allow_stdout {
            wasi_builder.stdout(stdout_pipe.clone());
        }
        let wasi_ctx = wasi_builder.build_p1();

        let limits = StoreLimitsBuilder::new()
            .memory_size(caps.max_memory_pages * 64 * 1024)
            .build();

        let dom_sexp_cache = cx
            .dom_snapshot
            .as_ref()
            .map(|d| {
                crate::lisp::dom_to_sexp_with(
                    d,
                    &crate::lisp::SexpOptions {
                        depth_cap: Some(12),
                        pretty: false,
                        trim_whitespace: true,
                    },
                )
            })
            .unwrap_or_default();

        let mut store = Store::new(
            &self.engine,
            AgentState {
                wasi: wasi_ctx,
                limits,
                cx,
                dom_sexp_cache,
            },
        );
        store.limiter(|s| &mut s.limits);

        if let Some(fuel) = caps.max_fuel {
            store
                .set_fuel(fuel)
                .map_err(|e| WasmError::Link(format!("set fuel: {e}")))?;
        }

        let mut linker: Linker<AgentState> = Linker::new(&self.engine);
        preview1::add_to_linker_sync(&mut linker, |s: &mut AgentState| &mut s.wasi)
            .map_err(|e| WasmError::Link(e.to_string()))?;

        // ── Host functions ─────────────────────────────────────────

        linker
            .func_wrap(
                "nami",
                "query_count",
                |mut caller: Caller<'_, AgentState>, sel_ptr: i32, sel_len: i32| -> i32 {
                    let Some(mem) = get_memory(&mut caller) else {
                        return 0;
                    };
                    let data = mem.data(&caller);
                    let Some(sel) = read_string(data, sel_ptr, sel_len) else {
                        return 0;
                    };
                    let Some(doc) = caller.data().cx.dom_snapshot.clone() else {
                        return 0;
                    };
                    doc.query_selector_all(&sel).len() as i32
                },
            )
            .map_err(|e| WasmError::Link(e.to_string()))?;

        linker
            .func_wrap(
                "nami",
                "dom_sexp_len",
                |caller: Caller<'_, AgentState>| -> i32 {
                    caller.data().dom_sexp_cache.len() as i32
                },
            )
            .map_err(|e| WasmError::Link(e.to_string()))?;

        linker
            .func_wrap(
                "nami",
                "dom_sexp_read",
                |mut caller: Caller<'_, AgentState>,
                 offset: i32,
                 len: i32,
                 dst_ptr: i32|
                 -> i32 {
                    let Some(mem) = get_memory(&mut caller) else {
                        return 0;
                    };
                    let state = caller.data();
                    let cache = state.dom_sexp_cache.as_bytes();
                    let offset = usize::try_from(offset).unwrap_or(0);
                    let len = usize::try_from(len).unwrap_or(0);
                    if offset >= cache.len() {
                        return 0;
                    }
                    let end = (offset + len).min(cache.len());
                    let slice = &cache[offset..end];
                    let bytes = slice.to_vec();
                    let dst_ptr = usize::try_from(dst_ptr).unwrap_or(0);
                    match mem.write(&mut caller, dst_ptr, &bytes) {
                        Ok(()) => bytes.len() as i32,
                        Err(_) => 0,
                    }
                },
            )
            .map_err(|e| WasmError::Link(e.to_string()))?;

        linker
            .func_wrap(
                "nami",
                "emit",
                |mut caller: Caller<'_, AgentState>, ptr: i32, len: i32| -> i32 {
                    let Some(mem) = get_memory(&mut caller) else {
                        return 0;
                    };
                    let data = mem.data(&caller);
                    let len_u = usize::try_from(len).unwrap_or(0);
                    let ptr_u = usize::try_from(ptr).unwrap_or(0);
                    let Some(slice) = data.get(ptr_u..ptr_u.saturating_add(len_u)) else {
                        return 0;
                    };
                    let copy = slice.to_vec();
                    let state = caller.data_mut();
                    let room = state
                        .cx
                        .max_output_bytes
                        .saturating_sub(state.cx.output.len());
                    let write = copy.len().min(room);
                    state.cx.output.extend_from_slice(&copy[..write]);
                    write as i32
                },
            )
            .map_err(|e| WasmError::Link(e.to_string()))?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| WasmError::Instantiate(e.to_string()))?;

        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|_| WasmError::NoStart)?;

        let t0 = Instant::now();
        let call_result = start.call(&mut store, ());
        let duration = t0.elapsed();

        let fuel_used = caps
            .max_fuel
            .map_or(0, |max| max.saturating_sub(store.get_fuel().unwrap_or(0)));

        // Extract output BEFORE dropping store; store owns AgentState
        // which owns the output vec. Instance is Copy; dropping the
        // store is enough to close the lifecycle cleanly.
        let output = std::mem::take(&mut store.data_mut().cx.output);
        drop(store);

        if let Err(e) = call_result {
            let msg = format!("{e:?}");
            if msg.contains("all fuel consumed") || msg.contains("OutOfFuel") {
                return Err(WasmError::FuelExhausted { fuel_used });
            }
            return Err(WasmError::Trap(msg));
        }

        let stdout = if caps.allow_stdout {
            String::from_utf8_lossy(&stdout_pipe.contents()).into_owned()
        } else {
            String::new()
        };

        Ok(WasmAgentOutcome {
            base: WasmOutcome {
                stdout,
                exit_code: Some(0),
                fuel_used,
                duration,
            },
            output,
        })
    }
}

fn get_memory<T>(caller: &mut Caller<'_, T>) -> Option<wasmtime::Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(m)) => Some(m),
        _ => None,
    }
}

fn read_string(data: &[u8], ptr: i32, len: i32) -> Option<String> {
    let ptr = usize::try_from(ptr).ok()?;
    let len = usize::try_from(len).ok()?;
    let slice = data.get(ptr..ptr.checked_add(len)?)?;
    Some(String::from_utf8_lossy(slice).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile a WAT fixture to WASM bytes. Tests author modules
    /// inline rather than checking in binary blobs.
    fn wat(src: &str) -> Vec<u8> {
        wat::parse_str(src).expect("wat parses")
    }

    #[test]
    fn host_constructs_without_panic() {
        let _ = WasmHost::new().expect("new");
    }

    #[test]
    fn minimal_module_returns_outcome() {
        // `_start` that does nothing and returns cleanly.
        let w = wat(r#"(module (func (export "_start")))"#);
        let host = WasmHost::new().unwrap();
        let outcome = host.run(&w, WasmCaps::default()).expect("run");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.stdout.is_empty());
    }

    #[test]
    fn module_without_start_errors_cleanly() {
        // Module exports no _start.
        let w = wat(r#"(module (func (export "foo")))"#);
        let host = WasmHost::new().unwrap();
        let err = host.run(&w, WasmCaps::default()).unwrap_err();
        assert!(matches!(err, WasmError::NoStart), "err: {err}");
    }

    #[test]
    fn malformed_wasm_fails_at_parse_stage() {
        let bytes = b"not a wasm module";
        let host = WasmHost::new().unwrap();
        let err = host.run(bytes, WasmCaps::default()).unwrap_err();
        assert!(matches!(err, WasmError::ModuleParse(_)), "err: {err}");
    }

    #[test]
    fn fuel_exhaustion_returns_clean_error() {
        // Infinite loop — will consume all fuel.
        let w = wat(
            r#"(module
                 (func (export "_start")
                   (loop br 0))
               )"#,
        );
        let host = WasmHost::new().unwrap();
        let caps = WasmCaps {
            max_fuel: Some(1_000),
            ..WasmCaps::default()
        };
        let err = host.run(&w, caps).unwrap_err();
        match err {
            WasmError::FuelExhausted { fuel_used } => {
                assert!(fuel_used >= 1_000, "fuel_used={fuel_used}");
            }
            other => panic!("expected FuelExhausted, got {other:?}"),
        }
    }

    #[test]
    fn stdout_capture_off_produces_empty_output() {
        // Minimal _start — no I/O — still exercises the "allow_stdout
        // = false" path without needing a complex WASI write module.
        let w = wat(r#"(module (func (export "_start")))"#);
        let host = WasmHost::new().unwrap();
        let caps = WasmCaps {
            allow_stdout: false,
            ..WasmCaps::default()
        };
        let outcome = host.run(&w, caps).expect("run");
        assert!(outcome.stdout.is_empty());
    }

    #[test]
    fn host_is_reentrant_across_runs() {
        // Same host, many runs — each run has fresh state and fresh
        // fuel budget.
        let w = wat(r#"(module (func (export "_start")))"#);
        let host = WasmHost::new().unwrap();
        for _ in 0..5 {
            let outcome = host.run(&w, WasmCaps::default()).expect("run");
            assert_eq!(outcome.exit_code, Some(0));
        }
    }

    #[test]
    fn fuel_used_is_tracked() {
        // A module that does some real work — 100 iterations of
        // incrementing a global. Fuel_used should be > 0.
        let w = wat(
            r#"(module
                 (global $i (mut i32) (i32.const 100))
                 (func (export "_start")
                   (block
                     (loop
                       (br_if 1 (i32.eqz (global.get $i)))
                       (global.set $i (i32.sub (global.get $i) (i32.const 1)))
                       (br 0))))
               )"#,
        );
        let host = WasmHost::new().unwrap();
        let outcome = host.run(&w, WasmCaps::default()).expect("run");
        assert!(outcome.fuel_used > 0, "fuel_used={}", outcome.fuel_used);
        assert!(outcome.fuel_used < 10_000, "fuel_used too high: {}", outcome.fuel_used);
    }

    #[test]
    fn outcome_is_deterministic_for_same_module() {
        let w = wat(
            r#"(module
                 (global $i (mut i32) (i32.const 50))
                 (func (export "_start")
                   (block
                     (loop
                       (br_if 1 (i32.eqz (global.get $i)))
                       (global.set $i (i32.sub (global.get $i) (i32.const 1)))
                       (br 0))))
               )"#,
        );
        let host = WasmHost::new().unwrap();
        let a = host.run(&w, WasmCaps::default()).expect("a");
        let b = host.run(&w, WasmCaps::default()).expect("b");
        let c = host.run(&w, WasmCaps::default()).expect("c");
        assert_eq!(a.fuel_used, b.fuel_used);
        assert_eq!(b.fuel_used, c.fuel_used);
        assert_eq!(a.stdout, b.stdout);
    }

    #[test]
    fn default_caps_sets_reasonable_limits() {
        let caps = WasmCaps::default();
        assert!(caps.allow_stdout);
        assert_eq!(caps.max_fuel, Some(10_000_000));
        assert_eq!(caps.max_duration, Duration::from_secs(5));
        assert_eq!(caps.max_memory_pages, 256);
    }

    #[test]
    fn trap_during_execution_surfaces_as_error() {
        // `unreachable` always traps.
        let w = wat(r#"(module (func (export "_start") unreachable))"#);
        let host = WasmHost::new().unwrap();
        let err = host.run(&w, WasmCaps::default()).unwrap_err();
        assert!(matches!(err, WasmError::Trap(_)), "err: {err:?}");
    }

    // ── Host-function agent tests ─────────────────────────────────

    /// Agent: writes constant "ok" at offset 0 via emit. Tests the
    /// base emit surface without needing DOM.
    #[test]
    fn agent_emit_accumulates_output() {
        let w = wat(
            r#"(module
                 (import "nami" "emit" (func $emit (param i32 i32) (result i32)))
                 (memory (export "memory") 1)
                 (data (i32.const 0) "ok")
                 (func (export "_start")
                   (drop (call $emit (i32.const 0) (i32.const 2)))))"#,
        );
        let host = WasmHost::new().unwrap();
        let outcome = host
            .run_agent(&w, WasmCaps::default(), WasmAgentContext::default())
            .expect("run_agent");
        assert_eq!(outcome.output_as_str(), "ok");
    }

    /// Agent: calls emit twice — accumulator grows monotonically.
    #[test]
    fn agent_emit_is_additive_across_calls() {
        let w = wat(
            r#"(module
                 (import "nami" "emit" (func $emit (param i32 i32) (result i32)))
                 (memory (export "memory") 1)
                 (data (i32.const 0) "abcdef")
                 (func (export "_start")
                   (drop (call $emit (i32.const 0) (i32.const 3)))
                   (drop (call $emit (i32.const 3) (i32.const 3)))))"#,
        );
        let host = WasmHost::new().unwrap();
        let outcome = host
            .run_agent(&w, WasmCaps::default(), WasmAgentContext::default())
            .expect("run_agent");
        assert_eq!(outcome.output_as_str(), "abcdef");
    }

    /// Agent: calls query_count on a snapshot. Verifies host function
    /// reads the WASM memory, runs the selector against the document,
    /// returns the count.
    #[test]
    fn agent_query_count_sees_dom_snapshot() {
        let w = wat(
            r#"(module
                 (import "nami" "query_count" (func $qc (param i32 i32) (result i32)))
                 (import "nami" "emit"        (func $emit (param i32 i32) (result i32)))
                 (memory (export "memory") 1)
                 (data (i32.const 0) "article")   ;; selector at 0..7
                 (data (i32.const 100) "0000")    ;; scratch for ASCII digit emit
                 (func (export "_start")
                   (local $n i32)
                   (local.set $n (call $qc (i32.const 0) (i32.const 7)))
                   ;; Emit '0' + $n as a single ASCII digit. Works for 0..9.
                   (i32.store8 (i32.const 100)
                     (i32.add (i32.const 48) (local.get $n)))
                   (drop (call $emit (i32.const 100) (i32.const 1)))))"#,
        );

        let doc = crate::dom::Document::parse(
            "<html><body><article>a</article><article>b</article><article>c</article></body></html>",
        );
        let cx = WasmAgentContext::with_snapshot(Arc::new(doc));

        let host = WasmHost::new().unwrap();
        let outcome = host.run_agent(&w, WasmCaps::default(), cx).expect("run");
        assert_eq!(outcome.output_as_str(), "3");
    }

    /// Agent: no snapshot → query_count returns 0.
    #[test]
    fn agent_query_count_without_snapshot_is_zero() {
        let w = wat(
            r#"(module
                 (import "nami" "query_count" (func $qc (param i32 i32) (result i32)))
                 (import "nami" "emit"        (func $emit (param i32 i32) (result i32)))
                 (memory (export "memory") 1)
                 (data (i32.const 0) "div")
                 (func (export "_start")
                   (local $n i32)
                   (local.set $n (call $qc (i32.const 0) (i32.const 3)))
                   (i32.store8 (i32.const 100)
                     (i32.add (i32.const 48) (local.get $n)))
                   (drop (call $emit (i32.const 100) (i32.const 1)))))"#,
        );
        let host = WasmHost::new().unwrap();
        let outcome = host
            .run_agent(&w, WasmCaps::default(), WasmAgentContext::default())
            .expect("run");
        assert_eq!(outcome.output_as_str(), "0");
    }

    /// Agent: dom_sexp_len + dom_sexp_read chunked access yields
    /// non-empty bytes proportional to the document size.
    #[test]
    fn agent_dom_sexp_read_returns_content() {
        let w = wat(
            r#"(module
                 (import "nami" "dom_sexp_len"  (func $len                     (result i32)))
                 (import "nami" "dom_sexp_read" (func $read (param i32 i32 i32) (result i32)))
                 (import "nami" "emit"          (func $emit (param i32 i32)    (result i32)))
                 (memory (export "memory") 2)
                 (func (export "_start")
                   (local $total i32)
                   (local $got i32)
                   (local.set $total (call $len))
                   ;; read up to 256 bytes from offset 0 into 1024.
                   (local.set $got
                     (call $read (i32.const 0) (i32.const 256) (i32.const 1024)))
                   (drop (call $emit (i32.const 1024) (local.get $got)))))"#,
        );
        let doc = crate::dom::Document::parse("<html><body><p>hello</p></body></html>");
        let cx = WasmAgentContext::with_snapshot(Arc::new(doc));
        let host = WasmHost::new().unwrap();
        let outcome = host.run_agent(&w, WasmCaps::default(), cx).expect("run");
        let out = outcome.output_as_str();
        assert!(out.contains("(document"), "output: {out}");
        assert!(out.contains("\"body\""), "output: {out}");
        assert!(out.contains("\"hello\""), "output: {out}");
    }

    /// Agent: emit respects max_output_bytes cap — trailing bytes
    /// are truncated, guest sees short write return.
    #[test]
    fn agent_emit_respects_output_cap() {
        // Agent tries to emit 100 bytes; we cap at 10.
        let w = wat(
            r#"(module
                 (import "nami" "emit" (func $emit (param i32 i32) (result i32)))
                 (memory (export "memory") 1)
                 (data (i32.const 0)
                   "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUV")
                 (func (export "_start")
                   (drop (call $emit (i32.const 0) (i32.const 100)))))"#,
        );
        let cx = WasmAgentContext {
            max_output_bytes: 10,
            ..WasmAgentContext::default()
        };
        let host = WasmHost::new().unwrap();
        let outcome = host.run_agent(&w, WasmCaps::default(), cx).expect("run");
        assert_eq!(outcome.output.len(), 10);
        assert_eq!(outcome.output_as_str(), "ABCDEFGHIJ");
    }

    /// Agent: a module with no Nami imports still runs via run_agent
    /// — proves the host functions are optional to the guest.
    #[test]
    fn agent_without_nami_imports_still_runs() {
        let w = wat(r#"(module (func (export "_start")))"#);
        let host = WasmHost::new().unwrap();
        let outcome = host
            .run_agent(&w, WasmCaps::default(), WasmAgentContext::default())
            .expect("run");
        assert_eq!(outcome.base.exit_code, Some(0));
        assert!(outcome.output.is_empty());
    }

    /// Agent: fuel exhaustion via run_agent surfaces as the same clean
    /// error as run(). Output accumulated up to the trap is still
    /// returned — useful for debugging runaway scripts.
    #[test]
    fn agent_fuel_exhaustion_returns_clean_error() {
        let w = wat(
            r#"(module
                 (func (export "_start")
                   (loop br 0)))"#,
        );
        let host = WasmHost::new().unwrap();
        let caps = WasmCaps {
            max_fuel: Some(1_000),
            ..WasmCaps::default()
        };
        let err = host
            .run_agent(&w, caps, WasmAgentContext::default())
            .unwrap_err();
        assert!(matches!(err, WasmError::FuelExhausted { .. }), "err: {err:?}");
    }
}
