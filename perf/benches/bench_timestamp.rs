use criterion::{Criterion, black_box, criterion_group, criterion_main};
use obsidian_util::timestamp::now_ns;
use std::time::Instant;

fn bench_now_ns(c: &mut Criterion) {
    c.bench_function("now_ns", |b| {
        b.iter(|| black_box(now_ns()));
    });
}

fn bench_instant_now(c: &mut Criterion) {
    c.bench_function("Instant::now", |b| {
        b.iter(|| black_box(Instant::now()));
    });
}

fn bench_clock_pair(c: &mut Criterion) {
    c.bench_function("now_ns pair (measure overhead)", |b| {
        b.iter(|| {
            let start = now_ns();
            let end = now_ns();
            black_box(end.saturating_sub(start));
        });
    });
}

criterion_group!(benches, bench_now_ns, bench_instant_now, bench_clock_pair);
criterion_main!(benches);
