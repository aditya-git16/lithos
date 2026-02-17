use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use obsidian_util::binance_book_ticker::parse_binance_book_ticker_fast;
use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};

const TEST_JSON: &str =
    r#"{"u":400900217,"s":"BTCUSDT","b":"12345.67","B":"0.123","a":"12345.68","A":"0.456"}"#;

fn bench_parse_px_2dp(c: &mut Criterion) {
    let prices = ["12345.67", "0.01", "99999.99", "1.50", "42000.00"];
    let mut group = c.benchmark_group("parse_px_2dp");
    group.throughput(Throughput::Elements(1));
    for price in &prices {
        group.bench_with_input(*price, price, |b, &p| {
            b.iter(|| black_box(parse_px_2dp(black_box(p))));
        });
    }
    group.finish();
}

fn bench_parse_qty_3dp(c: &mut Criterion) {
    let qtys = ["0.123", "100.500", "0.001", "99.999"];
    let mut group = c.benchmark_group("parse_qty_3dp");
    group.throughput(Throughput::Elements(1));
    for qty in &qtys {
        group.bench_with_input(*qty, qty, |b, &q| {
            b.iter(|| black_box(parse_qty_3dp(black_box(q))));
        });
    }
    group.finish();
}

fn bench_fast_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("binance_parser");
    group.throughput(Throughput::Elements(1));

    group.bench_function("fast_parser", |b| {
        b.iter(|| black_box(parse_binance_book_ticker_fast(black_box(TEST_JSON))));
    });

    group.bench_function("sonic_rs", |b| {
        b.iter(|| {
            let dto: Result<obsidian_core::dto::binance::BinanceDto, _> =
                sonic_rs::from_slice(black_box(TEST_JSON.as_bytes()));
            black_box(dto.ok());
        });
    });

    group.finish();
}

fn bench_full_parse_chain(c: &mut Criterion) {
    c.bench_function("full_parse_chain", |b| {
        b.iter(|| {
            let view = parse_binance_book_ticker_fast(black_box(TEST_JSON)).unwrap();
            black_box(parse_px_2dp(view.b));
            black_box(parse_qty_3dp(view.b_qty));
            black_box(parse_px_2dp(view.a));
            black_box(parse_qty_3dp(view.a_qty));
        });
    });
}

criterion_group!(
    benches,
    bench_parse_px_2dp,
    bench_parse_qty_3dp,
    bench_fast_parser,
    bench_full_parse_chain,
);
criterion_main!(benches);
