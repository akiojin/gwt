//! Worktree operations benchmarks

use criterion::{criterion_group, criterion_main, Criterion};

fn worktree_list_benchmark(c: &mut Criterion) {
    // TODO: Implement benchmark
    c.bench_function("worktree_list", |b| {
        b.iter(|| {
            // Placeholder
        })
    });
}

criterion_group!(benches, worktree_list_benchmark);
criterion_main!(benches);
