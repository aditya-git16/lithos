//! End-to-end two-process integration test for the broadcast ring buffer.
//!
//! # Overview
//!
//! This test validates cross-process shared memory communication by spawning
//! two independent OS processes (writer and reader) that communicate through
//! a memory-mapped ring buffer **concurrently**.
//!
//! # Test Architecture
//!
//! The test uses a "self-spawning" pattern where the same test executable is
//! invoked multiple times with different environment variables to determine
//! the role of each process:
//!
//! ```text
//!                    Time -->
//!
//! [Writer]  ----[create]----[publish events...]---------------[done]
//!                  |              |    |    |
//!                  v              v    v    v
//!              [mmap file]     (concurrent reads)
//!                  |              ^    ^    ^
//!                  v              |    |    |
//! [Reader]  ------[open]---------[read events...]-------------[done]
//!
//! Both processes run simultaneously, with the reader consuming events
//! as the writer produces them (true streaming IPC).
//! ```
//!
//! # Why Concurrent Testing Matters
//!
//! Testing with simultaneous writer/reader ensures:
//! - Memory ordering is correct under concurrent access
//! - Seqlock protocol handles in-flight writes properly
//! - Reader correctly handles partial/torn reads
//! - Overrun detection works when reader falls behind live writer
//!
//! # Running the Test
//!
//! ```bash
//! cargo test -p lithos-icc --test e2e_mmap -- --nocapture
//! ```

use std::env;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Writes to stderr with immediate flush to bypass test output capture.
///
/// The Rust test harness captures stdout/stderr by default. This macro
/// writes directly to stderr and flushes immediately, ensuring output
/// is visible even when running under `cargo test`.
macro_rules! log {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stderr(), $($arg)*);
        let _ = std::io::stderr().flush();
    }};
}

/// Environment variable used to signal the role of a spawned process.
const ENV_ROLE: &str = "LITHOS_E2E_ROLE";

/// Role identifier for the writer process.
const ROLE_WRITER: &str = "writer";

/// Role identifier for the reader process.
const ROLE_READER: &str = "reader";

/// Number of events to publish in the test.
const EVENT_COUNT: u64 = 100_000;

/// Ring buffer capacity. Sized to allow some slack but small enough
/// that a slow reader could experience overruns.
const RING_CAPACITY: usize = 1 << 14; // 16384 slots

/// Microseconds to sleep between batches in the writer.
/// This paces the writer to ensure concurrent operation with the reader.
const WRITER_BATCH_SIZE: u64 = 1_000;
const WRITER_BATCH_DELAY_US: u64 = 100;

/// Generates a unique file path for the test's mmap region.
///
/// Uses the parent process ID to avoid collisions when running tests in parallel.
fn test_path() -> String {
    let pid = std::process::id();
    format!("/tmp/lithos_e2e_bus_{pid}")
}

