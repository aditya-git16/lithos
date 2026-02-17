#!/usr/bin/env python3
"""Generate professional HFT-style plots from Lithos perf reports."""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_RESULTS_DIR = SCRIPT_DIR.parent / "results"
os.environ.setdefault("MPLCONFIGDIR", str(DEFAULT_RESULTS_DIR / ".mplconfig"))

try:
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.colors as mcolors
    import matplotlib.pyplot as plt
except ImportError:
    print("Error: matplotlib is required. Install with: pip3 install matplotlib")
    sys.exit(1)


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Plot Lithos perf report JSON")
    p.add_argument("--report", type=Path, default=None, help="Path to *_report.json")
    p.add_argument(
        "--results-dir",
        type=Path,
        default=DEFAULT_RESULTS_DIR,
        help="Directory containing *_report.json files",
    )
    p.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Output directory for plots (default: <results-dir>/plots)",
    )
    p.add_argument(
        "--no-clean",
        action="store_true",
        help="Do not remove old plot files before writing new ones",
    )
    return p.parse_args()


def apply_style() -> None:
    plt.rcParams.update(
        {
            "figure.facecolor": "white",
            "axes.facecolor": "white",
            "axes.edgecolor": "#2B2B2B",
            "axes.grid": True,
            "grid.color": "#D0D7DE",
            "grid.alpha": 0.5,
            "grid.linestyle": "-",
            "font.size": 10,
            "font.family": "DejaVu Sans",
            "axes.titleweight": "bold",
            "axes.labelweight": "bold",
            "legend.frameon": False,
            "savefig.bbox": "tight",
            "savefig.dpi": 170,
        }
    )


def clean_output_dir(output_dir: Path) -> int:
    removed = 0
    for ext in ("*.png", "*.svg", "*.pdf"):
        for f in output_dir.glob(ext):
            try:
                f.unlink()
                removed += 1
            except OSError:
                pass
    return removed


def find_latest_report(results_dir: Path) -> Path | None:
    reports = sorted(results_dir.glob("*_report.json"))
    return reports[-1] if reports else None


def to_int(v: object, default: int = 0) -> int:
    try:
        return int(v)
    except Exception:
        return default


def to_float(v: object, default: float = 0.0) -> float:
    try:
        return float(v)
    except Exception:
        return default


def extract_benchmarks(data: dict) -> list[dict]:
    benches = (
        data.get("component_benchmarks")
        or data.get("stage_benchmarks")
        or data.get("benchmarks")
        or []
    )
    merged: dict[str, dict] = {}
    for b in benches:
        name = b.get("name")
        if name:
            merged[name] = b
    cross = data.get("cross_thread", {}).get("stats")
    if cross and "publish->state_update" not in merged:
        merged["publish->state_update"] = {
            "name": "publish->state_update",
            "unit": "ns",
            "stats": cross,
        }
    return list(merged.values())


def extract_distributions(data: dict) -> dict[str, dict]:
    out: dict[str, dict] = {}
    for d in data.get("distributions", []):
        name = d.get("name")
        if name:
            out[name] = d
    return out


def stat(bench_map: dict[str, dict], name: str, key: str) -> int:
    b = bench_map.get(name)
    if not b:
        return 0
    return max(0, to_int((b.get("stats") or {}).get(key, 0), 0))


def quantile_lookup(dist: dict, pct: float) -> int:
    points = dist.get("quantiles") or []
    if not points:
        return 0
    best = min(points, key=lambda p: abs(to_float(p.get("pct", 0.0)) - pct))
    return max(1, to_int(best.get("value", 0), 1))


def path_defs() -> list[tuple[str, str, str]]:
    return [
        ("cross_thread.publish_to_state", "Cross-thread", "#C62828"),
        ("process.publish_to_state", "Process boundary", "#1E3A8A"),
        ("soak.sampled_latency", "Soak sampled", "#166534"),
        ("live.ingest_to_state", "Live network", "#D97706"),
    ]


def save(fig, output_dir: Path, name: str) -> None:
    out = output_dir / name
    fig.savefig(out)
    plt.close(fig)
    print(f"  Saved: {out}")


