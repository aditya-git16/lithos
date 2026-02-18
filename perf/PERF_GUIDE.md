# Lithos Performance Suite

## CLI

```bash
bash perf/scripts/run_perf.sh           # full suite (default)
bash perf/scripts/run_perf.sh perf      # full suite (explicit)
bash perf/scripts/run_perf.sh bench     # criterion micro-benchmarks only
bash perf/scripts/run_perf.sh plot      # generate charts from latest results
bash perf/scripts/run_perf.sh perf --flamegraph  # include flamegraph
```

`perf` mode runs the pipeline report + charts. `bench` mode runs criterion micro-benchmarks only. They do not overlap.

Plot mode always uses the latest `*_report.json` in `perf/results/` and overwrites any existing charts.

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

## Pipeline Architecture

The report measures two stages and their combined pipeline:

```
[OBSIDIAN]  WebSocket JSON → parse → build TopOfBook → publish to shm
[ONYX]      read from shm → update MarketState
```

### Obsidian Path (ingest → publish)

Individual functions measured in isolation:

| Benchmark | What it measures |
|---|---|
| `parse_book_ticker_fast()` | JSON parsing of Binance bookTicker |
| `parse_px/qty() ×4` | 2 price + 2 quantity numeric parses |
| `TopOfBook { .. }` | Struct construction |
| `writer.publish()` | Write to SPMC ring buffer |

Stage total: **`process_text()`** — the full Obsidian hot path from raw JSON to published event.

### Onyx Path (read → state update)

| Benchmark | What it measures |
|---|---|
| `reader.try_read()` | Read from SPMC ring buffer |
| `update_market_state_tob()` | Update fixed-array market state |

Stage total: **`read→update()`** — combined read + state update.

### Pipeline Summary

The report sums the two stage totals and compares against the real cross-thread measurement:

```
Obsidian  process_text()     p50 = X ns
Onyx      read→update()      p50 = Y ns
────────────────────────────────────────
Single-thread total (sum)    p50 = X+Y ns

Cross-thread e2e (measured)  p50 = Z ns
IPC transit overhead         p50 ≈ Z-(X+Y) ns
```

The IPC transit overhead captures cross-thread scheduling, cache-line transfer, and ring buffer contention — costs not visible in single-thread benchmarks.

## Measurement Technique

Each function is isolated in a tight loop (10k iterations per batch, 2000 batches). Total time divided by N gives ~1ns accuracy, bypassing the 42ns clock quantisation on Apple M3 (`mach_absolute_time` ticks at 24 MHz = 41.67ns/tick).

The cross-thread e2e measurement runs 200K events through the real two-thread pipeline: Obsidian stamps `ts_event_ns` before publish, Onyx reads it after `try_read()`, latency = `recv_ts - event.ts_event_ns`.

## Reading the Charts

All charts use a dark terminal theme. Latency units are nanoseconds unless labeled otherwise.

### 01 — Per-Function Breakdown

Horizontal bars showing per-function cost: `parse_book_ticker_fast()`, `parse_px/qty() ×4`, `TopOfBook { .. }`, `writer.publish()`, `reader.try_read()`, `update_market_state_tob()`.

**What to look for**: The longest bar is the primary optimisation target. These are accurate sub-nanosecond measurements from batched amortisation.

### 02 — Pipeline Waterfall

Cumulative p50 cost flow through all pipeline functions. Each segment starts where the previous ends.

**What to look for**: Identifies what fraction of the total path each function consumes. The total should approximate `process_text() + read→update()`.

### 03 — Percentile Matrix

Row = benchmark, column = percentile (p50 through max). Color intensity on a log scale. Includes both individual functions and stage totals (`process_text()`, `read→update()`, `pipeline e2e`, `soak_latency`).

**What to look for**: Horizontal brightening means tail amplification. Batched functions should stay uniformly cool. Pipeline e2e and soak rows show real cross-thread tail behavior.

### 04 — Latency Profile

Percentile ladder (p50 through max) on log-Y for cross-thread e2e, soak, Obsidian `process_text()`, and Onyx `read→update()`. Background shaded green (<100ns), amber (100ns-1us), red (>1us).

**What to look for**: A smooth slope means controlled tail growth. A steep cliff after p99 indicates a latency wall (scheduler preemption, page faults).

### 05 — Tail Amplification

Heatmap of multipliers: p99/p50, p99.9/p50, max/p50. Each cell shows how much tail latency exceeds typical.

**What to look for**: Lower factors mean tighter tails. p99.9/p50 above 10x warrants investigation.

### 06 — Soak Stability

Left: throughput per second during sustained load. Right: soak latency summary card.

**What to look for**: Stable runs have all points green and CV < 5%. Persistent drift or red points indicate thermal throttling or resource contention.

### 07 — Summary

Three-column dashboard: system info, pipeline latency (Obsidian p50, Onyx p50, sum, e2e p50/p99), resource usage.

**What to look for**: Quick sanity check. The sum should be close to individual stage totals. Peak RSS should stay under 100MB. Involuntary context switches should be low.

## Triage Workflow

1. Start with **07 Summary** for overall health and pipeline breakdown
2. Identify cost center with **01 Per-Function Breakdown** and **02 Pipeline Waterfall**
3. If tails look bad, check **04 Latency Profile** and **05 Tail Amplification**
4. Validate throughput stability with **06 Soak Stability**
5. Full percentile spread across all benchmarks in **03 Percentile Matrix**
