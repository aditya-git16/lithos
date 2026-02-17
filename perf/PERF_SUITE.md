# Lithos Performance Suite

Comprehensive benchmarking and latency measurement for the Lithos market data pipeline.

## Quick Start

```bash
# Run everything (build + report + criterion + plots)
bash perf/scripts/run_perf.sh

# Or run individually:
cargo build --release -p lithos-perf
./target/release/latency_report
cargo bench -p lithos-perf
./perf/scripts/plot_results.py

# Optional: include live exchange socket path
./target/release/latency_report --live-network --live-events 50000 --live-timeout-secs 30
```

## Architecture

The suite has two complementary measurement approaches:

### 1. `latency_report` (Custom Binary)
- Uses high-resolution monotonic timing for internal measurements:
  - macOS: `mach_absolute_time` (`mono_now_ns()`)
  - other OS: `clock_gettime(CLOCK_MONOTONIC)`
- Includes both micro and macro/full-path benchmarks:
  - in-thread hot path
  - cross-thread shared-memory path
  - process-boundary shared-memory path (separate writer/reader processes)
  - soak and burst behavior
  - optional live network path (WebSocket + TLS + parser + ring + state)
- Outputs ASCII table to stdout + JSON to `perf/results/`

### 2. Criterion Benchmarks
- Statistical rigor: warmup, outlier detection, confidence intervals
- HTML reports with regression detection in `target/criterion/`
- Best for tracking changes over time (CI integration)

## Benchmarks Covered

| Benchmark | File | What it measures |
|-----------|------|-----------------|
| Clock overhead | `bench_timestamp.rs` | `now_ns()` vs `Instant::now()` |
| Price parsing | `bench_parsing.rs` | `parse_px_2dp`, `parse_qty_3dp` |
| JSON parsing | `bench_parsing.rs` | Fast parser vs sonic-rs |
| Ring publish | `bench_broadcast.rs` | Write to shared memory ring |
| Ring read | `bench_broadcast.rs` | Read from ring (data/empty) |
| Ring round-trip | `bench_broadcast.rs` | Publish + read |
| Market state | `bench_market_state.rs` | `update_market_state_tob` |
| Full pipeline | `bench_pipeline.rs` | JSON parse -> publish -> read -> update |
| Cross-thread full path | `latency_report.rs` | Publisher thread -> ring -> consumer thread |
| Process-boundary full path | `latency_report.rs` | Writer process -> ring -> reader process |
| Soak/burst behavior | `latency_report.rs` | Sustained throughput and tail behavior |

## Interpreting Results

### Latency Report Output

```
Benchmark                         min      p50      p75      p90      p99    p99.9      max  unit
mono_now_ns()                      13       13       13       14       15       15       15  ns/op
```

- **min**: Best-case latency (often reflects measurement floor)
- **p50**: Median — what a "typical" operation costs
- **p99**: Tail latency — worst case for 1 in 100 ops
- **p99.9/max**: Extreme tail — OS scheduler noise, page faults

Cache notes:
- L1/L2 entries in the report are hardware capacity values plus working-set fit/latency sweeps.
- Direct PMU miss counters (L1/L2 miss rates) are not collected by default on macOS without privileged tooling.

### Batch vs Individual Measurement

Operations faster than the clock floor cannot be individually timed (clock overhead dominates). Batched measurements run N operations in a tight loop and divide total time by N, giving amortized per-op cost. Clock calibration rows (`obsidian now_ns()`, `mono_now_ns()`, `Instant::now()`) report the measurement floor used for interpretation.

### Why Cross-Thread Can Be Slower Than Single-Thread

This is expected:
- single-thread path avoids inter-core handoff and scheduler effects
- cross-thread/process paths include cache-coherency traffic and wakeup/preemption noise
- tails (`p99+`) are dominated by OS scheduling and not parser/ring arithmetic

## Results

- **ASCII report**: `perf/results/*_stdout.txt`
- **JSON data**: `perf/results/*_report.json`
- **Criterion HTML**: `target/criterion/report/index.html`
- **Plots**: `perf/results/plots/` (requires matplotlib)
  - `tail_ccdf_paths.png` (tail-risk curves across paths)
  - `percentile_ladders.png` (dense percentile escalation)
  - `latency_regimes.png` (share in latency buckets)
  - `pipeline_waterfall.png` (component contribution)
  - `path_snapshot.png` (single/thread/process/live p50 vs p99)
  - `soak_stability.png`, `burst_tail_tradeoff.png`

## Adding New Benchmarks

### Criterion
1. Add a new file in `perf/benches/bench_<name>.rs`
2. Add `[[bench]]` entry in `perf/Cargo.toml`
3. Use `criterion_group!` / `criterion_main!` macros
4. Use `black_box()` for inputs to prevent optimization

### Latency Report
1. Add measurement section in `perf/src/bin/latency_report.rs`
2. Use `measure()` for ops > 50ns, `measure_batched()` for faster ops
3. Push `BenchResult` into `results` so it is emitted to JSON/plots

## Dependencies

- Rust toolchain with edition 2024 support
- Optional: `matplotlib` for plot generation (`pip3 install matplotlib`)

## Plot Script Options

```bash
# Latest report in perf/results
./perf/scripts/plot_results.py

# Keep old plots instead of cleaning output dir
./perf/scripts/plot_results.py --no-clean

# Specific report file
./perf/scripts/plot_results.py --report perf/results/20260218_002907_report.json

# Custom output directory
./perf/scripts/plot_results.py --output-dir /tmp/lithos-plots
```

## Live-Network CLI Flags

```bash
--live-network
--live-url wss://stream.binance.com:9443/ws/btcusdt@bookTicker
--live-events 50000
--live-timeout-secs 30
```

## Chart Interpretation Guide

See `/Users/adityaanand/dev/lithos/perf/PLOT_READING_GUIDE.md` for a detailed explanation of every chart, red-flag patterns, and triage workflow.