def plot_path_snapshot(bench_map: dict[str, dict], output_dir: Path):
    names = [
        ("pipeline (batched)", "single"),
        ("publish->state_update", "thread"),
        ("process publish->state_update", "process"),
        ("live ingest->state_update", "live"),
    ]
    labels: list[str] = []
    p50: list[int] = []
    p99: list[int] = []
    p999: list[int] = []

    for n, l in names:
        a = stat(bench_map, n, "p50")
        b = stat(bench_map, n, "p99")
        c = stat(bench_map, n, "p999")
        if a == 0 and b == 0 and c == 0:
            continue
        labels.append(l)
        p50.append(max(1, a))
        p99.append(max(1, b))
        p999.append(max(1, c))

    if not labels:
        return

    fig, ax = plt.subplots(figsize=(8.8, 4.6))
    x = range(len(labels))
    w = 0.24
    ax.bar([i - w for i in x], p50, w, label="p50", color="#2E7D32")
    ax.bar(list(x), p99, w, label="p99", color="#C62828")
    ax.bar([i + w for i in x], p999, w, label="p99.9", color="#6A1B9A")
    ax.set_yscale("log")
    ax.set_ylabel("Latency (ns)")
    ax.set_xticks(list(x))
    ax.set_xticklabels(labels)
    ax.set_title("01. Path Snapshot: Typical vs Tail")
    ax.legend(ncols=3)
    save(fig, output_dir, "01_path_snapshot.png")


def plot_component_percentile_heatmap(bench_map: dict[str, dict], output_dir: Path):
    ordered = [
        "parse_binance_fast",
        "sonic_rs::from_slice",
        "full_parse_chain",
        "publish",
        "try_read (data)",
        "round-trip (pub+read)",
        "update_state (1 sym)",
        "pipeline (batched)",
        "publish->state_update",
        "process publish->state_update",
        "live ingest->state_update",
        "live socket.read wait",
    ]
    rows = [name for name in ordered if name in bench_map]
    if not rows:
        return

    cols = ["p50", "p90", "p99", "p999", "max"]
    matrix = [[max(1, stat(bench_map, r, c)) for c in cols] for r in rows]
    vals = [v for row in matrix for v in row]

    fig, ax = plt.subplots(figsize=(10.8, max(4.6, 0.46 * len(rows))))
    norm = mcolors.LogNorm(vmin=min(vals), vmax=max(vals))
    im = ax.imshow(matrix, cmap="YlOrRd", aspect="auto", norm=norm)
    ax.set_xticks(range(len(cols)))
    ax.set_xticklabels(["p50", "p90", "p99", "p99.9", "max"]) 
    ax.set_yticks(range(len(rows)))
    ax.set_yticklabels(rows)
    ax.set_title("02. Component Percentile Heatmap (ns)")
    cbar = fig.colorbar(im, ax=ax)
    cbar.set_label("Latency (ns)")
    save(fig, output_dir, "02_component_percentile_heatmap.png")


def plot_pipeline_waterfall(bench_map: dict[str, dict], output_dir: Path):
    parse = stat(bench_map, "parse_binance_fast", "p50")
    full = stat(bench_map, "full_parse_chain", "p50")
    publish = stat(bench_map, "publish", "p50")
    read = stat(bench_map, "try_read (data)", "p50")
    state = stat(bench_map, "update_state (1 sym)", "p50")
    pipeline = stat(bench_map, "pipeline (batched)", "p50")
    if pipeline <= 0:
        return

    numeric = max(0, full - parse)
    overhead = max(0, pipeline - (parse + numeric + publish + read + state))

    labels = ["JSON", "Numeric", "Publish", "Read", "State", "Overhead"]
    vals = [parse, numeric, publish, read, state, overhead]
    colors = ["#1565C0", "#0288D1", "#2E7D32", "#43A047", "#6A1B9A", "#757575"]

    fig, ax = plt.subplots(figsize=(9.2, 4.6))
    ax.barh(labels, vals, color=colors)
    for i, v in enumerate(vals):
        ax.text(v + max(1, pipeline * 0.01), i, f"{v}ns", va="center", fontsize=9)
    ax.set_xlabel("Latency contribution (ns @ p50)")
    ax.set_title(f"03. Pipeline Waterfall (total p50={pipeline}ns)")
    save(fig, output_dir, "03_pipeline_waterfall.png")