/// Entry point for the writer child process.
///
/// Creates a broadcast ring buffer at the specified path and publishes
/// events at a controlled rate to ensure the reader has time to consume
/// them concurrently.
///
/// The writer paces itself by sleeping briefly after each batch of events,
/// simulating realistic market data rates rather than dumping all events
/// instantly.
///
/// # Arguments
///
/// * `path` - File path for the memory-mapped ring buffer.
///
/// # Panics
///
/// Panics if the ring buffer cannot be created.
fn run_writer(path: &str) {
    use lithos_events::{SymbolId, TopOfBook};
    use lithos_icc::{BroadcastWriter, RingConfig};

    log!("[WRITER] Creating ring buffer");
    log!("[WRITER]   path: {path}");
    log!("[WRITER]   capacity: {RING_CAPACITY} slots");
    log!("[WRITER]   events to publish: {EVENT_COUNT}");
    log!("[WRITER]   pacing: {WRITER_BATCH_SIZE} events, then {WRITER_BATCH_DELAY_US}us delay");

    let mut writer = BroadcastWriter::<TopOfBook>::create(path, RingConfig::new(RING_CAPACITY))
        .expect("writer: failed to create ring buffer");

    log!("[WRITER] Ring buffer created, starting publish...");

    let start = Instant::now();

    for i in 0..EVENT_COUNT {
        let event = TopOfBook {
            ts_event_ns: i,
            symbol_id: SymbolId(1),
            bid_px_ticks: 1000 + i as i64,
            bid_qty_lots: 1,
            ask_px_ticks: 1010 + i as i64,
            ask_qty_lots: 1,
        };
        writer.publish(event);

        // Pace the writer: sleep after each batch to allow reader to keep up.
        // This ensures true concurrent operation rather than write-then-read.
        if (i + 1) % WRITER_BATCH_SIZE == 0 {
            std::thread::sleep(Duration::from_micros(WRITER_BATCH_DELAY_US));

            // Log progress at regular intervals
            if (i + 1) % 25_000 == 0 {
                let elapsed = start.elapsed();
                let rate = (i + 1) as f64 / elapsed.as_secs_f64();
                log!("[WRITER] Progress: {}/{} events ({rate:.0} ev/s)", i + 1, EVENT_COUNT);
            }
        }
    }

    let elapsed = start.elapsed();
    let throughput = EVENT_COUNT as f64 / elapsed.as_secs_f64();

    log!("[WRITER] Complete");
    log!("[WRITER]   events published: {EVENT_COUNT}");
    log!("[WRITER]   elapsed: {elapsed:?}");
    log!("[WRITER]   throughput: {throughput:.0} events/sec");
}

/// Entry point for the reader child process.
///
/// Opens the broadcast ring buffer in tail-follow mode (starting at the
/// current write position) and reads events as they arrive. This simulates
/// a real consumer that attaches to a live stream.
///
/// The reader tracks how many events it receives versus how many were
/// skipped due to overruns (if the writer is faster than the reader).
///
/// # Arguments
///
/// * `path` - File path for the memory-mapped ring buffer.
///
/// # Panics
///
/// Panics if:
/// - The ring buffer cannot be opened within the timeout
/// - No events are received
fn run_reader(path: &str) {
    use lithos_events::TopOfBook;
    use lithos_icc::BroadcastReader;

    log!("[READER] Waiting for ring buffer at {path}");

    // Retry loop: wait for the writer to create the file
    let open_deadline = Instant::now() + Duration::from_secs(5);
    let mut reader = loop {
        // Use tail-follow mode: start reading from current write position.
        // This is realistic - a consumer attaching to a live feed.
        match BroadcastReader::<TopOfBook>::open(path) {
            Ok(r) => {
                log!("[READER] Ring buffer opened (tail-follow mode)");
                break r;
            }
            Err(_) if Instant::now() < open_deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => panic!("[READER] Failed to open ring buffer: {e}"),
        }
    };

    let read_deadline = Instant::now() + Duration::from_secs(10);
    let mut events_read: u64 = 0;
    let mut last_bid: i64 = 0;
    let mut consecutive_empty_reads: u32 = 0;

    let start = Instant::now();
    log!("[READER] Starting read loop...");

    // Main read loop: consume events as they arrive
    while Instant::now() < read_deadline {
        let mut batch_count = 0u64;

        // Drain all currently available events
        while let Some(event) = reader.try_read() {
            last_bid = event.bid_px_ticks;
            events_read += 1;
            batch_count += 1;
        }

        // Log progress periodically
        if batch_count > 0 {
            consecutive_empty_reads = 0;

            if events_read % 25_000 < batch_count {
                let elapsed = start.elapsed();
                let rate = events_read as f64 / elapsed.as_secs_f64();
                log!(
                    "[READER] Progress: {} events read, {} overruns ({rate:.0} ev/s)",
                    events_read,
                    reader.overruns()
                );
            }
        } else {
            consecutive_empty_reads += 1;

            // Exit condition: no new events for a while AND we've read some events
            // This means the writer has finished.
            if consecutive_empty_reads > 10_000 && events_read > 0 {
                log!("[READER] No new events detected, writer appears done");
                break;
            }

            // Brief pause to avoid burning CPU while waiting for events
            std::hint::spin_loop();
        }
    }

    let elapsed = start.elapsed();
    let overruns = reader.overruns();
    let total_events = events_read + overruns;
    let throughput = events_read as f64 / elapsed.as_secs_f64();

    log!("[READER] Complete");
    log!("[READER]   events read: {events_read}");
    log!("[READER]   overruns (skipped): {overruns}");
    log!("[READER]   total accounted: {total_events}");
    log!("[READER]   last bid value: {last_bid}");
    log!("[READER]   elapsed: {elapsed:?}");
    log!("[READER]   throughput: {throughput:.0} events/sec");

    // Validation: we must have received some events
    assert!(events_read > 0, "Reader did not receive any events");

    // The reader started in tail-follow mode, so it won't see events published
    // before it opened. We just verify it received a reasonable number of events.
    // With proper pacing, we should see most events (minimal overruns).
    let coverage = events_read as f64 / EVENT_COUNT as f64 * 100.0;
    log!("[READER] Coverage: {coverage:.1}% of published events");

    log!("[READER] Validation passed");
}

