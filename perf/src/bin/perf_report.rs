use std::collections::BTreeMap;
use std::hint::black_box;
use std::mem::{align_of, size_of};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::time::Instant;

use lithos_events::{SymbolId, TopOfBook};
use lithos_icc::{BroadcastWriter, RingConfig};
use lithos_perf::*;
use lithos_perf_recorder::now_ns as perf_now_ns;
use obsidian_engine::ObsidianProcessor;
use obsidian_util::timestamp::now_ns as obs_now_ns;
use onyx_core::MarketStateManager;
use onyx_engine::OnyxEngine;

const NUM_SYMBOLS: u16 = 256;

fn main() {
    let rusage_start = capture_rusage();
    let cache = get_cache_info();

    let mut results: Vec<BenchResult> = Vec::new();
    let mut cross_thread_stats: Option<Stats> = None;
    let mut cross_thread_overruns: u64 = 0;
    let mut cross_thread_filtered: u64 = 0;
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
    // 4. Criterion Hot Path Results (read from criterion JSON)
    // ═══════════════════════════════════════════════════════════════════════
    let criterion_dir = criterion_target_dir();
    let estimates = read_criterion_estimates(&criterion_dir);
    section_criterion_paths(&estimates);

    // ═══════════════════════════════════════════════════════════════════════
    // 5. Cross-Thread Pipeline (measured e2e)
    // ═══════════════════════════════════════════════════════════════════════
    section_pipeline_summary(
        &estimates,
        &mut results,
        &mut cross_thread_stats,
        &mut cross_thread_overruns,
        &mut cross_thread_filtered,
    );

    // ═══════════════════════════════════════════════════════════════════════
    // 6. Soak Test
    // ═══════════════════════════════════════════════════════════════════════
    section_soak(&mut results, &mut soak_windows, &mut soak_stats);

    // ═══════════════════════════════════════════════════════════════════════
    // 7. Resource Usage
    // ═══════════════════════════════════════════════════════════════════════
    let rusage_end = capture_rusage();
    section_resources(&rusage_start, &rusage_end);

    // ═══════════════════════════════════════════════════════════════════════
    // 8. JSON Output
    // ═══════════════════════════════════════════════════════════════════════
    save_results(
        &results,
        &cache,
        &estimates,
        &cross_thread_stats,
        cross_thread_overruns,
        cross_thread_filtered,
        &soak_stats,
        &soak_windows,
        &rusage_start,
        &rusage_end,
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Criterion target directory
// ═══════════════════════════════════════════════════════════════════════════

fn criterion_target_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = perf/, criterion output is in <workspace>/target/criterion
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .parent()
        .unwrap()
        .join("target")
        .join("criterion")
}

// ═══════════════════════════════════════════════════════════════════════════
// Banner
// ═══════════════════════════════════════════════════════════════════════════

fn print_banner(cache: &CacheInfo) {
    let bar = "\u{2550}".repeat(90);
    println!("\n{bar}");
    println!("  LITHOS PERFORMANCE REPORT");
    println!("  criterion micro + cross-thread e2e + soak");
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
    println!("  * All timings below use batched amortisation (10k ops/batch) for ~1ns accuracy");
}

// ═══════════════════════════════════════════════════════════════════════════
// Criterion Hot Path Display (reads JSON from criterion runs)
// ═══════════════════════════════════════════════════════════════════════════

fn section_criterion_paths(estimates: &BTreeMap<String, CriterionEstimate>) {
    if estimates.is_empty() {
        section_header("CRITERION HOT PATH RESULTS");
        println!(
            "  No criterion data found. Run: cargo bench -p lithos-perf --bench bench_hot_path"
        );
        return;
    }

    // Obsidian path
    let obs_steps: &[(&str, &str)] = &[
        ("obsidian/parse_book_ticker_fast", "parse_book_ticker_fast"),
        ("obsidian/parse_px_qty_x4", "parse_px_qty_x4"),
        ("obsidian/build_tob", "build_tob"),
        ("obsidian/publish", "publish"),
    ];
    let obs_e2e = ("obsidian/process_text", "process_text");

    print_criterion_path(
        "OBSIDIAN HOT PATH",
        "WebSocket JSON \u{2192} parse \u{2192} build \u{2192} publish",
        obs_steps,
        obs_e2e,
        estimates,
    );

    // Onyx path
    let onyx_steps: &[(&str, &str)] = &[
        ("onyx/try_read", "try_read"),
        ("onyx/update_market_state", "update_market_state"),
    ];
    let onyx_e2e = ("onyx/poll_event", "poll_event");

    print_criterion_path(
        "ONYX HOT PATH",
        "read \u{2192} state update \u{2192} prefetch",
        onyx_steps,
        onyx_e2e,
        estimates,
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Cross-Thread Pipeline — criterion medians + measured e2e
// ═══════════════════════════════════════════════════════════════════════════

fn section_pipeline_summary(
    estimates: &BTreeMap<String, CriterionEstimate>,
    results: &mut Vec<BenchResult>,
    out_stats: &mut Option<Stats>,
    out_overruns: &mut u64,
    out_filtered: &mut u64,
) {
    let shm = temp_shm_path("xthread");
    let num_events = 200_000usize;
    let corpus = generate_replay_corpus(num_events);

    BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(65536)).expect("create ring");

    // Warmup using production ObsidianProcessor
    {
        let mut proc = ObsidianProcessor::new(&shm, SymbolId(0)).expect("processor");
        let warmup_json = r#"{"u":400900217,"s":"BTCUSDT","b":"12345.67","B":"0.123","a":"12345.68","A":"0.456"}"#;
        for _ in 0..1000 {
            proc.process_text(warmup_json);
        }
    }

    let barrier = Arc::new(Barrier::new(2));
    let b2 = barrier.clone();
    let shm2 = shm.clone();

    let consumer = std::thread::spawn(move || {
        set_thread_affinity(1); // Consumer on core 1
        let mut engine = OnyxEngine::new(&shm2).expect("onyx engine");
        let mut latencies = Vec::with_capacity(num_events);
        // Drain stale data
        while engine.reader.try_read().is_some() {}

        b2.wait();
        // Use the same clock as the producer (ObsidianProcessor::process_text
        // stamps via obsidian_util::timestamp::now_ns = clock_gettime CLOCK_MONOTONIC).
        let baseline_ts = obs_now_ns();

        let mut count = 0usize;
        let mut filtered = 0u64;
        while count < num_events {
            if let Some(event) = engine.reader.try_read() {
                // Production OnyxEngine::process_event + prefetch + spin_loop
                engine.market_state_manager.update_market_state_tob(&event);
                engine.reader.prefetch_next();
                core::hint::spin_loop();
                // Timestamp AFTER all consumer work — aligned with poll_event criterion bench
                // Uses same clock as producer (clock_gettime CLOCK_MONOTONIC)
                let recv = obs_now_ns();
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
        let overruns = engine.reader.overruns();
        (latencies, overruns, filtered)
    });

    barrier.wait();
    set_thread_affinity(0); // Producer on core 0

    // Producer: ObsidianProcessor::process_text with cycling symbol_id (production path)
    {
        let mut proc = ObsidianProcessor::new(&shm, SymbolId(0)).expect("processor");
        for (i, msg) in corpus.iter().enumerate() {
            proc.symbol_id = SymbolId((i % NUM_SYMBOLS as usize) as u16);
            proc.process_text(msg);
        }
    }

    let (mut latencies, overruns, filtered) = consumer.join().expect("consumer thread panicked");
    let _ = std::fs::remove_file(&shm);

    // ── Display ──
    println!("\n{}", "─".repeat(90));
    println!("  CROSS-THREAD PIPELINE  (Obsidian thread \u{2192} mmap ring \u{2192} Onyx thread)");
    println!("{}\n", "─".repeat(90));

    // Get criterion medians for single-thread sum
    let obs_median = estimates
        .get("obsidian/process_text")
        .map(|e| e.median_ns)
        .unwrap_or(0.0);
    let onyx_median = estimates
        .get("onyx/poll_event")
        .map(|e| e.median_ns)
        .unwrap_or(0.0);
    let sum_ns = obs_median + onyx_median;

    println!(
        "  {:<44} {:>10}",
        "Obsidian  process_text (criterion)",
        format_ns(obs_median),
    );
    println!(
        "  {:<44} {:>10}",
        "Onyx      poll_event   (criterion)",
        format_ns(onyx_median),
    );
    println!("  {:<44} {:>10}", "Single-thread sum", format_ns(sum_ns),);
    println!("  {}", "─".repeat(80));

    if !latencies.is_empty() {
        let stats = compute_stats(&mut latencies);
        *out_stats = Some(stats.clone());
        *out_overruns = overruns;
        *out_filtered = filtered;

        println!(
            "  {:<25} {:>10} {:>10} {:>10} {:>10}",
            "", "p50", "p99", "p99.9", "max"
        );
        println!(
            "  {:<25} {:>10} {:>10} {:>10} {:>10}",
            "Cross-thread e2e",
            format_ns(stats.p50 as f64),
            format_ns(stats.p99 as f64),
            format_ns(stats.p999 as f64),
            format_ns(stats.max as f64),
        );
        println!("  {}", "─".repeat(80));

        let ipc = (stats.p50 as f64) - sum_ns;
        println!(
            "  {:<44} {:>10}   (e2e p50 \u{2212} sum, core\u{2192}core)",
            "IPC cache-coherency overhead",
            format_ns(ipc),
        );

        println!(
            "\n  {}K events | {} symbols | {} overruns | {} filtered",
            num_events / 1000,
            NUM_SYMBOLS,
            overruns,
            filtered,
        );

        results.push(BenchResult {
            name: "pipeline e2e".into(),
            unit: "ns".into(),
            stats,
        });
    } else {
        println!("  WARNING: Cross-thread measurement returned no data.");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Soak Test — operational sanity gate
// Catches: thermal/freq drift, throughput stability, tail growth, regressions
// ═══════════════════════════════════════════════════════════════════════════

fn section_soak(
    results: &mut Vec<BenchResult>,
    windows: &mut Vec<serde_json::Value>,
    out_stats: &mut Option<Stats>,
) {
    section_header("SOAK TEST (5s sustained, 256 symbols)");

    let shm = temp_shm_path("soak_real");
    BroadcastWriter::<TopOfBook>::create(&shm, RingConfig::new(65536)).expect("create ring");

    // Both sides use production code paths
    let mut proc = ObsidianProcessor::new(&shm, SymbolId(0)).expect("processor");
    let mut engine = OnyxEngine::new(&shm).expect("onyx engine");

    let corpus = generate_replay_corpus(10_000);

    // ── Warmup: fill caches, fault pages, stabilize frequency ──
    for i in 0..100_000u64 {
        let msg = &corpus[(i as usize) % corpus.len()];
        proc.symbol_id = SymbolId((i as u16) % NUM_SYMBOLS);
        proc.process_text(msg);
        engine.poll_events();
    }

    let duration_ns = 5_000_000_000u64;
    let sample_interval = 1000u64;
    let check_interval = 50_000u64;

    let mut total = 0u64;
    let mut all_latencies = Vec::with_capacity(100_000);
    let mut window_latencies: Vec<u64> = Vec::with_capacity(20_000);
    let mut window_count = 0u64;
    let mut window_idx = 1usize;

    let start = mono_now_ns();
    let mut window_start = start;

    loop {
        total += 1;
        window_count += 1;

        let sample = total.is_multiple_of(sample_interval);
        let t0 = if sample { mono_now_ns() } else { 0 };

        // Producer: production ObsidianProcessor::process_text with cycling symbols
        let msg = &corpus[(total as usize) % corpus.len()];
        proc.symbol_id = SymbolId(((total as usize) % NUM_SYMBOLS as usize) as u16);
        proc.process_text(msg);

        // Consumer: production OnyxEngine::poll_events()
        engine.poll_events();

        if sample {
            let t1 = mono_now_ns();
            let lat = t1.saturating_sub(t0);
            all_latencies.push(lat);
            window_latencies.push(lat);
        }

        if total.is_multiple_of(check_interval) {
            let now = mono_now_ns();
            if now - window_start >= 1_000_000_000 {
                let elapsed = now - window_start;
                let tput = window_count as f64 / (elapsed as f64 / 1e9);

                // Per-window latency stats for tail-growth detection
                let (wp50, wp99, wmax) = if !window_latencies.is_empty() {
                    let mut wl = std::mem::take(&mut window_latencies);
                    let ws = compute_stats(&mut wl);
                    (ws.p50, ws.p99, ws.max)
                } else {
                    (0, 0, 0)
                };

                windows.push(serde_json::json!({
                    "second": window_idx,
                    "events": window_count,
                    "elapsed_ns": elapsed,
                    "throughput_meps": tput / 1e6,
                    "latency_p50_ns": wp50,
                    "latency_p99_ns": wp99,
                    "latency_max_ns": wmax,
                }));
                println!(
                    "  Second {:<3}: {:>10} events  {:>8.1} M/s  p50={:>4} ns  p99={:>4} ns  max={:>6} ns",
                    window_idx,
                    format_count(window_count),
                    tput / 1e6,
                    wp50, wp99, wmax,
                );
                window_idx += 1;
                window_start = now;
                window_count = 0;
                window_latencies = Vec::with_capacity(20_000);
            }
            if now - start >= duration_ns {
                break;
            }
        }
    }

    let total_elapsed = mono_now_ns() - start;
    let overall_tput = total as f64 / (total_elapsed as f64 / 1e9);
    let overruns = engine.reader.overruns();

    println!(
        "\n  Total: {} events in {:.2}s ({:.1} M/s) | {} overruns",
        format_count(total),
        total_elapsed as f64 / 1e9,
        overall_tput / 1e6,
        overruns,
    );

    if !all_latencies.is_empty() {
        let stats = compute_stats(&mut all_latencies);
        println!(
            "  Aggregate: p50={} ns  p90={} ns  p99={} ns  p99.9={} ns  max={} ns",
            stats.p50, stats.p90, stats.p99, stats.p999, stats.max
        );

        // Throughput stability (CV%)
        if windows.len() >= 2 {
            let tputs: Vec<f64> = windows
                .iter()
                .filter_map(|w| w.get("throughput_meps").and_then(|v| v.as_f64()))
                .collect();
            if !tputs.is_empty() {
                let mean = tputs.iter().sum::<f64>() / tputs.len() as f64;
                let var =
                    tputs.iter().map(|&t| (t - mean) * (t - mean)).sum::<f64>() / tputs.len() as f64;
                let cv = if mean > 0.0 {
                    var.sqrt() / mean * 100.0
                } else {
                    0.0
                };
                println!("  Throughput CV: {cv:.2}%");
            }
        }

        *out_stats = Some(stats.clone());
        results.push(BenchResult {
            name: "soak_latency".into(),
            unit: "ns".into(),
            stats,
        });
    }

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
// Save JSON — includes criterion data
// ═══════════════════════════════════════════════════════════════════════════

#[allow(clippy::too_many_arguments)]
fn save_results(
    results: &[BenchResult],
    cache: &CacheInfo,
    criterion_estimates: &BTreeMap<String, CriterionEstimate>,
    cross_thread_stats: &Option<Stats>,
    cross_thread_overruns: u64,
    cross_thread_filtered: u64,
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

    // Convert criterion estimates to serializable format
    let crit_json: Vec<&CriterionEstimate> = criterion_estimates.values().collect();

    let output = serde_json::json!({
        "report_type": "pipeline",
        "timestamp": timestamp,
        "system": cache,
        "stage_benchmarks": results,
        "criterion_benchmarks": crit_json,
        "cross_thread": {
            "stats": cross_thread_stats,
            "overruns": cross_thread_overruns,
            "filtered": cross_thread_filtered,
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

/// Hint the OS scheduler to run this thread on a distinct core.
/// macOS: uses thread_affinity_policy (hint, not hard pin).
/// Linux: uses sched_setaffinity (hard pin).
fn set_thread_affinity(tag: usize) {
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct ThreadAffinityPolicy {
            affinity_tag: i32,
        }
        const THREAD_AFFINITY_POLICY: u32 = 4;
        unsafe extern "C" {
            fn mach_thread_self() -> u32;
            fn thread_policy_set(
                thread: u32,
                flavor: u32,
                policy_info: *const i32,
                count: u32,
            ) -> i32;
        }
        unsafe {
            let policy = ThreadAffinityPolicy {
                affinity_tag: tag as i32 + 1,
            };
            thread_policy_set(
                mach_thread_self(),
                THREAD_AFFINITY_POLICY,
                &policy as *const _ as *const i32,
                1,
            );
        }
    }
    #[cfg(target_os = "linux")]
    {
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_SET(tag, &mut set);
            libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = tag;
    }
}

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
