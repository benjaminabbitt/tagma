//! Criterion benches for the query engine (PLAN.md §9 / P1).
//!
//! Dataset is generated arithmetically from a fixed seed (a splitmix64
//! sequence) so every run is byte-for-byte reproducible — no external
//! `rand` dependency, no wall-clock or OS entropy.
//!
//! Shape (PLAN.md §9): 100k items, 10 tags each; keys drawn from a pool of
//! 20 names (4 of them numeric-valued); values drawn from a pool of 1000
//! distinct strings; ~10% of tags carry a namespace (one of 3); ~20% of
//! tags are valueless (bare).

use std::sync::OnceLock;

use criterion::{criterion_group, criterion_main, Criterion};
use tagma_core::Index;

const N_ITEMS: usize = 100_000;
const TAGS_PER_ITEM: usize = 10;
const N_VALUES: u64 = 1000;

const KEYS: [&str; 20] = [
    "status", "priority", "region", "owner", "env", "tier", "category", "team", "stage", "level",
    "type", "channel", "source", "cohort", "segment", "phase", "batch", "group", "zone", "rank",
];

/// Numeric-valued keys: a subset of `KEYS` whose generated values are plain
/// integers (as strings) instead of `v<n>` tokens, so numeric-range
/// benchmarks exercise realistic cardinalities.
const NUMERIC_KEYS: [&str; 4] = ["level", "batch", "zone", "rank"];

const NAMESPACES: [&str; 3] = ["geo", "prio", "meta"];

/// A splitmix64 generator: fast, dependency-free, and fully deterministic
/// for a fixed seed, so the benchmark dataset never varies across runs.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Builds the `<id> <tag> <tag>...` bulk-ingest lines for the benchmark
/// dataset (PLAN.md §9 shape), deterministically.
fn build_dataset_lines() -> Vec<String> {
    let mut rng = Rng(0xC0FF_EE00_1234_5678);
    let mut lines = Vec::with_capacity(N_ITEMS);
    for i in 0..N_ITEMS {
        let mut line = format!("item{i}");
        for _ in 0..TAGS_PER_ITEM {
            let r = rng.next();
            let key = KEYS[(r as usize) % KEYS.len()];
            let namespaced = (r / 20).is_multiple_of(10); // ~10% of tags
            let bare = (r >> 40).is_multiple_of(5); // ~20% of tags valueless

            line.push(' ');
            if namespaced {
                let ns = NAMESPACES[(r as usize / 7) % NAMESPACES.len()];
                line.push_str(ns);
                line.push(':');
            }
            line.push_str(key);
            if !bare {
                line.push('=');
                if NUMERIC_KEYS.contains(&key) {
                    line.push_str(&(r % N_VALUES).to_string());
                } else {
                    line.push('v');
                    line.push_str(&(r % N_VALUES).to_string());
                }
            }
        }
        lines.push(line);
    }
    lines
}

fn build_index(lines: &[String]) -> Index {
    let mut idx = Index::new();
    for line in lines {
        idx.add_line(line).expect("bench dataset line is valid");
    }
    idx
}

/// Cached dataset lines: generated once per `cargo bench` process and
/// shared by every benchmark that needs a pre-built index, so only the
/// `index_build` group pays the generation + parse cost inside `b.iter`.
fn dataset_lines() -> &'static Vec<String> {
    static LINES: OnceLock<Vec<String>> = OnceLock::new();
    LINES.get_or_init(build_dataset_lines)
}

fn shared_index() -> &'static Index {
    static IDX: OnceLock<Index> = OnceLock::new();
    IDX.get_or_init(|| build_index(dataset_lines()))
}

fn bench_index_build(c: &mut Criterion) {
    let lines = dataset_lines();
    let mut group = c.benchmark_group("index_build");
    // 100k items x 10 tags is expensive per sample; keep the sample count
    // small so the group finishes in a reasonable time.
    group.sample_size(10);
    group.bench_function("100k_items_x10_tags", |b| b.iter(|| build_index(lines)));
    group.finish();
}

fn bench_single_atom_queries(c: &mut Criterion) {
    let idx = shared_index();
    let mut group = c.benchmark_group("single_atom_query_100k");

    group.bench_function("bare_atom", |b| b.iter(|| idx.query("status").unwrap()));

    group.bench_function("valued_equality", |b| {
        b.iter(|| idx.query("priority=v500").unwrap())
    });

    group.bench_function("numeric_range", |b| {
        b.iter(|| idx.query("rank>500").unwrap())
    });

    group.finish();
}

/// The gate benchmark (PLAN.md §9): a mixed 8-atom boolean query over the
/// 100k-item index, combining a bare atom, `=`, `>`, `~`, a namespace-`*`
/// atom, a namespace-`+` atom, a value-`*` atom, and `!=`, joined with
/// `and`/`or`/`not`. Target: p95 < 5 ms.
fn bench_gate(c: &mut Criterion) {
    let idx = shared_index();
    let query = "status and priority=v500 and rank>500 and not category~v1.. \
                 and *:region=v10 or +:owner and env=* and tier!=v5";
    // Fail fast (at bench-build time, not mid-measurement) if the gate
    // query stops compiling under future grammar changes.
    idx.query(query).expect("gate query must compile and run");

    let mut group = c.benchmark_group("gate");
    group.bench_function("mixed_8atom_query_100k", |b| {
        b.iter(|| idx.query(query).unwrap())
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_index_build,
    bench_single_atom_queries,
    bench_gate
);
criterion_main!(benches);
