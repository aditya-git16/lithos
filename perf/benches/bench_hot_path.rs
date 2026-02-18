//! Criterion benchmarks for both hot paths (obsidian + onyx).
//!
//! Individual steps + full e2e per path, so step times and e2e are measured
//! in the same run → coherent and connected. perf_report reads the resulting
//! criterion JSON for its display.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastReader, BroadcastWriter, RingConfig};
use lithos_perf::generate_replay_corpus;
use obsidian_engine::ObsidianProcessor;
use obsidian_util::binance_book_ticker::parse_binance_book_ticker_fast;
use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};
use onyx_core::MarketStateManager;

const CORPUS_SIZE: usize = 4096;
const RING_CAPACITY: usize = 65536;
const NUM_SYMBOLS: u16 = 256;

fn temp_shm(label: &str) -> String {
    format!("/tmp/lithos_bench_hp_{}_{}", label, std::process::id(),)
}

// ─── Obsidian group ─────────────────────────────────────────────────────────

fn bench_obsidian(c: &mut Criterion) {
    let corpus = generate_replay_corpus(CORPUS_SIZE);
    let mut group = c.benchmark_group("obsidian");

    // 1. parse_book_ticker_fast
    {
        let mut idx = 0usize;
        group.bench_function("parse_book_ticker_fast", |b| {
            b.iter(|| {
                let msg = &corpus[idx % corpus.len()];
                idx += 1;
                black_box(parse_binance_book_ticker_fast(msg))
            });
        });
    }

    // 2. parse_px_qty_x4
    {
        let view = parse_binance_book_ticker_fast(&corpus[0]).unwrap();
        let (bp, bq, ap, aq) = (view.b, view.b_qty, view.a, view.a_qty);
        group.bench_function("parse_px_qty_x4", |b| {
            b.iter(|| {
                black_box(parse_px_2dp(black_box(bp)));
                black_box(parse_qty_3dp(black_box(bq)));
                black_box(parse_px_2dp(black_box(ap)));
                black_box(parse_qty_3dp(black_box(aq)));
            });
        });
    }

    // 3. build_tob
    group.bench_function("build_tob", |b| {
        b.iter(|| {
            black_box(TopOfBook {
                ts_event_ns: 1234567890,
                symbol_id: SymbolId(1),
                bid_px_ticks: 1_234_567,
                bid_qty_lots: 1_500,
                ask_px_ticks: 1_234_568,
                ask_qty_lots: 2_300,
            });
        });
    });

    // 4. publish
    {
        let shm = temp_shm("obs_pub");
        BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(RING_CAPACITY))
            .expect("create ring");
        let mut writer = BroadcastWriter::<TopOfBook>::open(&shm).expect("open writer");
        let tob = TopOfBook {
            ts_event_ns: 0,
            symbol_id: SymbolId(1),
            bid_px_ticks: 1_234_567,
            bid_qty_lots: 1_500,
            ask_px_ticks: 1_234_568,
            ask_qty_lots: 2_300,
        };
        group.bench_function("publish", |b| {
            b.iter(|| writer.publish(black_box(tob)));
        });
        let _ = std::fs::remove_file(&shm);
    }

    // 5. process_text — full e2e, cycling 256 symbols
    {
        let shm = temp_shm("obs_e2e");
        BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(RING_CAPACITY))
            .expect("create ring");
        let mut proc = ObsidianProcessor::new(&shm, SymbolId(0)).expect("processor");
        let mut idx = 0usize;
        group.bench_function("process_text", |b| {
            b.iter(|| {
                let msg = &corpus[idx % corpus.len()];
                proc.symbol_id = SymbolId((idx % NUM_SYMBOLS as usize) as u16);
                idx += 1;
                black_box(proc.process_text(msg));
            });
        });
        let _ = std::fs::remove_file(&shm);
    }

    group.finish();
}

// ─── Onyx group ─────────────────────────────────────────────────────────────

