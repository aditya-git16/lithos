use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastReader, BroadcastWriter, RingConfig};
use lithos_perf::temp_shm_path;
use obsidian_util::binance_book_ticker::parse_binance_book_ticker_fast;
use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};
use obsidian_util::timestamp::now_ns;
use onyx_core::MarketStateManager;

const TEST_JSON: &str =
    r#"{"u":400900217,"s":"BTCUSDT","b":"12345.67","B":"0.123","a":"12345.68","A":"0.456"}"#;

fn bench_full_pipeline(c: &mut Criterion) {
    let path = temp_shm_path("crit_pipeline");
    let cfg = RingConfig::new(65536);
    let mut writer =
        BroadcastWriter::<TopOfBook>::create(&path, cfg).expect("failed to create writer");
    let mut reader = BroadcastReader::<TopOfBook>::open(&path).expect("failed to open reader");
    let mut mgr = MarketStateManager::new();

    let mut group = c.benchmark_group("pipeline");
    group.throughput(Throughput::Elements(1));

    group.bench_function("full_hot_path", |b| {
        b.iter(|| {
            // 1. Parse JSON
            let view = parse_binance_book_ticker_fast(black_box(TEST_JSON)).unwrap();

            // 2. Build TopOfBook
            let tob = TopOfBook {
                ts_event_ns: now_ns(),
                symbol_id: SymbolId(1),
                bid_px_ticks: parse_px_2dp(view.b),
                bid_qty_lots: parse_qty_3dp(view.b_qty),
                ask_px_ticks: parse_px_2dp(view.a),
                ask_qty_lots: parse_qty_3dp(view.a_qty),
            };

            // 3. Publish to ring buffer
            writer.publish(tob);

            // 4. Read from ring buffer
            let event = reader.try_read().unwrap();

            // 5. Update market state
            mgr.update_market_state_tob(black_box(&event));
        });
    });

    drop(group);
    drop(writer);
    drop(reader);
    let _ = std::fs::remove_file(&path);
}

criterion_group!(benches, bench_full_pipeline);
criterion_main!(benches);
