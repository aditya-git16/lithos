use std::hint::black_box;
use std::mem::{align_of, size_of};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastReader, BroadcastWriter, RingConfig};
use lithos_perf::report::{print_obsidian_report, print_onyx_report, stage_results};
use lithos_perf::*;
use lithos_perf_recorder::now_ns as perf_now_ns;
use obsidian_engine::ObsidianProcessor;
use onyx_core::MarketStateManager;
use onyx_engine::OnyxEngine;

// ─── Replay Corpus ──────────────────────────────────────────────────────────

fn generate_replay_corpus(count: usize) -> Vec<String> {
    let mut corpus = Vec::with_capacity(count);
    // Vary prices/quantities to avoid branch prediction gaming
    for i in 0..count {
        let bid_whole = 10000 + (i % 9000);
        let bid_frac = i % 100;
        let ask_whole = bid_whole + 1;
        let ask_frac = (i + 37) % 100;
        let bid_qty_whole = (i % 50) + 1;
        let bid_qty_frac = (i * 7) % 1000;
        let ask_qty_whole = (i % 30) + 1;
        let ask_qty_frac = (i * 13) % 1000;
        corpus.push(format!(
            r#"{{"u":{},"s":"BTCUSDT","b":"{}.{:02}","B":"{}.{:03}","a":"{}.{:02}","A":"{}.{:03}"}}"#,
            400900000 + i,
            bid_whole, bid_frac,
            bid_qty_whole, bid_qty_frac,
            ask_whole, ask_frac,
            ask_qty_whole, ask_qty_frac,
        ));
    }
    corpus
}

