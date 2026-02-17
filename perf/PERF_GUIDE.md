# Lithos Performance Suite

## CLI

```bash
bash perf/scripts/run_perf.sh           # full suite (default)
bash perf/scripts/run_perf.sh perf      # full suite (explicit)
bash perf/scripts/run_perf.sh bench     # criterion micro-benchmarks only
bash perf/scripts/run_perf.sh plot      # generate charts from latest results
bash perf/scripts/run_perf.sh perf --flamegraph  # include flamegraph
```

Plot mode always uses the latest `*_report.json` in `perf/results/` and overwrites any existing charts.

### Plot script directly

```bash
python3 perf/scripts/plot_results.py                          # latest report
python3 perf/scripts/plot_results.py --report path/to/file.json  # specific report
python3 perf/scripts/plot_results.py --output-dir /tmp/plots  # custom output
```

Requires `matplotlib` (`pip3 install matplotlib`).

## Output

- **JSON reports**: `perf/results/*_report.json`
- **Charts**: `perf/results/plots/`
- **Criterion HTML**: `target/criterion/`

## Reading the Charts

All charts use a dark terminal theme. Latency units are nanoseconds unless labeled otherwise.

### 01 — Path Snapshot

Grouped bars showing p50 / p99 / p99.9 for each measurement path (single-thread, cross-thread, process boundary, live network).

**What to look for**: Large gap between p50 and p99.9 indicates tail instability. Cross-thread and process paths should have higher tails than single-thread — if they don't, suspect measurement issues.

### 02 — Component Percentile Heatmap

Row = component, column = percentile (p50 through max). Color intensity is on a log scale.

**What to look for**: Horizontal brightening across columns means that component has tail amplification. Compare parser rows vs ring/state rows to locate the dominant cost center.

### 03 — Pipeline Waterfall

Cumulative contribution of each pipeline stage at p50. Each bar starts where the previous ends, showing how latency accumulates through JSON parse → numeric → publish → read → state update.

**What to look for**: The longest bar is the primary optimization target. The overhead segment should stay small and stable — growth suggests hidden work or timing noise.

### 04 — Tail CCDF

Complementary CDF on log-log axes. Each curve shows the probability that latency exceeds a given threshold.

**What to look for**: Lower/left is better. A curve that kinks or develops a fat tail beyond 10us signals scheduler preemption spikes. Vertical reference lines at 1us and 10us mark common SLA thresholds.

### 05 — Percentile Ladder

Dense percentile values from p50 up to p99.999 on a log-Y scale. Values are annotated at p99.9+.

**What to look for**: A smooth slope means controlled tail growth. A steep cliff after p99 indicates a latency wall (usually scheduler or page fault driven). Compare shapes across runs at the same config to detect regressions.

### 06 — Latency Regime Composition

Stacked bars showing what percentage of samples fall into each latency bucket (<125ns, 125–500ns, 0.5–2us, 2–10us, >10us).

**What to look for**: Healthy pipelines keep most mass in the leftmost (green) bucket. If the >10us (red) share is growing or the <125ns share is shrinking without workload changes, something is degrading.

### 07 — Tail Amplification Factor

Heatmap of multipliers: p99/p50, p99.9/p50, max/p50 for each path. Each cell shows the factor by which tail latency exceeds typical latency.

**What to look for**: Lower factors mean tighter tails. A p99.9/p50 above 10x warrants investigation. Sudden jumps in max/p50 suggest OS-level interference.

### 08 — Soak Control Chart

Throughput per second during a 5-second sustained load test. Points are colored green (within 1σ), orange (1–2σ), or red (>2σ) relative to the mean. The coefficient of variation (CV) is shown in the title.

**What to look for**: Stable runs have all points green and CV < 5%. Persistent drift or repeated red points indicate thermal throttling, GC pressure, or resource contention.

### 09 — Burst Tradeoff

Left panel: throughput by burst round. Right panel: p99 latency vs throughput scatter, colored by tail severity.

**What to look for**: A gradual tradeoff curve is expected. Rounds where throughput collapses simultaneously with tail explosion indicate fragile operating points.

### 10 — Live Socket CCDF

Tail distribution of `socket.read()` wait time in live mode. Only generated when live network data exists.

**What to look for**: This reflects network/TLS/frame arrival behavior, not local compute. A heavy right tail here dominating the total live path tail means the bottleneck is external.

### 11 — Stage Waterfall

Per-stage p50 breakdown from PerfRecorder instrumentation. Obsidian stages (parse, numeric, timestamp, build, publish) are separated from Onyx stages (read, process, prefetch). Totals shown in the title.

**What to look for**: Identifies the exact stage consuming the most time within each binary. The percentage annotation shows each stage's share of the total.

## Triage Workflow

1. Start with **01 Path Snapshot** for overall health
2. If tails look bad → **04 Tail CCDF** and **05 Percentile Ladder**
3. If throughput unstable → **08 Soak Control** and **09 Burst Tradeoff**
4. Identify cost center → **03 Pipeline Waterfall** and **11 Stage Waterfall**
5. Validate spread → **02 Component Heatmap** and **07 Tail Amplification**
6. Live mode issues → **10 Live Socket CCDF**