fn bench_onyx(c: &mut Criterion) {
    let mut group = c.benchmark_group("onyx");

    // 1. try_read — pre-fill ring per batch to avoid refill contamination
    {
        let shm = temp_shm("onyx_read");
        BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(RING_CAPACITY))
            .expect("create ring");
        let mut writer = BroadcastWriter::<TopOfBook>::open(&shm).expect("open writer");
        let mut reader = BroadcastReader::<TopOfBook>::open(&shm).expect("open reader");

        const CHUNK: u64 = RING_CAPACITY as u64 / 2;

        group.bench_function("try_read", |b| {
            b.iter_custom(|iters| {
                let mut elapsed = std::time::Duration::ZERO;
                let mut done = 0u64;
                while done < iters {
                    let batch = (iters - done).min(CHUNK);
                    // Pre-fill outside timing
                    for i in 0..batch {
                        writer.publish(TopOfBook {
                            ts_event_ns: 0,
                            symbol_id: SymbolId(((done + i) % NUM_SYMBOLS as u64) as u16),
                            bid_px_ticks: 1_234_567,
                            bid_qty_lots: 1_500,
                            ask_px_ticks: 1_234_568,
                            ask_qty_lots: 2_300,
                        });
                    }
                    let start = std::time::Instant::now();
                    for _ in 0..batch {
                        black_box(reader.try_read());
                    }
                    elapsed += start.elapsed();
                    done += batch;
                }
                elapsed
            });
        });
        let _ = std::fs::remove_file(&shm);
    }

    // 2. update_market_state — 256 symbols cycling
    {
        let mut mgr = MarketStateManager::new();
        let mut sym_idx = 0u16;
        group.bench_function("update_market_state", |b| {
            b.iter(|| {
                let tob = TopOfBook {
                    ts_event_ns: 0,
                    symbol_id: SymbolId(sym_idx),
                    bid_px_ticks: 1_234_567,
                    bid_qty_lots: 1_500,
                    ask_px_ticks: 1_234_568,
                    ask_qty_lots: 2_300,
                };
                sym_idx = sym_idx.wrapping_add(1) % NUM_SYMBOLS;
                mgr.update_market_state_tob(black_box(&tob));
            });
        });
    }

    // 3. poll_event — mirrors production OnyxEngine::poll_events() exactly:
    //    while let Some(event) = try_read() { update; prefetch_next; spin_loop; }
    //    Uses iter_custom for accurate per-op timing.
    {
        let shm = temp_shm("onyx_poll");
        BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(RING_CAPACITY))
            .expect("create ring");
        let mut writer = BroadcastWriter::<TopOfBook>::open(&shm).expect("open writer");
        let mut reader = BroadcastReader::<TopOfBook>::open(&shm).expect("open reader");
        let mut mgr = MarketStateManager::new();

        group.bench_function("poll_event", |b| {
            // Batch pre-fill + read in chunks to prevent ring overflow when
            // criterion scales iters past RING_CAPACITY.
            const CHUNK: u64 = RING_CAPACITY as u64 / 2;

            b.iter_custom(|iters| {
                let mut elapsed = std::time::Duration::ZERO;
                let mut done = 0u64;
                while done < iters {
                    let batch = (iters - done).min(CHUNK);
                    for i in 0..batch {
                        writer.publish(TopOfBook {
                            ts_event_ns: 0,
                            symbol_id: SymbolId(((done + i) % NUM_SYMBOLS as u64) as u16),
                            bid_px_ticks: 1_234_567,
                            bid_qty_lots: 1_500,
                            ask_px_ticks: 1_234_568,
                            ask_qty_lots: 2_300,
                        });
                    }
                    let start = std::time::Instant::now();
                    for _ in 0..batch {
                        // Matches OnyxEngine::poll_events() body exactly:
                        // try_read → update → prefetch_next → spin_loop
                        if let Some(event) = reader.try_read() {
                            mgr.update_market_state_tob(&event);
                            reader.prefetch_next();
                            std::hint::spin_loop();
                        }
                    }
                    elapsed += start.elapsed();
                    done += batch;
                }
                elapsed
            });
        });
        let _ = std::fs::remove_file(&shm);
    }

    group.finish();
}

criterion_group!(benches, bench_obsidian, bench_onyx);
criterion_main!(benches);