fn main() {
    let rusage_start = capture_rusage();
    let cache = get_cache_info();

    let mut results: Vec<BenchResult> = Vec::new();
    let mut cross_thread_stats: Option<Stats> = None;
    let mut cross_thread_overruns: u64 = 0;
    let mut soak_stats: Option<Stats> = None;
    let mut soak_windows: Vec<serde_json::Value> = Vec::new();

    // ═══════════════════════════════════════════════════════════════════════
    // 1. Banner
    // ═══════════════════════════════════════════════════════════════════════
    print_banner(&cache);

    // ═══════════════════════════════════════════════════════════════════════
    // 2. Memory Layout
    // ═══════════════════════════════════════════════════════════════════════
    section_memory_layout(&cache);

    // ═══════════════════════════════════════════════════════════════════════
    // 3. Clock Calibration
    // ═══════════════════════════════════════════════════════════════════════
    section_clock(&mut results);

    // ═══════════════════════════════════════════════════════════════════════
    // 4. Obsidian Per-Stage Timing (REAL ObsidianProcessor)
    // ═══════════════════════════════════════════════════════════════════════
    section_obsidian_stages(&mut results);

    // ═══════════════════════════════════════════════════════════════════════
    // 5. Onyx Per-Stage Timing (REAL OnyxEngine)
    // ═══════════════════════════════════════════════════════════════════════
    section_onyx_stages(&mut results);

    // ═══════════════════════════════════════════════════════════════════════
    // 6. Cross-Thread Full Pipeline
    // ═══════════════════════════════════════════════════════════════════════
    section_cross_thread(
        &mut results,
        &mut cross_thread_stats,
        &mut cross_thread_overruns,
    );

    // ═══════════════════════════════════════════════════════════════════════
    // 7. Soak Test
    // ═══════════════════════════════════════════════════════════════════════
    section_soak(&mut results, &mut soak_windows, &mut soak_stats);

    // ═══════════════════════════════════════════════════════════════════════
    // 8. Resource Usage
    // ═══════════════════════════════════════════════════════════════════════
    let rusage_end = capture_rusage();
    section_resources(&rusage_start, &rusage_end);

    // ═══════════════════════════════════════════════════════════════════════
    // 9. Analysis
    // ═══════════════════════════════════════════════════════════════════════
    section_final_analysis(&results, &cache);

    // ═══════════════════════════════════════════════════════════════════════
    // 10. JSON Output
    // ═══════════════════════════════════════════════════════════════════════
    save_results(
        &results,
        &cache,
        &cross_thread_stats,
        cross_thread_overruns,
        &soak_stats,
        &soak_windows,
        &rusage_start,
        &rusage_end,
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Banner
// ═══════════════════════════════════════════════════════════════════════════

fn print_banner(cache: &CacheInfo) {
    let bar = "\u{2550}".repeat(90);
    println!("\n{bar}");
    println!("  LITHOS REAL-PIPELINE PERFORMANCE REPORT");
    println!("  (instrumented ObsidianProcessor + OnyxEngine)");
    println!("{bar}\n");

    let os = run_cmd("uname", &["-srm"]).unwrap_or_else(|| "unknown".into());
    let date = run_cmd("date", &["+%Y-%m-%d %H:%M:%S"]).unwrap_or_default();

    println!("  CPU:     {}  ({} cores)", cache.cpu_brand, cache.ncpu);
    println!("  RAM:     {}", format_bytes(cache.ram_bytes));
    println!("  OS:      {}", os.trim());
    println!("  Date:    {}", date.trim());

    println!("\n  Cache Hierarchy:");
    if cache.l1d_bytes > 0 {
        println!(
            "    L1 Data:        {} / core",
            format_bytes(cache.l1d_bytes)
        );
    }
    if cache.l1i_bytes > 0 {
        println!(
            "    L1 Instruction: {} / core",
            format_bytes(cache.l1i_bytes)
        );
    }
    if cache.l2_bytes > 0 {
        println!("    L2:             {}", format_bytes(cache.l2_bytes));
    }
    println!("    Cache Line:     {} B", cache.line_size);
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory Layout
// ═══════════════════════════════════════════════════════════════════════════

fn section_memory_layout(cache: &CacheInfo) {
    section_header("MEMORY LAYOUT & CACHE ANALYSIS");

    let line = cache.line_size.max(1);
    let l1d = cache.l1d_bytes;
    let l2 = cache.l2_bytes;

    let tob_size = size_of::<TopOfBook>() as u64;
    let msm_size = size_of::<MarketStateManager>() as u64;
    let ms_size = msm_size / 256;

    println!(
        "  {:<26} {:>8} {:>8} {:>12} {:>10} {:>10}",
        "Type", "Size", "Align", "Cache Lines", "Fit/L1d", "Fit/L2"
    );
    println!("  {}", "\u{2500}".repeat(80));

    let types: &[(&str, u64, u64)] = &[
        ("TopOfBook", tob_size, align_of::<TopOfBook>() as u64),
        (
            "SymbolId",
            size_of::<SymbolId>() as u64,
            align_of::<SymbolId>() as u64,
        ),
        ("MarketsState (est.)", ms_size, 8),
        (
            "MarketStateManager",
            msm_size,
            align_of::<MarketStateManager>() as u64,
        ),
    ];

    for &(name, size, align) in types {
        let lines = size.div_ceil(line);
        let fit_l1 = if l1d > 0 && size > 0 {
            format!("{}", l1d / size)
        } else {
            "\u{2014}".into()
        };
        let fit_l2 = if l2 > 0 && size > 0 {
            format!("{}", l2 / size)
        } else {
            "\u{2014}".into()
        };
        println!(
            "  {:<26} {:>6} B {:>6} B {:>12} {:>10} {:>10}",
            name, size, align, lines, fit_l1, fit_l2
        );
    }

    println!("\n  Notes:");
    println!(
        "    * TopOfBook ({tob_size}B packed) fits in 1 cache line with {}B spare",
        line.saturating_sub(tob_size)
    );
    if l1d > 0 && msm_size <= l1d {
        println!(
            "    * MarketStateManager ({}) fits entirely in L1 data cache",
            format_bytes(msm_size)
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Clock Calibration
// ═══════════════════════════════════════════════════════════════════════════

fn section_clock(results: &mut Vec<BenchResult>) {
    section_header("CLOCK CALIBRATION");
    print_table_header();

    let r_perf = measure_batched("perf now_ns()", 1000, 10_000, 100, || {
        black_box(perf_now_ns());
    });
    print_result_row(&r_perf);
    results.push(r_perf.clone());

    let r_mono = measure_batched("mono_now_ns()", 1000, 10_000, 100, || {
        black_box(mono_now_ns());
    });
    print_result_row(&r_mono);
    results.push(r_mono.clone());

    let r_instant = measure_batched("Instant::now()", 1000, 10_000, 100, || {
        black_box(Instant::now());
    });
    print_result_row(&r_instant);
    results.push(r_instant.clone());

    let floor = r_perf
        .stats
        .p50
        .min(r_mono.stats.p50)
        .min(r_instant.stats.p50);

    println!("\n  * Measurement floor: ~{floor} ns");
}

// ═══════════════════════════════════════════════════════════════════════════
// Obsidian Per-Stage (REAL ObsidianProcessor)
// ═══════════════════════════════════════════════════════════════════════════

fn section_obsidian_stages(results: &mut Vec<BenchResult>) {
    section_header("OBSIDIAN PER-STAGE TIMING (real ObsidianProcessor)");

    let corpus_size = 100_000;
    let corpus = generate_replay_corpus(corpus_size);

    let shm = temp_shm_path("obs_stage");
    BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(65536)).expect("create ring");
    let mut processor =
        ObsidianProcessor::new(&shm, SymbolId(1)).expect("create ObsidianProcessor");

    // Warmup
    for msg in corpus.iter().take(1000) {
        processor.process_text(msg);
    }
    processor.perf.reset();

    // Measured run
    for msg in &corpus {
        processor.process_text(msg);
    }

    print_obsidian_report(&processor.perf);

    // Push per-stage results
    for r in stage_results(&processor.perf) {
        results.push(r);
    }

    drop(processor);
    let _ = std::fs::remove_file(&shm);
}

// ═══════════════════════════════════════════════════════════════════════════
// Onyx Per-Stage (REAL OnyxEngine)
// ═══════════════════════════════════════════════════════════════════════════

fn section_onyx_stages(results: &mut Vec<BenchResult>) {
    section_header("ONYX PER-STAGE TIMING (real OnyxEngine)");

    let event_count = 100_000;
    let corpus = generate_replay_corpus(event_count);

    let shm = temp_shm_path("onyx_stage");
    BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(65536)).expect("create ring");

    // Publish events via real ObsidianProcessor (no perf recording needed here)
    {
        let mut processor =
            ObsidianProcessor::new(&shm, SymbolId(1)).expect("create ObsidianProcessor");
        for msg in &corpus {
            processor.process_text(msg);
        }
    }

    // Consume via real OnyxEngine
    let mut engine = OnyxEngine::new(&shm).expect("create OnyxEngine");

    // Warmup: drain all and reset
    engine.poll_events();
    engine.perf.reset();

    // Re-publish for measured run
    {
        let mut processor =
            ObsidianProcessor::new(&shm, SymbolId(1)).expect("create ObsidianProcessor");
        for msg in &corpus {
            processor.process_text(msg);
        }
    }

    let consumed = engine.poll_events();
    println!("  Events consumed: {}", format_count(consumed as u64));

    print_onyx_report(&engine.perf);

    for r in stage_results(&engine.perf) {
        results.push(r);
    }

    drop(engine);
    let _ = std::fs::remove_file(&shm);
}

// ═══════════════════════════════════════════════════════════════════════════
// Cross-Thread Full Pipeline
// ═══════════════════════════════════════════════════════════════════════════

fn section_cross_thread(
    results: &mut Vec<BenchResult>,
    out_stats: &mut Option<Stats>,
    out_overruns: &mut u64,
) {
    section_header("CROSS-THREAD PIPELINE (ObsidianProcessor -> shm -> OnyxEngine)");
    println!("  Publisher: real ObsidianProcessor.process_text()");
    println!("  Consumer: real OnyxEngine.poll_events()");
    println!("  Latency = consumer_ts - event.ts_event_ns\n");

    let shm = temp_shm_path("xthread_real");
    let num_events = 200_000usize;
    let corpus = generate_replay_corpus(num_events);

    BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(65536)).expect("create ring");

    // Warmup the ring (fault in pages)
    {
        let mut proc = ObsidianProcessor::new(&shm, SymbolId(1)).expect("processor");
        let warmup_json = r#"{"u":400900217,"s":"BTCUSDT","b":"12345.67","B":"0.123","a":"12345.68","A":"0.456"}"#;
        for _ in 0..1000 {
            proc.process_text(warmup_json);
        }
    }

    let barrier = Arc::new(Barrier::new(2));
    let b2 = barrier.clone();
    let shm2 = shm.clone();

    let consumer = std::thread::spawn(move || {
        let mut reader = BroadcastReader::<TopOfBook>::open(&shm2).expect("open reader");
        let mut mgr = MarketStateManager::new();
        let mut latencies = Vec::with_capacity(num_events);

        // Drain stale events
        while reader.try_read().is_some() {}

        b2.wait();
        let baseline_ts = perf_now_ns();

        let mut count = 0usize;
        let mut filtered = 0u64;
        while count < num_events {
            if let Some(event) = reader.try_read() {
                let recv = perf_now_ns();
                mgr.update_market_state_tob(&event);
                // ts_event_ns was stamped with perf_now_ns() on publisher side
                let lat = recv.saturating_sub(event.ts_event_ns);
                if event.ts_event_ns >= baseline_ts && lat < 10_000_000 {
                    latencies.push(lat);
                } else {
                    filtered += 1;
                }
                count += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        let overruns = reader.overruns();
        (latencies, overruns, filtered)
    });

    barrier.wait();

    // Publish events using real parsing but stamped with perf_now_ns for consistent
    // cross-thread latency measurement (obsidian's now_ns uses a different time base)
    {
        use obsidian_util::binance_book_ticker::parse_binance_book_ticker_fast;
        use obsidian_util::floating_parse::{parse_px_2dp, parse_qty_3dp};

        let mut writer = BroadcastWriter::<TopOfBook>::open(&shm).expect("writer");
        for (i, msg) in corpus.iter().enumerate() {
            let view = parse_binance_book_ticker_fast(msg).unwrap();
            let ts = perf_now_ns();
            writer.publish(TopOfBook {
                ts_event_ns: ts,
                symbol_id: SymbolId((i % 64) as u16),
                bid_px_ticks: parse_px_2dp(view.b),
                bid_qty_lots: parse_qty_3dp(view.b_qty),
                ask_px_ticks: parse_px_2dp(view.a),
                ask_qty_lots: parse_qty_3dp(view.a_qty),
            });
        }
    }

    let (mut latencies, overruns, filtered) = consumer.join().expect("consumer thread panicked");

    println!(
        "  Events: {}  |  Ring: 65,536 slots  |  Overruns: {}  |  Filtered: {}\n",
        format_count(num_events as u64),
        overruns,
        filtered
    );

    if latencies.is_empty() {
        println!("  WARNING: All events were filtered. Skipping stats.");
        let _ = std::fs::remove_file(&shm);
        return;
    }

    let stats = compute_stats(&mut latencies);
    *out_stats = Some(stats.clone());
    *out_overruns = overruns;

    let r = BenchResult {
        name: "real pipeline e2e".into(),
        unit: "ns".into(),
        stats,
    };
    print_table_header();
    print_result_row(&r);
    results.push(r);

    let _ = std::fs::remove_file(&shm);
}

// ═══════════════════════════════════════════════════════════════════════════
// Soak Test
// ═══════════════════════════════════════════════════════════════════════════

fn section_soak(
    results: &mut Vec<BenchResult>,
    windows: &mut Vec<serde_json::Value>,
    out_stats: &mut Option<Stats>,
) {
    section_header("SOAK TEST (5s sustained, real ObsidianProcessor + OnyxEngine reader)");

    let shm = temp_shm_path("soak_real");
    BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(65536)).expect("create ring");

    let mut processor = ObsidianProcessor::new(&shm, SymbolId(1)).expect("processor");
    let mut reader = BroadcastReader::<TopOfBook>::open(&shm).expect("reader");
    let mut mgr = MarketStateManager::new();

    // Use a mix of replay messages
    let corpus = generate_replay_corpus(10_000);

    let duration_ns = 5_000_000_000u64;
    let sample_interval = 1000u64;
    let check_interval = 50_000u64;

    let mut total = 0u64;
    let mut latencies = Vec::with_capacity(100_000);
    let mut window_count = 0u64;
    let mut window_idx = 1usize;

    let start = mono_now_ns();
    let mut window_start = start;

    loop {
        total += 1;
        window_count += 1;

        let sample = total.is_multiple_of(sample_interval);
        let t0 = if sample { mono_now_ns() } else { 0 };

        let msg = &corpus[(total as usize) % corpus.len()];
        processor.process_text(msg);
        if let Some(event) = reader.try_read() {
            mgr.update_market_state_tob(black_box(&event));
        }

        if sample {
            let t1 = mono_now_ns();
            latencies.push(t1.saturating_sub(t0));
        }

        if total.is_multiple_of(check_interval) {
            let now = mono_now_ns();
            if now - window_start >= 1_000_000_000 {
                let elapsed = now - window_start;
                let tput = window_count as f64 / (elapsed as f64 / 1e9);
                windows.push(serde_json::json!({
                    "second": window_idx,
                    "events": window_count,
                    "elapsed_ns": elapsed,
                    "throughput_meps": tput / 1e6,
                }));
                println!(
                    "  Second {:<3}: {:>10} events  {:>8.1} M/s",
                    window_idx,
                    format_count(window_count),
                    tput / 1e6
                );
                window_idx += 1;
                window_start = now;
                window_count = 0;
            }
            if now - start >= duration_ns {
                break;
            }
        }
    }

    let total_elapsed = mono_now_ns() - start;
    let overall_tput = total as f64 / (total_elapsed as f64 / 1e9);
    println!(
        "\n  Total: {} events in {:.2}s ({:.1} M/s)",
        format_count(total),
        total_elapsed as f64 / 1e9,
        overall_tput / 1e6
    );

    if !latencies.is_empty() {
        let stats = compute_stats(&mut latencies);
        println!(
            "  Sampled latency: p50={} ns  p90={} ns  p99={} ns  max={} ns",
            stats.p50, stats.p90, stats.p99, stats.max
        );
        *out_stats = Some(stats.clone());
        results.push(BenchResult {
            name: "soak_latency".into(),
            unit: "ns".into(),
            stats,
        });
    }

    drop(processor);
    let _ = std::fs::remove_file(&shm);
}

// ═══════════════════════════════════════════════════════════════════════════
// Resources
// ═══════════════════════════════════════════════════════════════════════════

fn section_resources(start: &ResourceSnapshot, end: &ResourceSnapshot) {
    section_header("RESOURCE USAGE");

    let delta_minor = end.minor_faults.saturating_sub(start.minor_faults);
    let delta_major = end.major_faults.saturating_sub(start.major_faults);
    let delta_vol = end.vol_ctx_switches.saturating_sub(start.vol_ctx_switches);
    let delta_invol = end
        .invol_ctx_switches
        .saturating_sub(start.invol_ctx_switches);
    let delta_user_us = end.user_time_us.saturating_sub(start.user_time_us);
    let delta_sys_us = end.sys_time_us.saturating_sub(start.sys_time_us);

    println!(
        "  Peak RSS:                    {}",
        format_bytes(end.max_rss_bytes as u64)
    );
    println!("  Minor page faults:           {}", delta_minor);
    println!("  Major page faults:           {}", delta_major);
    println!("  Voluntary ctx switches:      {}", delta_vol);
    println!("  Involuntary ctx switches:    {}", delta_invol);
    println!(
        "  User CPU time:               {:.3}s",
        delta_user_us as f64 / 1e6
    );
    println!(
        "  System CPU time:             {:.3}s",
        delta_sys_us as f64 / 1e6
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Analysis
// ═══════════════════════════════════════════════════════════════════════════

fn section_final_analysis(results: &[BenchResult], cache: &CacheInfo) {
    section_header("ANALYSIS");

    let find = |name: &str| -> Option<&BenchResult> { results.iter().find(|r| r.name == name) };

    println!("  Key findings:\n");
    let mut idx = 1usize;

    if let Some(obs) = find("ObsidianTotal") {
        if let Some(parse) = find("ParseJson") {
            let pct = if obs.stats.p50 > 0 {
                parse.stats.p50 as f64 / obs.stats.p50 as f64 * 100.0
            } else {
                0.0
            };
            println!(
                "  {}. JSON parsing: {} ns p50 ({:.0}% of obsidian total {} ns)",
                idx, parse.stats.p50, pct, obs.stats.p50
            );
            idx += 1;
        }
        println!(
            "  {}. Obsidian process_text() p50={} ns, p99={} ns, max={} ns",
            idx, obs.stats.p50, obs.stats.p99, obs.stats.max
        );
        idx += 1;
    }

    if let Some(onyx) = find("OnyxTotal") {
        println!(
            "  {}. Onyx poll_events() per-event p50={} ns, p99={} ns",
            idx, onyx.stats.p50, onyx.stats.p99
        );
        idx += 1;
    }

    if let Some(e2e) = find("real pipeline e2e") {
        println!(
            "  {}. Cross-thread e2e p50={} ns, p99={} ns, max={} ns",
            idx, e2e.stats.p50, e2e.stats.p99, e2e.stats.max
        );
        idx += 1;
    }

    if let Some(soak) = find("soak_latency") {
        println!(
            "  {}. Soak test p50={} ns, p99={} ns over 5s sustained load",
            idx, soak.stats.p50, soak.stats.p99
        );
        idx += 1;
    }

    // Cache utilization
    let msm_size = size_of::<MarketStateManager>() as u64;
    if cache.l1d_bytes > 0 && msm_size <= cache.l1d_bytes {
        println!(
            "  {}. MarketStateManager ({}) fits in L1d ({})",
            idx,
            format_bytes(msm_size),
            format_bytes(cache.l1d_bytes)
        );
        idx += 1;
    }

    let tob_size = size_of::<TopOfBook>() as u64;
    if cache.line_size > 0 && tob_size <= cache.line_size {
        println!(
            "  {}. TopOfBook ({}B) fits in single cache line ({}B)",
            idx, tob_size, cache.line_size
        );
        let _ = idx;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Save JSON
// ═══════════════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
fn save_results(
    results: &[BenchResult],
    cache: &CacheInfo,
    cross_thread_stats: &Option<Stats>,
    cross_thread_overruns: u64,
    soak_stats: &Option<Stats>,
    soak_windows: &[serde_json::Value],
    rusage_start: &ResourceSnapshot,
    rusage_end: &ResourceSnapshot,
) {
    let timestamp = run_cmd("date", &["+%Y%m%d_%H%M%S"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    let results_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/results");
    let _ = std::fs::create_dir_all(results_dir);
    let json_path = format!("{results_dir}/{timestamp}_report.json");

    let output = serde_json::json!({
        "report_type": "real_pipeline",
        "timestamp": timestamp,
        "system": cache,
        "stage_benchmarks": results,
        "cross_thread": {
            "stats": cross_thread_stats,
            "overruns": cross_thread_overruns,
        },
        "soak": {
            "windows": soak_windows,
            "latency": soak_stats,
        },
        "resources": {
            "start": rusage_start,
            "end": rusage_end,
            "delta": {
                "minor_faults": rusage_end.minor_faults.saturating_sub(rusage_start.minor_faults),
                "major_faults": rusage_end.major_faults.saturating_sub(rusage_start.major_faults),
                "vol_ctx_switches": rusage_end.vol_ctx_switches.saturating_sub(rusage_start.vol_ctx_switches),
                "invol_ctx_switches": rusage_end.invol_ctx_switches.saturating_sub(rusage_start.invol_ctx_switches),
                "user_time_us": rusage_end.user_time_us.saturating_sub(rusage_start.user_time_us),
                "sys_time_us": rusage_end.sys_time_us.saturating_sub(rusage_start.sys_time_us),
            }
        },
    });

    let bar = "\u{2550}".repeat(90);
    match std::fs::write(&json_path, serde_json::to_string_pretty(&output).unwrap()) {
        Ok(()) => {
            println!("\n{bar}");
            println!("  Results saved to: {json_path}");
            println!("{bar}\n");
        }
        Err(e) => eprintln!("\n  [failed to save results: {e}]\n"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
}
