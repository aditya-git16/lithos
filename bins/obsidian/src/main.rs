use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastWriter, RingConfig};
use std::time::{Duration, Instant};

fn now_ns() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    t.as_nanos() as u64
}

fn main() {
    let path = "/tmp/lithos_md_bus";
    let capacity = 1 << 16;

    let mut bus = BroadcastWriter::<TopOfBook>::create(path, RingConfig::new(capacity))
        .expect("failed to create mmap ring");

    eprintln!("OBSIDIAN: publishing TopOfBook to {path} (cap={capacity})");

    let mut bid = 100_000i64;

    let mut last = Instant::now();
    let mut count: u64 = 0;

    loop {
        bid += 1;
        let ask = bid + 10;

        let ev = TopOfBook {
            ts_event_ns: now_ns(),
            symbol_id: SymbolId(1),
            bid_px_ticks: bid,
            bid_qty_lots: 10,
            ask_px_ticks: ask,
            ask_qty_lots: 12,
        };

        bus.publish(ev);
        count += 1;

        if last.elapsed() >= Duration::from_secs(1) {
            eprintln!("OBSIDIAN: publish rate ~ {} ev/s", count);
            count = 0;
            last = Instant::now();
        }

        std::hint::spin_loop();
    }
}