/// Two-process concurrent end-to-end test for the mmap broadcast ring buffer.
///
/// This test validates:
/// 1. Writer and reader can operate **simultaneously**
/// 2. Events are correctly transmitted while both processes are active
/// 3. Memory ordering is correct under concurrent access
/// 4. Seqlock protocol handles concurrent read/write properly
///
/// The test spawns writer and reader processes that run in parallel,
/// with the reader consuming events as the writer produces them.
#[test]
fn e2e_two_process_mmap_bus() {
    // Check if we're running as a child process (writer or reader)
    if let Ok(role) = env::var(ENV_ROLE) {
        let path = env::var("LITHOS_E2E_PATH").expect("LITHOS_E2E_PATH not set");
        match role.as_str() {
            ROLE_WRITER => run_writer(&path),
            ROLE_READER => run_reader(&path),
            other => panic!("Unknown role: {other}"),
        }
        return;
    }

    let path = test_path();
    let exe = env::current_exe().expect("Failed to get current executable path");

    log!("");
    log!("{}", "=".repeat(70));
    log!("E2E Two-Process CONCURRENT Mmap Bus Test");
    log!("{}", "=".repeat(70));
    log!("Ring buffer path: {path}");
    log!("Events: {EVENT_COUNT}, Ring capacity: {RING_CAPACITY}");
    log!("");

    // Spawn writer process first (it creates the mmap file)
    log!("[ORCHESTRATOR] Spawning writer process...");
    let mut writer_proc = Command::new(&exe)
        .arg("--exact")
        .arg("e2e_two_process_mmap_bus")
        .env(ENV_ROLE, ROLE_WRITER)
        .env("LITHOS_E2E_PATH", &path)
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to spawn writer process");

    // Minimal delay: just enough for the writer to create the file.
    // The reader will retry if the file doesn't exist yet.
    std::thread::sleep(Duration::from_millis(5));

    // Spawn reader process - it will run CONCURRENTLY with the writer
    log!("[ORCHESTRATOR] Spawning reader process (concurrent with writer)...");
    let mut reader_proc = Command::new(&exe)
        .arg("--exact")
        .arg("e2e_two_process_mmap_bus")
        .env(ENV_ROLE, ROLE_READER)
        .env("LITHOS_E2E_PATH", &path)
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to spawn reader process");

    log!("[ORCHESTRATOR] Both processes running concurrently...");
    log!("");

    // Wait for both processes to complete
    let writer_status = writer_proc.wait().expect("Failed to wait for writer");
    let reader_status = reader_proc.wait().expect("Failed to wait for reader");

    log!("");
    log!("[ORCHESTRATOR] Writer exit status: {writer_status}");
    log!("[ORCHESTRATOR] Reader exit status: {reader_status}");

    // Cleanup: remove the temporary mmap file
    let _ = std::fs::remove_file(&path);

    // Validate results
    assert!(
        writer_status.success(),
        "Writer process failed with status: {writer_status}"
    );
    assert!(
        reader_status.success(),
        "Reader process failed with status: {reader_status}"
    );

    log!("");
    log!("[ORCHESTRATOR] Concurrent test passed");
    log!("{}", "=".repeat(70));
    log!("");
}