def plot_tail_ccdf(distributions: dict[str, dict], output_dir: Path):
    fig, ax = plt.subplots(figsize=(9.6, 5.2))
    plotted = 0

    for key, label, color in path_defs():
        d = distributions.get(key)
        if not d:
            continue
        pts = sorted(d.get("quantiles") or [], key=lambda p: to_float(p.get("pct", 0.0)))
        xs, ys = [], []
        for p in pts:
            pct = to_float(p.get("pct", 0.0))
            if pct >= 100.0:
                continue
            xs.append(max(1, to_int(p.get("value", 0), 1)))
            ys.append(max(1e-6, 1.0 - pct / 100.0))
        if not xs:
            continue
        ax.plot(xs, ys, linewidth=2, color=color, label=label)
        plotted += 1

    if plotted == 0:
        plt.close(fig)
        return

    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("Latency (ns)")
    ax.set_ylabel("Exceedance Probability")
    ax.set_title("04. Tail Risk Curves (CCDF)")
    ax.legend()
    save(fig, output_dir, "04_tail_ccdf_paths.png")


def plot_percentile_ladders(distributions: dict[str, dict], output_dir: Path):
    ladder = [50.0, 75.0, 90.0, 95.0, 99.0, 99.5, 99.9, 99.99, 99.999]
    fig, ax = plt.subplots(figsize=(9.6, 5.2))
    plotted = 0

    for key, label, color in path_defs():
        d = distributions.get(key)
        if not d:
            continue
        ys = [quantile_lookup(d, p) for p in ladder]
        ax.plot(ladder, ys, marker="o", linewidth=2, color=color, label=label)
        plotted += 1

    if plotted == 0:
        plt.close(fig)
        return

    ax.set_yscale("log")
    ax.set_xlabel("Percentile")
    ax.set_ylabel("Latency (ns)")
    ax.set_title("05. Percentile Ladder")
    ax.legend()
    save(fig, output_dir, "05_percentile_ladders.png")


def regime_buckets(hist: list[dict]) -> list[int]:
    cuts = [125, 500, 2_000, 10_000]
    out = [0, 0, 0, 0, 0]
    for b in hist:
        lo = to_int(b.get("lo", 0), 0)
        hi = to_int(b.get("hi", 0), 0)
        cnt = to_int(b.get("count", 0), 0)
        if cnt <= 0:
            continue
        mid = (lo + hi) / 2
        if mid <= cuts[0]:
            out[0] += cnt
        elif mid <= cuts[1]:
            out[1] += cnt
        elif mid <= cuts[2]:
            out[2] += cnt
        elif mid <= cuts[3]:
            out[3] += cnt
        else:
            out[4] += cnt
    return out


def plot_latency_regimes(distributions: dict[str, dict], output_dir: Path):
    labels, shares = [], []
    for key, label, _ in path_defs():
        d = distributions.get(key)
        if not d:
            continue
        buckets = regime_buckets(d.get("hist") or [])
        total = sum(buckets)
        if total <= 0:
            continue
        labels.append(label)
        shares.append([b * 100.0 / total for b in buckets])

    if not labels:
        return

    fig, ax = plt.subplots(figsize=(9.6, 5.2))
    regimes = ["<=125ns", "125-500ns", "0.5-2us", "2-10us", ">10us"]
    colors = ["#166534", "#22C55E", "#F59E0B", "#F97316", "#B91C1C"]
    bottoms = [0.0] * len(labels)
    for i, (name, color) in enumerate(zip(regimes, colors)):
        vals = [s[i] for s in shares]
        ax.bar(labels, vals, bottom=bottoms, label=name, color=color)
        bottoms = [b + v for b, v in zip(bottoms, vals)]

    ax.set_ylabel("Share of samples (%)")
    ax.set_title("06. Latency Regime Composition")
    ax.legend(ncols=3, fontsize=8)
    save(fig, output_dir, "06_latency_regimes.png")


