use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use obsidian_core::dto::BinanceDto;
use obsidian_util::binance_book_ticker::parse_binance_book_ticker_fast;
use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};
use sonic_rs::{from_slice, from_slice_unchecked};

const MSG: &str =
    r#"{"u":123,"s":"BTCUSDT","b":"12345.67","B":"0.123","a":"12345.68","A":"0.456"}"#;

fn bench_binance_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("binance_parse");
    group.throughput(Throughput::Elements(1));

    let bytes = MSG.as_bytes();

    group.bench_with_input(BenchmarkId::new("fast_fields_only", "bookTicker"), &MSG, |b, msg| {
        b.iter(|| {
            let v = parse_binance_book_ticker_fast(black_box(msg)).expect("fast parse failed");
            black_box(v.b.len() + v.b_qty.len() + v.a.len() + v.a_qty.len())
        });
    });

    group.bench_with_input(
        BenchmarkId::new("sonic_unchecked_fields_only", "bookTicker"),
        &bytes,
        |b, msg| {
            b.iter(|| {
                let v: BinanceDto =
                    unsafe { from_slice_unchecked(black_box(msg)).expect("sonic parse failed") };
                black_box(v.b.len() + v.b_qty.len() + v.a.len() + v.a_qty.len())
            });
        },
    );

    group.bench_with_input(
        BenchmarkId::new("sonic_checked_fields_only", "bookTicker"),
        &bytes,
        |b, msg| {
            b.iter(|| {
                let v: BinanceDto = from_slice(black_box(msg)).expect("sonic parse failed");
                black_box(v.b.len() + v.b_qty.len() + v.a.len() + v.a_qty.len())
            });
        },
    );

    group.bench_with_input(
        BenchmarkId::new("fast_plus_numeric", "bookTicker"),
        &MSG,
        |b, msg| {
            b.iter(|| {
                let v = parse_binance_book_ticker_fast(black_box(msg)).expect("fast parse failed");
                let bid = parse_px_2dp(v.b);
                let bid_qty = parse_qty_3dp(v.b_qty);
                let ask = parse_px_2dp(v.a);
                let ask_qty = parse_qty_3dp(v.a_qty);
                black_box((bid as u64) ^ (bid_qty as u64) ^ (ask as u64) ^ (ask_qty as u64))
            });
        },
    );

    group.bench_with_input(
        BenchmarkId::new("sonic_unchecked_plus_numeric", "bookTicker"),
        &bytes,
        |b, msg| {
            b.iter(|| {
                let v: BinanceDto =
                    unsafe { from_slice_unchecked(black_box(msg)).expect("sonic parse failed") };
                let bid = parse_px_2dp(v.b);
                let bid_qty = parse_qty_3dp(v.b_qty);
                let ask = parse_px_2dp(v.a);
                let ask_qty = parse_qty_3dp(v.a_qty);
                black_box((bid as u64) ^ (bid_qty as u64) ^ (ask as u64) ^ (ask_qty as u64))
            });
        },
    );

    group.finish();
}

criterion_group!(benches, bench_binance_parse);
criterion_main!(benches);
