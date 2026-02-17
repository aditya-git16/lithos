# Lithos Performance Plot Reading Guide

This guide explains how to read each generated chart and what to do when a chart looks bad.

## Scope

The plot set visualizes:
- single-thread hot path
- cross-thread shared-memory path
- process-boundary shared-memory path
- optional live network path (`--live-network`)

All latency units are nanoseconds unless explicitly stated.

## Output Files

Generated in `/Users/adityaanand/dev/lithos/perf/results/plots`:

1. `01_path_snapshot.png`
2. `02_component_percentile_heatmap.png`
3. `03_pipeline_waterfall.png`
4. `04_tail_ccdf_paths.png`
5. `05_percentile_ladders.png`
6. `06_latency_regimes.png`
7. `07_tail_amplification_heatmap.png`
8. `08_soak_control_chart.png`
9. `09_burst_tail_tradeoff.png`
10. `10_live_socket_wait_ccdf.png` (only when live socket data exists)

## How To Read Each Chart

## 1) Path Snapshot (`01_path_snapshot.png`)

What it shows:
- `p50`, `p99`, `p99.9` for each path (single/thread/process/live).

How to read:
- Big gap between `p50` and `p99.9` means unstable tail behavior.
- Process/live should usually have higher tails than single-thread.

Red flags:
- `p99` or `p99.9` rising run-over-run while `p50` is flat.
- Live path lower than single-thread by large factor (likely measurement/setup issue).

## 2) Component Percentile Heatmap (`02_component_percentile_heatmap.png`)

What it shows:
- Row = component
- Column = percentile (`p50`, `p90`, `p99`, `p99.9`, `max`)
- Color intensity on log scale.

How to read:
- Horizontal brightening across columns indicates tail amplification.
- Compare parser rows vs ring/state rows to locate dominant cost center.

Red flags:
- `max` orders of magnitude above `p99.9` for core path components.
- Ring/state rows suddenly as hot as parser rows (likely contention/scheduling issue).

## 3) Pipeline Waterfall (`03_pipeline_waterfall.png`)

What it shows:
- `p50` contribution decomposition of pipeline total.

How to read:
- Biggest bars are primary optimization targets.
- Overhead bar should stay modest and stable.

Red flags:
- Overhead bar expanding over time (timing noise, control-flow drift, or hidden work).

## 4) Tail CCDF (`04_tail_ccdf_paths.png`)

What it shows:
- Tail probability vs latency on log-log axes.

How to read:
- Lower curve is better (lower probability of exceeding latency threshold).
- Right-shift means worse tail.

Red flags:
- Curve kinks/fat tail beyond ~10us indicating scheduler/preemption spikes.
- Process/live curves crossing unpredictably run-to-run with large swings.

## 5) Percentile Ladder (`05_percentile_ladders.png`)

What it shows:
- Dense percentile points from `p50` to `p99.999`.

How to read:
- Smooth slope = controlled tail growth.
- Steep lift after `p99` = tail cliff.

Red flags:
- Near-vertical jump around `p99.9+`.
- Large day-to-day ladder shape change at same config.

## 6) Latency Regime Composition (`06_latency_regimes.png`)

What it shows:
- Share of samples in latency buckets (`<=125ns`, `125-500ns`, `0.5-2us`, `2-10us`, `>10us`).

How to read:
- Healthy systems keep most mass in low-latency bins.
- Small high-latency bins are expected; growth indicates jitter pressure.

Red flags:
- Expanding `>10us` share.
- Shrinking `<=125ns` share without workload change.

## 7) Tail Amplification Heatmap (`07_tail_amplification_heatmap.png`)

What it shows:
- Multipliers: `p99/p50`, `p99.9/p50`, `max/p50` per path.

How to read:
- Lower multipliers indicate tighter tails.
- Compare across paths to quantify tail fragility, not absolute speed.

Red flags:
- Sharp increase in `p99.9/p50` or `max/p50` for thread/process paths.

## 8) Soak Control Chart (`08_soak_control_chart.png`)

What it shows:
- Throughput per second with mean and ±1 sigma bands.

How to read:
- Stable throughput oscillates tightly around mean.
- Persistent drift or oscillation indicates thermal/scheduler/resource effects.

Red flags:
- Repeated points outside ±1σ in one direction.
- Downward drift across run window.

## 9) Burst Tail Tradeoff (`09_burst_tail_tradeoff.png`)

What it shows:
- Left: throughput by burst round.
- Right: `p99` vs throughput scatter, annotated by round.

How to read:
- Tradeoff curve should be gradual, not chaotic.
- Specific rounds with poor tail identify fragile states.

Red flags:
- Throughput collapse with simultaneous tail explosion in multiple rounds.

## 10) Live Socket Read-Wait CCDF (`10_live_socket_wait_ccdf.png`)

What it shows:
- Tail distribution of `socket.read()` wait in live mode.

How to read:
- This reflects network/TLS/frame arrival behavior, not just local compute.

Red flags:
- Heavy right tail at socket stage dominating total live path tail.

## Practical Triage Order

1. Check `01_path_snapshot.png` for overall path health.
2. If tails look bad, inspect `04_tail_ccdf_paths.png` and `05_percentile_ladders.png`.
3. If throughput unstable, inspect `08_soak_control_chart.png` and `09_burst_tail_tradeoff.png`.
4. Identify dominant compute cost in `03_pipeline_waterfall.png`.
5. Validate component percentile spread in `02_component_percentile_heatmap.png`.
6. For live mode, isolate socket behavior with `10_live_socket_wait_ccdf.png`.

## Caveats

- macOS `ru_majflt` is page reclaims, not Linux disk-backed major faults.
- Cache sections are capacity/fit analyses unless PMU counters are collected externally.
- Tail values (`p99.9+`, `max`) are sensitive to scheduler noise; compare multiple runs, not one sample.

## Reproducible Workflow

```bash
cargo build --release -p lithos-perf
./target/release/latency_report
./perf/scripts/plot_results.py
```

Live mode:

```bash
./target/release/latency_report --live-network --live-events 50000 --live-timeout-secs 30
./perf/scripts/plot_results.py
```