def plot_tail_amplification_heatmap(bench_map: dict[str, dict], output_dir: Path):
    paths = [
        "pipeline (batched)",
        "publish->state_update",
        "process publish->state_update",
        "live ingest->state_update",
    ]
    rows = [p for p in paths if p in bench_map]
    if not rows:
        return

    cols = ["p99/p50", "p99.9/p50", "max/p50"]
    matrix: list[list[float]] = []
    for r in rows:
        p50 = max(1, stat(bench_map, r, "p50"))
        matrix.append(
            [
                stat(bench_map, r, "p99") / p50,
                stat(bench_map, r, "p999") / p50,
                stat(bench_map, r, "max") / p50,
            ]
        )

    vmax = max(max(row) for row in matrix)
    fig, ax = plt.subplots(figsize=(8.2, max(3.8, 0.7 * len(rows))))
    im = ax.imshow(matrix, cmap="OrRd", aspect="auto", vmin=1.0, vmax=max(1.0, vmax))
    ax.set_xticks(range(len(cols)))
    ax.set_xticklabels(cols)
    ax.set_yticks(range(len(rows)))
    ax.set_yticklabels(rows)
    ax.set_title("07. Tail Amplification Heatmap")
    for i, row in enumerate(matrix):
        for j, v in enumerate(row):
            ax.text(j, i, f"{v:.1f}x", ha="center", va="center", fontsize=9)
    cbar = fig.colorbar(im, ax=ax)
    cbar.set_label("Amplification factor")
    save(fig, output_dir, "07_tail_amplification_heatmap.png")


def plot_soak_control_chart(data: dict, output_dir: Path):
    windows = (data.get("soak") or {}).get("windows") or []
    if not windows:
        return

    x = [to_int(w.get("second", 0), 0) for w in windows]
    y = [to_float(w.get("throughput_meps", 0.0), 0.0) for w in windows]
    if not y:
        return

    mean = sum(y) / len(y)
    var = sum((v - mean) ** 2 for v in y) / len(y)
    sd = math.sqrt(var)

    fig, ax = plt.subplots(figsize=(9.6, 4.8))
    ax.plot(x, y, marker="o", linewidth=2, color="#2563EB", label="throughput")
    ax.axhline(mean, color="#111827", linestyle="--", linewidth=1.4, label=f"mean={mean:.2f}")
    ax.axhline(mean + sd, color="#9CA3AF", linestyle=":", linewidth=1.2, label="mean±1σ")
    ax.axhline(mean - sd, color="#9CA3AF", linestyle=":", linewidth=1.2)
    ax.set_xlabel("Second")
    ax.set_ylabel("Throughput (M events/s)")
    ax.set_title("08. Soak Control Chart")
    ax.legend(ncols=3, fontsize=8)
    save(fig, output_dir, "08_soak_control_chart.png")


def plot_burst_tail_tradeoff(data: dict, output_dir: Path):
    rounds = (data.get("burst") or {}).get("rounds") or []
    if not rounds:
        return

    r = [to_int(x.get("round", 0), 0) for x in rounds]
    t = [to_float(x.get("throughput_meps", 0.0), 0.0) for x in rounds]
    p99 = [max(1, to_int((x.get("stats") or {}).get("p99", 0), 1)) for x in rounds]

    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(12.6, 4.8))
    ax1.plot(r, t, marker="o", linewidth=2, color="#0F766E")
    ax1.set_xlabel("Burst round")
    ax1.set_ylabel("Throughput (M events/s)")
    ax1.set_title("09a. Burst Throughput by Round")

    ax2.scatter(t, p99, s=44, color="#B91C1C")
    for i, rr in enumerate(r):
        ax2.annotate(str(rr), (t[i], p99[i]), textcoords="offset points", xytext=(4, 3), fontsize=8)
    ax2.set_yscale("log")
    ax2.set_xlabel("Throughput (M events/s)")
    ax2.set_ylabel("p99 latency (ns)")
    ax2.set_title("09b. Tail vs Throughput")

    save(fig, output_dir, "09_burst_tail_tradeoff.png")


