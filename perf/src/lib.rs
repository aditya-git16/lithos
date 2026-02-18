use lithos_events::{SymbolId, TopOfBook};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;

// ─── Statistics ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Stats {
    pub min: u64,
    pub max: u64,
    pub mean: f64,
    pub median: u64,
    pub stddev: f64,
    pub p50: u64,
    pub p75: u64,
    pub p90: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub p9999: u64,
    pub count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BenchResult {
    pub name: String,
    pub unit: String,
    pub stats: Stats,
}

pub fn compute_stats(samples: &mut [u64]) -> Stats {
    assert!(!samples.is_empty(), "cannot compute stats on empty samples");
    samples.sort_unstable();

    let count = samples.len();
    let sum: u64 = samples.iter().sum();
    let mean = sum as f64 / count as f64;

    let variance = samples
        .iter()
        .map(|&x| {
            let diff = x as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / count as f64;
    let stddev = variance.sqrt();

    Stats {
        min: samples[0],
        max: samples[count - 1],
        mean,
        median: percentile_sorted(samples, 50.0),
        stddev,
        p50: percentile_sorted(samples, 50.0),
        p75: percentile_sorted(samples, 75.0),
        p90: percentile_sorted(samples, 90.0),
        p95: percentile_sorted(samples, 95.0),
        p99: percentile_sorted(samples, 99.0),
        p999: percentile_sorted(samples, 99.9),
        p9999: percentile_sorted(samples, 99.99),
        count,
    }
}

fn percentile_sorted(sorted: &[u64], pct: f64) -> u64 {
    let len = sorted.len();
    if len == 1 {
        return sorted[0];
    }
    let rank = (pct / 100.0 * len as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(len - 1);
    sorted[idx]
}

// ─── Measurement Harness ────────────────────────────────────────────────────

pub fn measure_batched<F: FnMut()>(
    name: &str,
    batches: usize,
    batch_size: usize,
    warmup: usize,
    mut f: F,
) -> BenchResult {
    for _ in 0..warmup * batch_size {
        f();
    }

    let mut samples = Vec::with_capacity(batches);
    for _ in 0..batches {
        let start = Instant::now();
        for _ in 0..batch_size {
            f();
        }
        let total = start.elapsed().as_nanos();
        let per_op = ((total + (batch_size as u128 / 2)) / batch_size as u128) as u64;
        samples.push(per_op.max(1));
    }

    BenchResult {
        name: name.to_string(),
        unit: "ns/op".to_string(),
        stats: compute_stats(&mut samples),
    }
}

// ─── Hardware Info ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheInfo {
    pub l1d_bytes: u64,
    pub l1i_bytes: u64,
    pub l2_bytes: u64,
    pub line_size: u64,
    pub ram_bytes: u64,
    pub cpu_brand: String,
    pub ncpu: u64,
}

pub fn get_cache_info() -> CacheInfo {
    let ncpu = std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(0);
    let ram_bytes = total_ram_bytes().unwrap_or(0);
    let cpu_brand = cpu_brand_string().unwrap_or_else(|| "unknown".into());
    let line_size =
        cacheline_bytes().unwrap_or_else(|| if cpu_brand.contains("Apple") { 128 } else { 64 });

    CacheInfo {
        l1d_bytes: l1d_cache_bytes().unwrap_or(0),
        l1i_bytes: l1i_cache_bytes().unwrap_or(0),
        l2_bytes: l2_cache_bytes().unwrap_or(0),
        line_size,
        ram_bytes,
        cpu_brand,
        ncpu,
    }
}

#[cfg(target_vendor = "apple")]
fn l1d_cache_bytes() -> Option<u64> {
    sysctl_u64("hw.perflevel0.l1dcachesize").or_else(|| sysctl_u64("hw.l1dcachesize"))
}

#[cfg(not(target_vendor = "apple"))]
fn l1d_cache_bytes() -> Option<u64> {
    None
}

#[cfg(target_vendor = "apple")]
fn l1i_cache_bytes() -> Option<u64> {
    sysctl_u64("hw.perflevel0.l1icachesize").or_else(|| sysctl_u64("hw.l1icachesize"))
}

#[cfg(not(target_vendor = "apple"))]
fn l1i_cache_bytes() -> Option<u64> {
    None
}

#[cfg(target_vendor = "apple")]
fn l2_cache_bytes() -> Option<u64> {
    sysctl_u64("hw.perflevel0.l2cachesize").or_else(|| sysctl_u64("hw.l2cachesize"))
}

#[cfg(not(target_vendor = "apple"))]
fn l2_cache_bytes() -> Option<u64> {
    None
}

#[cfg(target_vendor = "apple")]
fn cacheline_bytes() -> Option<u64> {
    sysctl_u64("hw.cachelinesize")
}

#[cfg(not(target_vendor = "apple"))]
fn cacheline_bytes() -> Option<u64> {
    None
}

#[cfg(target_vendor = "apple")]
fn total_ram_bytes() -> Option<u64> {
    sysctl_u64("hw.memsize")
}

#[cfg(not(target_vendor = "apple"))]
fn total_ram_bytes() -> Option<u64> {
    None
}

#[cfg(target_vendor = "apple")]
fn cpu_brand_string() -> Option<String> {
    sysctl_str("machdep.cpu.brand_string")
        .or_else(|| sysctl_str("hw.model"))
        .or_else(|| sysctl_str("hw.machine"))
}

#[cfg(not(target_vendor = "apple"))]
fn cpu_brand_string() -> Option<String> {
    None
}

#[cfg(target_vendor = "apple")]
fn sysctl_raw(key: &str) -> Option<Vec<u8>> {
    let c_key = CString::new(key).ok()?;
    let mut len: libc::size_t = 0;
    let rc = unsafe {
        libc::sysctlbyname(
            c_key.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let rc = unsafe {
        libc::sysctlbyname(
            c_key.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return None;
    }
    buf.truncate(len);
    Some(buf)
}

#[cfg(target_vendor = "apple")]
fn sysctl_u64(key: &str) -> Option<u64> {
    let bytes = sysctl_raw(key)?;
    match bytes.len() {
        8 => Some(u64::from_ne_bytes(bytes[..8].try_into().ok()?)),
        4 => Some(u32::from_ne_bytes(bytes[..4].try_into().ok()?) as u64),
        _ => None,
    }
}

#[cfg(not(target_vendor = "apple"))]
fn sysctl_u64(_key: &str) -> Option<u64> {
    None
}

#[cfg(target_vendor = "apple")]
fn sysctl_str(key: &str) -> Option<String> {
    let mut bytes = sysctl_raw(key)?;
    if bytes.last().copied() == Some(0) {
        let _ = bytes.pop();
    }
    String::from_utf8(bytes).ok().map(|s| s.trim().to_string())
}

#[cfg(not(target_vendor = "apple"))]
fn sysctl_str(_key: &str) -> Option<String> {
    None
}

// ─── Resource Usage ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct ResourceSnapshot {
    pub max_rss_bytes: i64,
    pub minor_faults: i64,
    pub major_faults: i64,
    pub vol_ctx_switches: i64,
    pub invol_ctx_switches: i64,
    pub user_time_us: i64,
    pub sys_time_us: i64,
}

pub fn capture_rusage() -> ResourceSnapshot {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    #[cfg(target_os = "linux")]
    let max_rss_bytes = usage.ru_maxrss * 1024;
    #[cfg(not(target_os = "linux"))]
    let max_rss_bytes = usage.ru_maxrss;
    ResourceSnapshot {
        max_rss_bytes,
        minor_faults: usage.ru_minflt,
        major_faults: usage.ru_majflt,
        vol_ctx_switches: usage.ru_nvcsw,
        invol_ctx_switches: usage.ru_nivcsw,
        user_time_us: usage.ru_utime.tv_sec * 1_000_000 + usage.ru_utime.tv_usec as i64,
        sys_time_us: usage.ru_stime.tv_sec * 1_000_000 + usage.ru_stime.tv_usec as i64,
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

pub fn temp_shm_path(label: &str) -> String {
    let pid = std::process::id();
    format!("/tmp/lithos_bench_{label}_{pid}")
}

pub fn make_test_tob() -> TopOfBook {
    TopOfBook {
        ts_event_ns: mono_now_ns(),
        symbol_id: SymbolId(1),
        bid_px_ticks: 1_234_567,
        bid_qty_lots: 1_500,
        ask_px_ticks: 1_234_568,
        ask_qty_lots: 2_300,
    }
}

pub const TEST_JSON: &str =
    r#"{"u":400900217,"s":"BTCUSDT","b":"12345.67","B":"0.123","a":"12345.68","A":"0.456"}"#;

#[inline(always)]
#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn mono_now_ns() -> u64 {
    static TIMEBASE: OnceLock<(u64, u64)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut info = libc::mach_timebase_info_data_t { numer: 0, denom: 0 };
        let rc = unsafe { libc::mach_timebase_info(&mut info) };
        if rc != 0 || info.denom == 0 {
            (1, 1)
        } else {
            (info.numer as u64, info.denom as u64)
        }
    });
    let t = unsafe { libc::mach_absolute_time() } as u128;
    ((t * numer as u128) / denom as u128) as u64
}

#[inline(always)]
#[cfg(not(target_os = "macos"))]
pub fn mono_now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    (ts.tv_sec as u64) * 1_000_000_000 + ts.tv_nsec as u64
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

pub fn print_result_row(r: &BenchResult) {
    println!(
        "  {:<30} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}  {}",
        r.name,
        r.stats.min,
        r.stats.p50,
        r.stats.p75,
        r.stats.p90,
        r.stats.p99,
        r.stats.p999,
        r.stats.max,
        r.unit,
    );
}

pub fn print_table_header() {
    println!(
        "  {:<30} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}  unit",
        "Benchmark", "min", "p50", "p75", "p90", "p99", "p99.9", "max",
    );
    println!("  {}", "─".repeat(100));
}

pub fn section_header(title: &str) {
    println!("\n{}", "─".repeat(90));
    println!("  {title}");
    println!("{}\n", "─".repeat(90));
}

// ─── Replay Corpus (shared by bench + report) ──────────────────────────────

pub fn generate_replay_corpus(count: usize) -> Vec<String> {
    let mut corpus = Vec::with_capacity(count);
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

// ─── Criterion JSON Reader ─────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CriterionEstimate {
    pub name: String,
    pub median_ns: f64,
    pub mean_ns: f64,
    pub stddev_ns: f64,
}

/// Read estimates.json from bench_hot_path's criterion groups only (obsidian/, onyx/).
/// Scoped to prevent stale artifacts from other bench files leaking in.
/// Returns a map of "group/bench_name" → CriterionEstimate.
pub fn read_criterion_estimates(criterion_dir: &Path) -> BTreeMap<String, CriterionEstimate> {
    let mut out = BTreeMap::new();
    // Only walk the two groups produced by bench_hot_path.rs
    for group in &["obsidian", "onyx"] {
        let group_dir = criterion_dir.join(group);
        if group_dir.is_dir() {
            walk_criterion_dir(&group_dir, &mut out);
        }
    }
    out
}

fn walk_criterion_dir(dir: &Path, out: &mut BTreeMap<String, CriterionEstimate>) {
    let estimates = dir.join("new").join("estimates.json");
    if estimates.is_file()
        && let Some(est) = parse_criterion_estimates(&estimates, dir)
    {
        out.insert(est.name.clone(), est);
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.file_name().is_none_or(|n| n != "report") {
                walk_criterion_dir(&path, out);
            }
        }
    }
}

fn parse_criterion_estimates(json_path: &Path, bench_dir: &Path) -> Option<CriterionEstimate> {
    let data = std::fs::read_to_string(json_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;

    let median_ns = v.get("median")?.get("point_estimate")?.as_f64()?;
    let mean_ns = v.get("mean")?.get("point_estimate")?.as_f64()?;
    let stddev_ns = v.get("std_dev")?.get("point_estimate")?.as_f64()?;

    // Build the name from the directory structure relative to criterion root.
    // e.g. target/criterion/obsidian/parse_book_ticker_fast/new/estimates.json
    // bench_dir = .../obsidian/parse_book_ticker_fast
    // We want "obsidian/parse_book_ticker_fast"
    let name = criterion_bench_name(bench_dir)?;

    Some(CriterionEstimate {
        name,
        median_ns,
        mean_ns,
        stddev_ns,
    })
}

fn criterion_bench_name(bench_dir: &Path) -> Option<String> {
    // Walk up from bench_dir to find "criterion" parent, then take relative path
    let mut parts = Vec::new();
    let mut cur = bench_dir;
    loop {
        let name = cur.file_name()?.to_str()?;
        if name == "criterion" {
            break;
        }
        parts.push(name.to_string());
        cur = cur.parent()?;
    }
    parts.reverse();
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

// ─── Criterion Display ─────────────────────────────────────────────────────

pub fn format_ns(ns: f64) -> String {
    let abs = ns.abs();
    if abs >= 1_000_000.0 {
        format!("{:.1} ms", ns / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("{:.1} us", ns / 1_000.0)
    } else {
        format!("{:.0} ns", ns)
    }
}

/// Print the criterion-based path table (obsidian or onyx).
pub fn print_criterion_path(
    title: &str,
    subtitle: &str,
    steps: &[(&str, &str)], // (criterion_key, display_name)
    e2e: (&str, &str),      // (criterion_key, display_name)
    estimates: &BTreeMap<String, CriterionEstimate>,
) {
    println!("\n{}", "─".repeat(90));
    println!("  {title}  ({subtitle})");
    println!("{}\n", "─".repeat(90));

    println!(
        "  {:<40} {:>10} {:>10} {:>10} {:>6}",
        "Step", "median", "mean", "stddev", "%"
    );
    println!("  {}", "─".repeat(80));

    let e2e_est = estimates.get(e2e.0);
    let e2e_median = e2e_est.map(|e| e.median_ns).unwrap_or(0.0);

    let mut sum_median = 0.0;
    for &(key, display) in steps {
        if let Some(est) = estimates.get(key) {
            let pct = if e2e_median > 0.0 {
                est.median_ns / e2e_median * 100.0
            } else {
                0.0
            };
            println!(
                "  {:<40} {:>10} {:>10} {:>10} {:>5.0}%",
                format!("{}()", display),
                format_ns(est.median_ns),
                format_ns(est.mean_ns),
                format_ns(est.stddev_ns),
                pct,
            );
            sum_median += est.median_ns;
        }
    }

    println!("  {}", "─".repeat(80));

    if let Some(est) = e2e_est {
        println!(
            "  {:<40} {:>10} {:>10} {:>10} {:>5}%",
            format!("\u{25b8} {}  [e2e]", e2e.1),
            format_ns(est.median_ns),
            format_ns(est.mean_ns),
            format_ns(est.stddev_ns),
            "100",
        );
    }

    println!(
        "  {:<40} {:>10}",
        "\u{03a3} steps",
        format_ns(sum_median),
    );

    if e2e_median > 0.0 {
        let overhead = e2e_median - sum_median;
        let overhead_pct = overhead / e2e_median * 100.0;
        println!(
            "  {:<40} {:>10}   ({:.0}% of e2e)",
            "\u{0394} intra-path overhead",
            format_ns(overhead),
            overhead_pct,
        );
    }

    println!(
        "\n  Note: % and \u{0394} are approximate — step medians come from independent"
    );
    println!("  criterion runs and are not statistically composable.");
}
