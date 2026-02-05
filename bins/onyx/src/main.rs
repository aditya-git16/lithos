use lithos_events::TopOfBook;
use lithos_icc::BroadcastReader;
use std::time::{Duration, Instant};

fn main() {
    let path = "/tmp/lithos_md_bus";

    let mut r = BroadcastReader::<TopOfBook>::open(path)
        .expect("failed to open mmap ring (start obsidian first)");

    eprintln!("ONYX: attached to {path}. Reading...");

    let mut last = Instant::now();
    let mut count: u64 = 0;
    let mut last_mid: i64 = 0;

    loop {
        while let Some(ev) = r.try_read() {
            last_mid = ev.mid_ticks();
            count += 1;
        }

        if last.elapsed() >= Duration::from_secs(1) {
            eprintln!(
                "ONYX: read rate ~ {} ev/s | last_mid={} | overruns={}",
                count,
                last_mid,
                r.overruns()
            );
            count = 0;
            last = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(1));
    }
}