def plot_live_socket_wait_ccdf(distributions: dict[str, dict], output_dir: Path):
    d = distributions.get("live.socket_read_wait")
    if not d:
        return
    pts = sorted(d.get("quantiles") or [], key=lambda p: to_float(p.get("pct", 0.0)))
    xs, ys = [], []
    for p in pts:
        pct = to_float(p.get("pct", 0.0))
        if pct >= 100.0:
            continue
        xs.append(max(1, to_int(p.get("value", 0), 1)))
        ys.append(max(1e-6, 1.0 - pct / 100.0))
    if not xs:
        return

    fig, ax = plt.subplots(figsize=(9.6, 5.0))
    ax.plot(xs, ys, linewidth=2, color="#7C3AED")
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("socket.read wait (ns)")
    ax.set_ylabel("Exceedance Probability")
    ax.set_title("10. Live Socket Read-Wait Tail (CCDF)")
    save(fig, output_dir, "10_live_socket_wait_ccdf.png")


def plot_stage_waterfall(bench_map: dict[str, dict], output_dir: Path):
    """Per-stage breakdown from PerfRecorder instrumented data."""
    obs_stages = [
        ("ParseJson", "#1565C0"),
        ("ParseNumeric", "#0288D1"),
        ("TimestampEvent", "#00838F"),
        ("BuildTob", "#00695C"),
        ("Publish", "#2E7D32"),
    ]
    onyx_stages = [
        ("TryRead", "#43A047"),
        ("ProcessEvent", "#6A1B9A"),
        ("PrefetchNext", "#AD1457"),
    ]

    obs_vals = [(n, stat(bench_map, n, "p50"), c) for n, c in obs_stages if n in bench_map]
    onyx_vals = [(n, stat(bench_map, n, "p50"), c) for n, c in onyx_stages if n in bench_map]

    if not obs_vals and not onyx_vals:
        return

    all_items = obs_vals + onyx_vals
    labels = [n for n, _, _ in all_items]
    vals = [v for _, v, _ in all_items]
    colors = [c for _, _, c in all_items]

    fig, ax = plt.subplots(figsize=(9.6, max(4.0, 0.5 * len(labels))))
    ax.barh(labels, vals, color=colors)
    for i, v in enumerate(vals):
        total = max(1, sum(vals))
        ax.text(
            v + max(1, total * 0.01),
            i,
            f"{v}ns ({v*100//max(1,total)}%)",
            va="center",
            fontsize=9,
        )
    ax.set_xlabel("Latency (ns @ p50)")
    ax.set_title("Stage Waterfall (real pipeline instrumentation)")
    ax.invert_yaxis()
    save(fig, output_dir, "11_stage_waterfall.png")


def main() -> None:
    apply_style()
    args = parse_args()

    report = args.report or find_latest_report(args.results_dir)
    if report is None:
        print("Error: no *_report.json files found. Run perf_report first.")
        sys.exit(1)

    output_dir = args.output_dir or (args.results_dir / "plots")
    output_dir.mkdir(parents=True, exist_ok=True)
    if not args.no_clean:
        removed = clean_output_dir(output_dir)
        print(f"Cleaned {removed} stale plot files from: {output_dir}")

    print(f"Reading: {report}")
    with open(report, "r", encoding="utf-8") as f:
        data = json.load(f)

    benchmarks = extract_benchmarks(data)
    if not benchmarks:
        print("Error: no benchmark data in report.")
        sys.exit(1)

    bench_map = {b["name"]: b for b in benchmarks if "name" in b}
    distributions = extract_distributions(data)

    plot_path_snapshot(bench_map, output_dir)
    plot_component_percentile_heatmap(bench_map, output_dir)
    plot_pipeline_waterfall(bench_map, output_dir)
    plot_tail_ccdf(distributions, output_dir)
    plot_percentile_ladders(distributions, output_dir)
    plot_latency_regimes(distributions, output_dir)
    plot_tail_amplification_heatmap(bench_map, output_dir)
    plot_soak_control_chart(data, output_dir)
    plot_burst_tail_tradeoff(data, output_dir)
    plot_live_socket_wait_ccdf(distributions, output_dir)
    plot_stage_waterfall(bench_map, output_dir)

    print(f"\nAll plots saved to: {output_dir}")


if __name__ == "__main__":
    main()
