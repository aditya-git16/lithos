use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use lithos_events::TopOfBook;
use lithos_icc::{BroadcastReader, BroadcastWriter, RingConfig};
use lithos_perf::{make_test_tob, temp_shm_path};

fn bench_publish(c: &mut Criterion) {
    let path = temp_shm_path("crit_pub");
    let cfg = RingConfig::new(65536);
    let mut writer =
        BroadcastWriter::<TopOfBook>::create(&path, cfg).expect("failed to create writer");
    let tob = make_test_tob();

    let mut group = c.benchmark_group("broadcast");
    group.throughput(Throughput::Elements(1));

    group.bench_function("publish", |b| {
        b.iter(|| writer.publish(black_box(tob)));
    });

    drop(group);
    drop(writer);
    let _ = std::fs::remove_file(&path);
}

fn bench_try_read_data(c: &mut Criterion) {
    let path = temp_shm_path("crit_read");
    let cfg = RingConfig::new(65536);
    let mut writer =
        BroadcastWriter::<TopOfBook>::create(&path, cfg).expect("failed to create writer");
    let mut reader = BroadcastReader::<TopOfBook>::open(&path).expect("failed to open reader");
    let tob = make_test_tob();

    let mut group = c.benchmark_group("broadcast");
    group.throughput(Throughput::Elements(1));

    group.bench_function("try_read (data)", |b| {
        b.iter_custom(|iters| {
            // Pre-fill
            for _ in 0..iters {
                writer.publish(tob);
            }
            let start = std::time::Instant::now();
            for _ in 0..iters {
                black_box(reader.try_read());
            }
            start.elapsed()
        });
    });

    drop(group);
    drop(writer);
    drop(reader);
    let _ = std::fs::remove_file(&path);
}

fn bench_try_read_empty(c: &mut Criterion) {
    let path = temp_shm_path("crit_empty");
    let cfg = RingConfig::new(65536);
    let _writer =
        BroadcastWriter::<TopOfBook>::create(&path, cfg).expect("failed to create writer");
    let mut reader = BroadcastReader::<TopOfBook>::open(&path).expect("failed to open reader");

    let mut group = c.benchmark_group("broadcast");
    group.throughput(Throughput::Elements(1));

    group.bench_function("try_read (empty)", |b| {
        b.iter(|| black_box(reader.try_read()));
    });

    drop(group);
    drop(_writer);
    drop(reader);
    let _ = std::fs::remove_file(&path);
}

fn bench_round_trip(c: &mut Criterion) {
    let path = temp_shm_path("crit_rt");
    let cfg = RingConfig::new(65536);
    let mut writer =
        BroadcastWriter::<TopOfBook>::create(&path, cfg).expect("failed to create writer");
    let mut reader = BroadcastReader::<TopOfBook>::open(&path).expect("failed to open reader");
    let tob = make_test_tob();

    let mut group = c.benchmark_group("broadcast");
    group.throughput(Throughput::Elements(1));

    group.bench_function("round_trip", |b| {
        b.iter(|| {
            writer.publish(black_box(tob));
            black_box(reader.try_read());
        });
    });

    drop(group);
    drop(writer);
    drop(reader);
    let _ = std::fs::remove_file(&path);
}

fn bench_throughput_capacities(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcast_capacity");
    group.throughput(Throughput::Elements(1));

    for &cap in &[1024usize, 4096, 16384, 65536] {
        let path = temp_shm_path(&format!("crit_cap_{cap}"));
        let cfg = RingConfig::new(cap);
        let mut writer =
            BroadcastWriter::<TopOfBook>::create(&path, cfg).expect("failed to create writer");
        let mut reader = BroadcastReader::<TopOfBook>::open(&path).expect("failed to open reader");
        let tob = make_test_tob();

        group.bench_function(format!("round_trip_cap_{cap}"), |b| {
            b.iter(|| {
                writer.publish(black_box(tob));
                black_box(reader.try_read());
            });
        });

        drop(writer);
        drop(reader);
        let _ = std::fs::remove_file(&path);
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_publish,
    bench_try_read_data,
    bench_try_read_empty,
    bench_round_trip,
    bench_throughput_capacities,
);
criterion_main!(benches);
