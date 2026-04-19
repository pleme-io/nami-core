//! Microbenchmarks for `storage::kv::Store`.
//!
//! Runs off `cargo bench`. Measures the hot paths: volatile set,
//! volatile get, journal replay from disk, compaction. Tracks
//! throughput as a function of entry count so we can spot
//! super-linear regressions at PR time.
//!
//! These are *benchmarks*, not invariants — the accompanying
//! `perf_*_under_*` unit tests in `src/storage/kv.rs` carry the
//! assertions.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use nami_core::storage::kv::{Store, StorageSpec};
use serde_json::json;

fn spec(name: &str) -> StorageSpec {
    StorageSpec {
        name: name.into(),
        path: None,
        ttl_seconds: None,
        description: None,
    }
}

fn persistent_spec(name: &str) -> StorageSpec {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("nami-bench-{name}-{ts}.log"));
    StorageSpec {
        name: name.into(),
        path: Some(path),
        ttl_seconds: None,
        description: None,
    }
}

fn bench_set_volatile(c: &mut Criterion) {
    let mut group = c.benchmark_group("set_volatile");
    for &n in &[100usize, 1_000, 10_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let s = Store::from_spec(&spec("b"));
                for i in 0..n {
                    s.set(format!("k{i}"), json!(i));
                }
                black_box(s.len());
            });
        });
    }
    group.finish();
}

fn bench_get_volatile(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_volatile");
    for &n in &[100usize, 1_000, 10_000] {
        let s = Store::from_spec(&spec("b"));
        for i in 0..n {
            s.set(format!("k{i}"), json!(i));
        }
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &s, |b, s| {
            b.iter(|| {
                for i in 0..n {
                    black_box(s.get(&format!("k{i}")));
                }
            });
        });
    }
    group.finish();
}

fn bench_replay(c: &mut Criterion) {
    let mut group = c.benchmark_group("replay_journal");
    for &n in &[100usize, 1_000, 10_000] {
        // Pre-populate on disk once.
        let sp = persistent_spec("replay");
        {
            let s = Store::from_spec(&sp);
            for i in 0..n {
                s.set(format!("k{i}"), json!(i));
            }
        }
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &sp, |b, sp| {
            b.iter(|| {
                let s = Store::from_spec(sp);
                black_box(s.len());
            });
        });
        if let Some(path) = sp.path.as_ref() {
            std::fs::remove_file(path).ok();
        }
    }
    group.finish();
}

fn bench_compact(c: &mut Criterion) {
    let mut group = c.benchmark_group("compact");
    for &n in &[100usize, 1_000, 10_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let sp = persistent_spec("compact");
                    let s = Store::from_spec(&sp);
                    // Worst case: rewrite the same key N times so
                    // the journal has N redundant entries to fold.
                    for i in 0..n {
                        s.set("k", json!(i));
                    }
                    (sp, s)
                },
                |(sp, s)| {
                    s.compact().unwrap();
                    if let Some(path) = sp.path.as_ref() {
                        std::fs::remove_file(path).ok();
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_prefix_keys(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefix_keys");
    for &n in &[100usize, 1_000, 10_000] {
        let s = Store::from_spec(&spec("prefix"));
        for i in 0..n {
            let prefix = if i % 10 == 0 { "user/" } else { "cookie/" };
            s.set(format!("{prefix}{i}"), json!(i));
        }
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &s, |b, s| {
            b.iter(|| {
                black_box(s.prefix_keys("user/"));
            });
        });
    }
    group.finish();
}

criterion_group!(
    storage_kv,
    bench_set_volatile,
    bench_get_volatile,
    bench_replay,
    bench_compact,
    bench_prefix_keys
);
criterion_main!(storage_kv);
