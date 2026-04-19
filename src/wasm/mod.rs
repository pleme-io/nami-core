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

use std::time::{Duration, Instant};
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::{WasiCtxBuilder, pipe::MemoryOutputPipe};

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
        drop(store);
        drop(instance);

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
}
