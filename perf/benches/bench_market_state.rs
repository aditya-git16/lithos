use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use lithos_events::{SymbolId, TopOfBook};
use obsidian_util::timestamp::now_ns;
use onyx_core::MarketStateManager;

fn make_tob(symbol: u16) -> TopOfBook {
    TopOfBook {
        ts_event_ns: now_ns(),
        symbol_id: SymbolId(symbol),
        bid_px_ticks: 1_234_567,
        bid_qty_lots: 1_500,
        ask_px_ticks: 1_234_568,
        ask_qty_lots: 2_300,
    }
}

fn bench_single_symbol(c: &mut Criterion) {
    let mut mgr = MarketStateManager::new();
    let tob = make_tob(1);

    let mut group = c.benchmark_group("market_state");
    group.throughput(Throughput::Elements(1));

    group.bench_function("update_single_symbol", |b| {
        b.iter(|| mgr.update_market_state_tob(black_box(&tob)));
    });

    group.finish();
}

fn bench_random_symbols(c: &mut Criterion) {
    let mut mgr = MarketStateManager::new();
    let tobs: Vec<TopOfBook> = (0..64u16).map(|i| make_tob(i)).collect();

    let mut group = c.benchmark_group("market_state");
    group.throughput(Throughput::Elements(1));

    let mut idx = 0usize;
    group.bench_function("update_64_symbols_cycling", |b| {
        b.iter(|| {
            mgr.update_market_state_tob(black_box(&tobs[idx % 64]));
            idx += 1;
        });
    });

    group.finish();
}

fn bench_sequential_symbols(c: &mut Criterion) {
    let mut mgr = MarketStateManager::new();
    let tobs: Vec<TopOfBook> = (0..256u16).map(|i| make_tob(i)).collect();

    let mut group = c.benchmark_group("market_state");
    group.throughput(Throughput::Elements(1));

    let mut idx = 0usize;
    group.bench_function("update_256_sequential", |b| {
        b.iter(|| {
            mgr.update_market_state_tob(black_box(&tobs[idx % 256]));
            idx += 1;
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_single_symbol,
    bench_random_symbols,
    bench_sequential_symbols,
);
criterion_main!(benches);
