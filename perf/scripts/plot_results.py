#!/usr/bin/env python3
"""Generate professional HFT-style latency charts from Lithos perf reports."""

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
    import matplotlib.ticker as mticker
    from matplotlib.patches import FancyBboxPatch
except ImportError:
    print("Error: matplotlib is required. Install with: pip3 install matplotlib")
    sys.exit(1)

# ═══════════════════════════════════════════════════════════════════════════════
# Theme: Dark terminal aesthetic (Bloomberg / trading desk)
# ═══════════════════════════════════════════════════════════════════════════════

BG = "#0D1117"
BG_PANEL = "#161B22"
FG = "#C9D1D9"
FG_DIM = "#6E7681"
FG_BRIGHT = "#F0F6FC"
GRID = "#21262D"
ACCENT_GREEN = "#3FB950"
ACCENT_RED = "#F85149"
ACCENT_BLUE = "#58A6FF"
ACCENT_PURPLE = "#BC8CFF"
ACCENT_ORANGE = "#D29922"
ACCENT_CYAN = "#39D2C0"
ACCENT_PINK = "#F778BA"
ACCENT_YELLOW = "#E3B341"

# Ordered palette for multi-series
PALETTE = [ACCENT_BLUE, ACCENT_GREEN, ACCENT_RED, ACCENT_PURPLE, ACCENT_ORANGE, ACCENT_CYAN]

# Percentile colors: cold → hot
PCTL_COLORS = {
    "p50": ACCENT_GREEN,
    "p99": ACCENT_ORANGE,
    "p99.9": ACCENT_RED,
    "max": ACCENT_PINK,
}


def apply_style() -> None:
    plt.rcParams.update(
        {
            "figure.facecolor": BG,
            "axes.facecolor": BG_PANEL,
            "axes.edgecolor": GRID,
            "axes.grid": True,
            "axes.grid.which": "major",
            "grid.color": GRID,
            "grid.alpha": 0.6,
            "grid.linewidth": 0.5,
            "grid.linestyle": "-",
            "font.size": 10,
            "font.family": "monospace",
            "text.color": FG,
            "axes.titlecolor": FG_BRIGHT,
            "axes.labelcolor": FG,
            "axes.titleweight": "bold",
            "axes.titlesize": 12,
            "axes.labelsize": 10,
            "xtick.color": FG_DIM,
            "ytick.color": FG_DIM,
            "xtick.labelsize": 9,
            "ytick.labelsize": 9,
            "legend.facecolor": BG_PANEL,
            "legend.edgecolor": GRID,
            "legend.labelcolor": FG,
            "legend.fontsize": 9,
            "legend.framealpha": 0.9,
            "savefig.bbox": "tight",
            "savefig.dpi": 200,
            "savefig.facecolor": BG,
            "savefig.pad_inches": 0.3,
        }
    )


# ═══════════════════════════════════════════════════════════════════════════════
# CLI
# ═══════════════════════════════════════════════════════════════════════════════


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
    return p.parse_args()


# ═══════════════════════════════════════════════════════════════════════════════
# Data extraction
# ═══════════════════════════════════════════════════════════════════════════════


def find_latest_report(results_dir: Path) -> Path | None:
    reports = sorted(results_dir.glob("*_report.json"))
    return reports[-1] if reports else None


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
        ("cross_thread.publish_to_state", "Cross-thread", ACCENT_BLUE),
        ("process.publish_to_state", "Process boundary", ACCENT_GREEN),
        ("soak.sampled_latency", "Soak sampled", ACCENT_ORANGE),
        ("live.ingest_to_state", "Live network", ACCENT_RED),
    ]


def save(fig, output_dir: Path, name: str) -> None:
    out = output_dir / name
    fig.savefig(out)
    plt.close(fig)
    print(f"  {name}")


def format_ns(ns: int) -> str:
    """Human-readable latency formatting."""
    if ns >= 1_000_000:
        return f"{ns / 1_000_000:.1f}ms"
    if ns >= 1_000:
        return f"{ns / 1_000:.1f}us"
    return f"{ns}ns"


