# Lithos Performance Suite

## Architecture


- **Criterion** (`bench_hot_path.rs`) — micro-benchmarks for individual hot-path functions. Statistical rigor with confidence intervals, regression detection.
- **perf_report** (`perf_report.rs`) — cross-thread e2e pipeline measurement and sustained soak test. Reads criterion JSON for the report display.

```
cargo bench (criterion)  →  target/criterion/**/estimates.json
                                          ↓
perf_report (binary)     →  reads criterion JSON + runs e2e/soak  →  results/*_report.json
                                          ↓
plot_results.py          →  reads report JSON  →  results/plots/*.png
```

## CLI

```bash
bash perf/scripts/run_perf.sh           # full suite: criterion → report → plots (default)
bash perf/scripts/run_perf.sh perf      # full suite (explicit)
bash perf/scripts/run_perf.sh bench     # criterion micro-benchmarks only
bash perf/scripts/run_perf.sh plot      # generate charts from latest results
bash perf/scripts/run_perf.sh perf --flamegraph  # include flamegraph
```

`perf` mode runs criterion first so that `perf_report` can read the criterion JSON.

### Running criterion directly

```bash
cargo bench -p lithos-perf --bench bench_hot_path           # both groups
cargo bench -p lithos-perf --bench bench_hot_path -- obsidian  # obsidian group only
cargo bench -p lithos-perf --bench bench_hot_path -- onyx      # onyx group only
```

### Plot script directly

```bash
python3 perf/scripts/plot_results.py                             # latest report
python3 perf/scripts/plot_results.py --report path/to/file.json  # specific report
python3 perf/scripts/plot_results.py --output-dir /tmp/plots     # custom output
```

Requires `matplotlib` (`pip3 install matplotlib`).

## Output

- **JSON reports**: `perf/results/*_report.json`
- **Charts**: `perf/results/plots/`
- **Criterion HTML**: `target/criterion/`

## bench_hot_path.rs Design

One criterion bench file with two groups, each containing individual steps and full e2e:

### Group: `obsidian`

| Benchmark | What it measures |
|---|---|
| `parse_book_ticker_fast` | JSON parsing of Binance bookTicker with corpus cycling |
| `parse_px_qty_x4` | 2 price + 2 quantity numeric parses |
| `build_tob` | TopOfBook struct construction |
| `publish` | writer.publish() to ring buffer |
| `process_text` | **Full e2e** — ObsidianProcessor.process_text() |

### Group: `onyx`

| Benchmark | What it measures |
|---|---|
| `try_read` | reader.try_read() with pre-filled ring, 256 symbols |
| `update_market_state` | update_market_state_tob(), 256 symbols cycling |
| `poll_event` | **Full e2e** — try_read + update + prefetch_next + spin_loop, 256 symbols, `iter_custom` |

All benchmarks use 256 symbols and the same corpus/ring configuration for coherent numbers. The e2e benchmarks measure the complete hot path so that step times and e2e times are directly comparable.

## perf_report Sections

### Criterion Path Display

Reads `target/criterion/obsidian/*/new/estimates.json` and `target/criterion/onyx/*/new/estimates.json`. Displays each path as:

```
  OBSIDIAN HOT PATH  (WebSocket JSON → parse → build → publish)

  Step                                 median        mean      stddev       %
  ─────────────────────────────────────────────────────────────────────────────
  parse_book_ticker_fast()               23 ns       24 ns       1 ns    51%
  parse_px_qty_x4()                      12 ns       12 ns       1 ns    27%
  build_tob()                             1 ns        1 ns       0 ns     2%
  publish()                               5 ns        6 ns       0 ns    11%
  ─────────────────────────────────────────────────────────────────────────────
  ▸ process_text  [e2e]                  45 ns       46 ns       2 ns   100%
  Σ steps                                41 ns
  Δ intra-path overhead                   4 ns   (9% of e2e)
```

### Cross-Thread Pipeline

Uses criterion medians for the single-thread sum, then runs a real two-thread measurement:

- **Producer**: 256 symbols, parse + publish with `perf_now_ns()` timestamps
- **Consumer**: try_read + update + `prefetch_next()` + `spin_loop()`
- 200K events, latency = `recv_ts - event.ts_event_ns`

```
  CROSS-THREAD PIPELINE  (Obsidian thread → mmap ring → Onyx thread)

  Obsidian  process_text (criterion)     45 ns
  Onyx      poll_event   (criterion)      8 ns
  Single-thread sum                      53 ns
  ─────────────────────────────────────────────────────────────────────────────
  Cross-thread e2e         p50       p99     p99.9       max
                          120 ns    210 ns    450 ns   1200 ns
  ─────────────────────────────────────────────────────────────────────────────
  IPC cache-coherency overhead           67 ns   (e2e p50 − sum, core→core)

  200K events | 256 symbols | 0 overruns | 0 filtered
```

### Soak Test

5-second sustained load with 256 symbols cycling. Uses manual writer (not ObsidianProcessor) with `prefetch_next()` + `spin_loop()` in the consumer path. Reports per-second throughput and sampled latency.

## JSON Format

The `*_report.json` includes:

- `criterion_benchmarks`: Array of `{name, median_ns, mean_ns, stddev_ns}` from criterion
- `stage_benchmarks`: Array of `{name, unit, stats}` from perf_report's own measurements
- `cross_thread`: `{stats, overruns}` for the e2e pipeline measurement
- `soak`: `{windows, latency}` for the soak test
- `system`: Hardware info
- `resources`: rusage snapshots

## Reading the Charts

All charts use a dark terminal theme. Latency units are nanoseconds unless labeled otherwise.

### 01 — Per-Function Breakdown

Horizontal bars showing criterion median per function. Single bar per function (no percentile spread from criterion).

### 02 — Pipeline Waterfall

Cumulative median cost flow through all pipeline functions. Each segment starts where the previous ends.

### 03 — Percentile Matrix

Criterion micro-benchmarks show a single median value across all columns. Pipeline e2e and soak rows show full p50 through max percentiles.

### 04 — Latency Profile

Percentile ladder (p50 through max) on log-Y for cross-thread e2e and soak only. Background shaded green (<100ns), amber (100ns-1us), red (>1us).

### 05 — Tail Amplification

Heatmap of multipliers for pipeline e2e and soak: p99/p50, p99.9/p50, max/p50.

### 06 — Soak Stability

Left: throughput per second during sustained load. Right: soak latency summary card.

### 07 — Summary

Three-column dashboard: system info, pipeline latency (criterion medians for Obsidian/Onyx, measured e2e p50/p99), resource usage.

## Triage Workflow

1. Start with **07 Summary** for overall health
2. Identify cost center with **01 Per-Function Breakdown** and **02 Pipeline Waterfall**
3. Check **04 Latency Profile** and **05 Tail Amplification** for tail behavior
4. Validate throughput stability with **06 Soak Stability**
5. Full spread in **03 Percentile Matrix**