def add_watermark(fig, text: str = "LITHOS PERF") -> None:
    """Subtle bottom-right watermark."""
    fig.text(
        0.98,
        0.02,
        text,
        transform=fig.transFigure,
        fontsize=7,
        color=FG_DIM,
        alpha=0.4,
        ha="right",
        va="bottom",
        family="monospace",
        weight="bold",
    )


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 01: Path Snapshot (p50 / p99 / p99.9 comparison)
# ═══════════════════════════════════════════════════════════════════════════════


def plot_path_snapshot(bench_map: dict[str, dict], output_dir: Path):
    names = [
        ("pipeline (batched)", "single"),
        ("publish->state_update", "thread"),
        ("pipeline e2e", "thread"),
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

    fig, ax = plt.subplots(figsize=(9, 5))
    x = range(len(labels))
    w = 0.22

    bars_50 = ax.bar([i - w for i in x], p50, w, label="p50", color=ACCENT_GREEN, alpha=0.9)
    bars_99 = ax.bar(list(x), p99, w, label="p99", color=ACCENT_ORANGE, alpha=0.9)
    bars_999 = ax.bar([i + w for i in x], p999, w, label="p99.9", color=ACCENT_RED, alpha=0.9)

    # Value labels on bars
    for bars in [bars_50, bars_99, bars_999]:
        for bar in bars:
            h = bar.get_height()
            ax.text(
                bar.get_x() + bar.get_width() / 2,
                h * 1.15,
                format_ns(int(h)),
                ha="center",
                va="bottom",
                fontsize=7,
                color=FG_DIM,
            )

    ax.set_yscale("log")
    ax.set_ylabel("Latency (ns)")
    ax.set_xticks(list(x))
    ax.set_xticklabels(labels, fontsize=10)
    ax.set_title("Path Snapshot: Typical vs Tail Latency")
    ax.legend(ncols=3, loc="upper left")
    add_watermark(fig)
    save(fig, output_dir, "01_path_snapshot.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 02: Component Percentile Heatmap
# ═══════════════════════════════════════════════════════════════════════════════


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
        "pipeline e2e",
        "process publish->state_update",
        "live ingest->state_update",
        "live socket.read wait",
    ]
    rows = [name for name in ordered if name in bench_map]
    if not rows:
        return

    cols = ["p50", "p90", "p99", "p999", "max"]
    col_labels = ["p50", "p90", "p99", "p99.9", "max"]
    matrix = [[max(1, stat(bench_map, r, c)) for c in cols] for r in rows]
    vals = [v for row in matrix for v in row]

    fig, ax = plt.subplots(figsize=(11, max(4.5, 0.5 * len(rows))))
    norm = mcolors.LogNorm(vmin=max(1, min(vals)), vmax=max(vals))

    # Custom colormap: dark blue → orange → red
    from matplotlib.colors import LinearSegmentedColormap

    cmap = LinearSegmentedColormap.from_list(
        "hft_heat", ["#0D1117", "#1A3A5C", "#2E6B8A", "#D29922", "#F85149", "#FF6B6B"]
    )

    im = ax.imshow(matrix, cmap=cmap, aspect="auto", norm=norm)
    ax.set_xticks(range(len(cols)))
    ax.set_xticklabels(col_labels)
    ax.set_yticks(range(len(rows)))
    ax.set_yticklabels(rows, fontsize=9)

    # Annotate cells
    for i, row in enumerate(matrix):
        for j, v in enumerate(row):
            text_color = FG_BRIGHT if v > (max(vals) * 0.1) else FG_DIM
            ax.text(j, i, format_ns(v), ha="center", va="center", fontsize=8, color=text_color)

    ax.set_title("Component Percentile Heatmap")
    cbar = fig.colorbar(im, ax=ax, shrink=0.8, pad=0.02)
    cbar.set_label("Latency (ns)", color=FG)
    cbar.ax.yaxis.set_tick_params(color=FG_DIM)
    plt.setp(plt.getp(cbar.ax.axes, "yticklabels"), color=FG_DIM)
    add_watermark(fig)
    save(fig, output_dir, "02_component_percentile_heatmap.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 03: Pipeline Waterfall (p50 decomposition)
# ═══════════════════════════════════════════════════════════════════════════════


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

    labels = ["JSON Parse", "Numeric", "Publish", "Read", "State Update", "Overhead"]
    vals = [parse, numeric, publish, read, state, overhead]
    colors = [ACCENT_BLUE, ACCENT_CYAN, ACCENT_GREEN, "#3FB950", ACCENT_PURPLE, FG_DIM]

    fig, ax = plt.subplots(figsize=(10, 5))

    # Cumulative waterfall
    lefts = []
    cumulative = 0
    for v in vals:
        lefts.append(cumulative)
        cumulative += v

    bars = ax.barh(labels, vals, left=lefts, color=colors, height=0.6, alpha=0.9)

    for i, (v, left) in enumerate(zip(vals, lefts)):
        if v > 0:
            pct = v * 100 / max(1, pipeline)
            ax.text(
                left + v + max(1, pipeline * 0.015),
                i,
                f"{v}ns ({pct:.0f}%)",
                va="center",
                fontsize=9,
                color=FG,
            )

    ax.set_xlabel("Cumulative Latency (ns)")
    ax.set_title(f"Pipeline Waterfall @ p50 (total: {format_ns(pipeline)})")
    ax.invert_yaxis()
    add_watermark(fig)
    save(fig, output_dir, "03_pipeline_waterfall.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 04: Tail CCDF (exceedance probability curves)
# ═══════════════════════════════════════════════════════════════════════════════


def plot_tail_ccdf(distributions: dict[str, dict], output_dir: Path):
    fig, ax = plt.subplots(figsize=(10, 5.5))
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
        ax.plot(xs, ys, linewidth=2, color=color, label=label, alpha=0.9)
        plotted += 1

    if plotted == 0:
        plt.close(fig)
        return

    # Reference lines for common SLA thresholds
    for threshold_ns, label in [(1000, "1us"), (10000, "10us")]:
        ax.axvline(threshold_ns, color=FG_DIM, linestyle=":", linewidth=0.8, alpha=0.5)
        ax.text(
            threshold_ns * 1.1,
            ax.get_ylim()[0] * 2,
            label,
            color=FG_DIM,
            fontsize=8,
            alpha=0.6,
        )

    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("Latency (ns)")
    ax.set_ylabel("P(Latency > x)")
    ax.set_title("Tail Risk: Complementary CDF")
    ax.legend()
    add_watermark(fig)
    save(fig, output_dir, "04_tail_ccdf.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 05: Percentile Ladder (p50 → p99.999)
# ═══════════════════════════════════════════════════════════════════════════════


def plot_percentile_ladders(distributions: dict[str, dict], output_dir: Path):
    ladder = [50.0, 75.0, 90.0, 95.0, 99.0, 99.5, 99.9, 99.99, 99.999]
    labels = ["p50", "p75", "p90", "p95", "p99", "p99.5", "p99.9", "p99.99", "p99.999"]

    fig, ax = plt.subplots(figsize=(10, 5.5))
    plotted = 0

    for key, label, color in path_defs():
        d = distributions.get(key)
        if not d:
            continue
        ys = [quantile_lookup(d, p) for p in ladder]
        ax.plot(
            range(len(ladder)),
            ys,
            marker="o",
            markersize=5,
            linewidth=2,
            color=color,
            label=label,
            alpha=0.9,
        )
        # Annotate the p99.9+ values
        for i in range(6, len(ys)):
            ax.annotate(
                format_ns(ys[i]),
                (i, ys[i]),
                textcoords="offset points",
                xytext=(8, 0),
                fontsize=7,
                color=color,
                alpha=0.8,
            )
        plotted += 1

    if plotted == 0:
        plt.close(fig)
        return

    ax.set_yscale("log")
    ax.set_xticks(range(len(ladder)))
    ax.set_xticklabels(labels, rotation=45, ha="right")
    ax.set_ylabel("Latency (ns)")
    ax.set_title("Percentile Ladder")
    ax.legend()
    add_watermark(fig)
    save(fig, output_dir, "05_percentile_ladder.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 06: Latency Regime Composition (stacked bar)
# ═══════════════════════════════════════════════════════════════════════════════


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
    all_labels, shares = [], []
    for key, label, _ in path_defs():
        d = distributions.get(key)
        if not d:
            continue
        buckets = regime_buckets(d.get("hist") or [])
        total = sum(buckets)
        if total <= 0:
            continue
        all_labels.append(label)
        shares.append([b * 100.0 / total for b in buckets])

    if not all_labels:
        return

    fig, ax = plt.subplots(figsize=(10, 5.5))
    regimes = ["<125ns", "125-500ns", "0.5-2us", "2-10us", ">10us"]
    colors = [ACCENT_GREEN, ACCENT_CYAN, ACCENT_YELLOW, ACCENT_ORANGE, ACCENT_RED]
    bottoms = [0.0] * len(all_labels)
    for i, (name, color) in enumerate(zip(regimes, colors)):
        vals = [s[i] for s in shares]
        ax.bar(all_labels, vals, bottom=bottoms, label=name, color=color, alpha=0.85, width=0.5)
        # Label segments > 5%
        for j, v in enumerate(vals):
            if v > 5:
                ax.text(
                    j,
                    bottoms[j] + v / 2,
                    f"{v:.0f}%",
                    ha="center",
                    va="center",
                    fontsize=8,
                    color=BG,
                    weight="bold",
                )
        bottoms = [b + v for b, v in zip(bottoms, vals)]

    ax.set_ylabel("Share of Samples (%)")
    ax.set_title("Latency Regime Composition")
    ax.legend(ncols=5, loc="upper center", fontsize=8)
    ax.set_ylim(0, 105)
    add_watermark(fig)
    save(fig, output_dir, "06_latency_regimes.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 07: Tail Amplification Heatmap
# ═══════════════════════════════════════════════════════════════════════════════


def plot_tail_amplification_heatmap(bench_map: dict[str, dict], output_dir: Path):
    paths = [
        "pipeline (batched)",
        "publish->state_update",
        "pipeline e2e",
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
    fig, ax = plt.subplots(figsize=(8.5, max(3.8, 0.8 * len(rows))))

    from matplotlib.colors import LinearSegmentedColormap

    cmap = LinearSegmentedColormap.from_list(
        "amplification", [BG_PANEL, "#1A3A5C", ACCENT_ORANGE, ACCENT_RED]
    )

    im = ax.imshow(matrix, cmap=cmap, aspect="auto", vmin=1.0, vmax=max(2.0, vmax))
    ax.set_xticks(range(len(cols)))
    ax.set_xticklabels(cols)
    ax.set_yticks(range(len(rows)))
    ax.set_yticklabels(rows, fontsize=9)
    ax.set_title("Tail Amplification Factor")

    for i, row in enumerate(matrix):
        for j, v in enumerate(row):
            color = FG_BRIGHT if v > vmax * 0.3 else FG
            ax.text(j, i, f"{v:.1f}x", ha="center", va="center", fontsize=10, color=color, weight="bold")

    cbar = fig.colorbar(im, ax=ax, shrink=0.8, pad=0.02)
    cbar.set_label("Multiplier", color=FG)
    cbar.ax.yaxis.set_tick_params(color=FG_DIM)
    plt.setp(plt.getp(cbar.ax.axes, "yticklabels"), color=FG_DIM)
    add_watermark(fig)
    save(fig, output_dir, "07_tail_amplification.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 08: Soak Control Chart (throughput stability)
# ═══════════════════════════════════════════════════════════════════════════════


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
    cv = (sd / mean * 100) if mean > 0 else 0

    fig, ax = plt.subplots(figsize=(10, 5))

    # Fill sigma bands
    ax.axhspan(mean - sd, mean + sd, color=ACCENT_BLUE, alpha=0.08)
    ax.axhline(mean, color=ACCENT_BLUE, linestyle="--", linewidth=1.5, alpha=0.8, label=f"mean={mean:.2f} M/s")
    ax.axhline(mean + sd, color=FG_DIM, linestyle=":", linewidth=0.8, alpha=0.5)
    ax.axhline(mean - sd, color=FG_DIM, linestyle=":", linewidth=0.8, alpha=0.5)

    # Color points by deviation
    for i in range(len(x)):
        c = ACCENT_GREEN if abs(y[i] - mean) <= sd else ACCENT_ORANGE if abs(y[i] - mean) <= 2 * sd else ACCENT_RED
        ax.plot(x[i], y[i], "o", color=c, markersize=6, zorder=5)

    ax.plot(x, y, linewidth=1.5, color=ACCENT_BLUE, alpha=0.5, zorder=4)

    ax.set_xlabel("Second")
    ax.set_ylabel("Throughput (M events/s)")
    ax.set_title(f"Soak Stability (CV={cv:.1f}%)")
    ax.legend(loc="upper right")

    # Annotate sigma bands
    ax.text(max(x) + 0.3, mean + sd, "+1\u03c3", color=FG_DIM, fontsize=8, va="center")
    ax.text(max(x) + 0.3, mean - sd, "-1\u03c3", color=FG_DIM, fontsize=8, va="center")

    add_watermark(fig)
    save(fig, output_dir, "08_soak_control.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 09: Burst Tail Tradeoff
# ═══════════════════════════════════════════════════════════════════════════════


def plot_burst_tail_tradeoff(data: dict, output_dir: Path):
    rounds = (data.get("burst") or {}).get("rounds") or []
    if not rounds:
        return

    r = [to_int(x.get("round", 0), 0) for x in rounds]
    t = [to_float(x.get("throughput_meps", 0.0), 0.0) for x in rounds]
    p99 = [max(1, to_int((x.get("stats") or {}).get("p99", 0), 1)) for x in rounds]

    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(13, 5))

    # Left: throughput by round
    ax1.fill_between(r, t, color=ACCENT_CYAN, alpha=0.15)
    ax1.plot(r, t, marker="o", markersize=4, linewidth=2, color=ACCENT_CYAN)
    ax1.set_xlabel("Burst Round")
    ax1.set_ylabel("Throughput (M events/s)")
    ax1.set_title("Throughput by Round")

    # Right: tail vs throughput scatter
    scatter = ax2.scatter(t, p99, s=50, c=p99, cmap="YlOrRd", norm=mcolors.LogNorm(), zorder=5, edgecolors=GRID)
    for i, rr in enumerate(r):
        ax2.annotate(
            str(rr),
            (t[i], p99[i]),
            textcoords="offset points",
            xytext=(5, 3),
            fontsize=7,
            color=FG_DIM,
        )
    ax2.set_yscale("log")
    ax2.set_xlabel("Throughput (M events/s)")
    ax2.set_ylabel("p99 Latency (ns)")
    ax2.set_title("Tail vs Throughput")

    add_watermark(fig)
    save(fig, output_dir, "09_burst_tradeoff.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 10: Live Socket Wait CCDF
# ═══════════════════════════════════════════════════════════════════════════════


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

    fig, ax = plt.subplots(figsize=(10, 5))
    ax.plot(xs, ys, linewidth=2, color=ACCENT_PURPLE, alpha=0.9)
    ax.fill_between(xs, ys, alpha=0.1, color=ACCENT_PURPLE)
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("socket.read() Wait (ns)")
    ax.set_ylabel("P(Wait > x)")
    ax.set_title("Live Socket Read-Wait Tail (CCDF)")
    add_watermark(fig)
    save(fig, output_dir, "10_live_socket_ccdf.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Chart 11: Stage Waterfall (real pipeline per-stage timing)
# ═══════════════════════════════════════════════════════════════════════════════


def plot_stage_waterfall(bench_map: dict[str, dict], output_dir: Path):
    obs_stages = [
        ("ParseJson", ACCENT_BLUE),
        ("ParseNumeric", ACCENT_CYAN),
        ("TimestampEvent", "#4C8BF5"),
        ("BuildTob", "#3D6B9E"),
        ("Publish", ACCENT_GREEN),
    ]
    onyx_stages = [
        ("TryRead", ACCENT_ORANGE),
        ("ProcessEvent", ACCENT_PURPLE),
        ("PrefetchNext", ACCENT_PINK),
    ]

    obs_vals = [(n, stat(bench_map, n, "p50"), c) for n, c in obs_stages if n in bench_map]
    onyx_vals = [(n, stat(bench_map, n, "p50"), c) for n, c in onyx_stages if n in bench_map]

    if not obs_vals and not onyx_vals:
        return

    # Add section divider
    all_items = []
    if obs_vals:
        all_items.extend(obs_vals)
    if obs_vals and onyx_vals:
        all_items.append(("---", 0, BG))  # separator
    if onyx_vals:
        all_items.extend(onyx_vals)

    labels = []
    vals = []
    colors = []
    for n, v, c in all_items:
        if n == "---":
            labels.append("")
            vals.append(0)
            colors.append(BG)
        else:
            labels.append(n)
            vals.append(v)
            colors.append(c)

    total = max(1, sum(v for v in vals if v > 0))
    obs_total = stat(bench_map, "ObsidianTotal", "p50")
    onyx_total = stat(bench_map, "OnyxTotal", "p50")

    fig, ax = plt.subplots(figsize=(10, max(4.5, 0.55 * len(labels))))
    bars = ax.barh(labels, vals, color=colors, height=0.6, alpha=0.9)

    for i, v in enumerate(vals):
        if v > 0:
            pct = v * 100 // total
            ax.text(
                v + max(1, total * 0.015),
                i,
                f"{v}ns ({pct}%)",
                va="center",
                fontsize=9,
                color=FG,
            )

    ax.set_xlabel("Latency (ns @ p50)")

    title_parts = []
    if obs_total > 0:
        title_parts.append(f"Obsidian: {format_ns(obs_total)}")
    if onyx_total > 0:
        title_parts.append(f"Onyx: {format_ns(onyx_total)}")
    subtitle = " | ".join(title_parts)
    ax.set_title(f"Stage Waterfall ({subtitle})" if subtitle else "Stage Waterfall")
    ax.invert_yaxis()
    add_watermark(fig)
    save(fig, output_dir, "11_stage_waterfall.png")


# ═══════════════════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════════════════


def main() -> None:
    apply_style()
    args = parse_args()

    report = args.report or find_latest_report(args.results_dir)
    if report is None:
        print("Error: no *_report.json found. Run perf_report first.")
        sys.exit(1)

    output_dir = args.output_dir or (args.results_dir / "plots")
    output_dir.mkdir(parents=True, exist_ok=True)

    # Always clean old plots (overwrite)
    removed = clean_output_dir(output_dir)
    if removed > 0:
        print(f"  Cleaned {removed} old plot files")

    print(f"  Reading: {report.name}")
    with open(report, "r", encoding="utf-8") as f:
        data = json.load(f)

    benchmarks = extract_benchmarks(data)
    if not benchmarks:
        print("Error: no benchmark data in report.")
        sys.exit(1)

    bench_map = {b["name"]: b for b in benchmarks if "name" in b}
    distributions = extract_distributions(data)

    print("  Generating charts:")
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

    print(f"\n  All charts saved to: {output_dir}")


if __name__ == "__main__":
    main()
