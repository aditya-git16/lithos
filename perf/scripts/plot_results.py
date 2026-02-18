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
    import matplotlib.pyplot as plt
    import matplotlib.ticker as mticker
    from matplotlib.colors import LinearSegmentedColormap
except ImportError:
    print("Error: matplotlib is required.  pip3 install matplotlib")
    sys.exit(1)

# ═══════════════════════════════════════════════════════════════════════════════
# Theme
# ═══════════════════════════════════════════════════════════════════════════════

BG       = "#0C1016"
PANEL    = "#131A24"
BORDER   = "#1C2635"
GRID     = "#1C2635"
FG       = "#B0BEC5"
FG_DIM   = "#546E7A"
FG_BRIGHT= "#ECEFF1"

# Monochromatic blue-teal gradient for stages (cool → warm within family)
C_STAGE = [
    "#1565C0",  # deep blue
    "#1976D2",  # blue
    "#1E88E5",  # lighter blue
    "#039BE5",  # sky
    "#00ACC1",  # teal
    "#00897B",  # dark teal
    "#43A047",  # green
]

# Semantic accents
C_P50  = "#4FC3F7"   # light blue  — typical
C_P90  = "#FFB74D"   # amber       — elevated
C_P99  = "#FF8A65"   # deep orange — tail
C_P999 = "#EF5350"   # red         — extreme tail
C_MAX  = "#E53935"   # dark red    — worst case
C_GOOD = "#66BB6A"   # green       — healthy
C_WARN = "#FFA726"   # orange      — warning
C_BAD  = "#EF5350"   # red         — bad

PCTL_COLORS = [C_P50, C_P90, C_P99, C_P999, C_MAX]
PCTL_LABELS = ["p50", "p90", "p99", "p99.9", "max"]


def apply_style():
    plt.rcParams.update({
        "figure.facecolor":   BG,
        "axes.facecolor":     PANEL,
        "axes.edgecolor":     BORDER,
        "axes.grid":          True,
        "grid.color":         GRID,
        "grid.alpha":         0.7,
        "grid.linewidth":     0.4,
        "font.size":          10,
        "font.family":        "monospace",
        "text.color":         FG,
        "axes.titlecolor":    FG_BRIGHT,
        "axes.labelcolor":    FG,
        "axes.titleweight":   "bold",
        "axes.titlesize":     13,
        "axes.labelsize":     10,
        "xtick.color":        FG_DIM,
        "ytick.color":        FG_DIM,
        "xtick.labelsize":    9,
        "ytick.labelsize":    9,
        "legend.facecolor":   PANEL,
        "legend.edgecolor":   BORDER,
        "legend.labelcolor":  FG,
        "legend.fontsize":    9,
        "savefig.bbox":       "tight",
        "savefig.dpi":        200,
        "savefig.facecolor":  BG,
        "savefig.pad_inches": 0.4,
    })


# ═══════════════════════════════════════════════════════════════════════════════
# Helpers
# ═══════════════════════════════════════════════════════════════════════════════

def to_int(v, d=0):
    try: return int(v)
    except Exception: return d

def to_float(v, d=0.0):
    try: return float(v)
    except Exception: return d

def fmt_ns(ns: int) -> str:
    if ns >= 1_000_000: return f"{ns/1e6:.1f}ms"
    if ns >= 1_000:     return f"{ns/1e3:.1f}us"
    return f"{ns}ns"

def stat(bench_map, name, key):
    b = bench_map.get(name)
    if not b: return 0
    return max(0, to_int((b.get("stats") or {}).get(key, 0)))

def save(fig, d, name):
    fig.savefig(d / name)
    plt.close(fig)
    print(f"    {name}")

def watermark(fig):
    fig.text(0.985, 0.015, "LITHOS", transform=fig.transFigure,
             fontsize=7, color=FG_DIM, alpha=0.3, ha="right", va="bottom",
             family="monospace", weight="bold")


# ═══════════════════════════════════════════════════════════════════════════════
# CLI + data loading
# ═══════════════════════════════════════════════════════════════════════════════

def parse_args():
    p = argparse.ArgumentParser(description="Plot Lithos perf report")
    p.add_argument("--report", type=Path, default=None)
    p.add_argument("--results-dir", type=Path, default=DEFAULT_RESULTS_DIR)
    p.add_argument("--output-dir", type=Path, default=None)
    return p.parse_args()

def find_latest_report(d: Path) -> Path | None:
    reports = sorted(d.glob("*_report.json"))
    return reports[-1] if reports else None

def clean_dir(d: Path) -> int:
    n = 0
    for ext in ("*.png", "*.svg", "*.pdf"):
        for f in d.glob(ext):
            try: f.unlink(); n += 1
            except OSError: pass
    return n


# ═══════════════════════════════════════════════════════════════════════════════
# 01  Stage Cost Breakdown  (batched micro-benchmarks)
#     The hero chart — shows accurate per-operation cost
# ═══════════════════════════════════════════════════════════════════════════════

def plot_stage_breakdown(bench_map, out):
    stages = [
        ("parse_book_ticker_fast()", "parse_book_ticker_fast()"),
        ("parse_px/qty() ×4",       "parse_px/qty() ×4"),
        ("TopOfBook { .. }",         "TopOfBook { .. }"),
        ("writer.publish()",         "writer.publish()"),
        ("reader.try_read()",        "reader.try_read()"),
        ("update_market_state_tob()","update_market_state_tob()"),
    ]
    labels, p50s, p99s, maxs = [], [], [], []
    for key, label in stages:
        if key not in bench_map: continue
        labels.append(label)
        p50s.append(max(1, stat(bench_map, key, "p50")))
        p99s.append(max(1, stat(bench_map, key, "p99")))
        maxs.append(max(1, stat(bench_map, key, "max")))
    if not labels: return

    fig, ax = plt.subplots(figsize=(10, 4.8))
    y = range(len(labels))
    h = 0.25

    bars_50  = ax.barh([i + h for i in y], p50s, h, color=C_P50,  alpha=0.9, label="p50")
    bars_99  = ax.barh(list(y),            p99s, h, color=C_P99,  alpha=0.9, label="p99")
    bars_max = ax.barh([i - h for i in y], maxs, h, color=C_MAX,  alpha=0.7, label="max")

    # Annotate p50 values inline
    for i, v in enumerate(p50s):
        ax.text(v + 0.8, i + h, f"{v}ns", va="center", fontsize=8, color=C_P50, weight="bold")
    for i, v in enumerate(p99s):
        ax.text(v + 0.8, i, f"{v}ns", va="center", fontsize=8, color=C_P99)

    ax.set_yticks(list(y))
    ax.set_yticklabels(labels, fontsize=10)
    ax.set_xlabel("Latency (ns)")
    ax.invert_yaxis()
    ax.legend(loc="lower right", ncols=3)

    hot_path = stat(bench_map, "process_text()", "p50")
    title = "Per-Function Latency (batched, amortised)"
    if hot_path > 0:
        title += f"  |  process_text() p50 = {fmt_ns(hot_path)}"
    ax.set_title(title)
    watermark(fig)
    save(fig, out, "01_stage_breakdown.png")


# ═══════════════════════════════════════════════════════════════════════════════
# 02  Pipeline Waterfall  (cumulative p50 from batched data)
# ═══════════════════════════════════════════════════════════════════════════════

def plot_pipeline_waterfall(bench_map, out):
    stages = [
        ("parse_book_ticker_fast()", "parse_book_ticker_fast()", C_STAGE[0]),
        ("parse_px/qty() ×4",       "parse_px/qty() ×4",        C_STAGE[2]),
        ("TopOfBook { .. }",         "TopOfBook { .. }",         C_STAGE[3]),
        ("writer.publish()",         "writer.publish()",          C_STAGE[4]),
        ("reader.try_read()",        "reader.try_read()",        C_STAGE[5]),
        ("update_market_state_tob()","update_market_state_tob()",C_STAGE[6]),
    ]
    labels, vals, colors = [], [], []
    for key, label, c in stages:
        v = stat(bench_map, key, "p50")
        if key not in bench_map: continue
        labels.append(label)
        vals.append(max(1, v))
        colors.append(c)
    if not labels: return

    total = sum(vals)
    fig, ax = plt.subplots(figsize=(10, 3.5))

    # Draw cumulative waterfall
    lefts = []
    cum = 0
    for v in vals:
        lefts.append(cum)
        cum += v

    bars = ax.barh(["pipeline"], [total], color=PANEL, edgecolor=BORDER, height=0.5)

    # Stack segments
    for i, (v, left, c, label) in enumerate(zip(vals, lefts, colors, labels)):
        ax.barh(["pipeline"], [v], left=left, color=c, height=0.5, alpha=0.9)
        if v >= total * 0.04:  # only label segments > 4%
            ax.text(left + v/2, 0, f"{label}\n{v}ns", ha="center", va="center",
                    fontsize=8, color=FG_BRIGHT, weight="bold")

    ax.set_xlabel("Cumulative Latency (ns)")
    ax.set_title(f"Pipeline Waterfall @ p50  (total: {fmt_ns(total)})")
    ax.set_xlim(0, total * 1.05)

    # Legend
    from matplotlib.patches import Patch
    handles = [Patch(facecolor=c, label=f"{l} ({v}ns)") for l, v, c in zip(labels, vals, colors)]
    ax.legend(handles=handles, loc="upper right", ncols=3, fontsize=8)

    watermark(fig)
    save(fig, out, "02_pipeline_waterfall.png")


# ═══════════════════════════════════════════════════════════════════════════════
# 03  Component Percentile Matrix  (all batched benchmarks, p50 → max)
# ═══════════════════════════════════════════════════════════════════════════════

def plot_component_matrix(bench_map, out):
    ordered = [
        "parse_book_ticker_fast()",
        "parse_px/qty() ×4",
        "TopOfBook { .. }",
        "writer.publish()",
        "reader.try_read()",
        "update_market_state_tob()",
        "process_text()",
        "read→update()",
        "pipeline e2e",
        "soak_latency",
    ]
    rows = [n for n in ordered if n in bench_map]
    if not rows: return

    cols = ["p50", "p90", "p99", "p999", "max"]
    col_labels = ["p50", "p90", "p99", "p99.9", "max"]
    matrix = [[max(1, stat(bench_map, r, c)) for c in cols] for r in rows]
    flat = [v for row in matrix for v in row]
    vmin, vmax = max(1, min(flat)), max(flat)

    fig, ax = plt.subplots(figsize=(10, max(3.5, 0.42 * len(rows))))

    cmap = LinearSegmentedColormap.from_list("lat", [
        "#0D2137", "#164E6B", "#1B7D8E", "#D29922", "#E55934", "#C62828"
    ])
    from matplotlib.colors import LogNorm
    norm = LogNorm(vmin=vmin, vmax=vmax)
    im = ax.imshow(matrix, cmap=cmap, aspect="auto", norm=norm)

    ax.set_xticks(range(len(cols)))
    ax.set_xticklabels(col_labels)
    ax.set_yticks(range(len(rows)))
    ax.set_yticklabels(rows, fontsize=9)

    for i, row in enumerate(matrix):
        for j, v in enumerate(row):
            brightness = (v - vmin) / max(1, vmax - vmin)
            tc = FG_BRIGHT if brightness > 0.15 else FG_DIM
            ax.text(j, i, fmt_ns(v), ha="center", va="center", fontsize=8, color=tc)

    ax.set_title("Percentile Matrix (all benchmarks)")
    cbar = fig.colorbar(im, ax=ax, shrink=0.8, pad=0.02)
    cbar.set_label("ns", color=FG)
    cbar.ax.yaxis.set_tick_params(color=FG_DIM)
    plt.setp(plt.getp(cbar.ax.axes, "yticklabels"), color=FG_DIM)
    watermark(fig)
    save(fig, out, "03_percentile_matrix.png")


# ═══════════════════════════════════════════════════════════════════════════════
# 04  E2E Latency Profile  (pipeline e2e percentile ladder)
# ═══════════════════════════════════════════════════════════════════════════════

def plot_e2e_profile(bench_map, out):
    paths = [
        ("pipeline e2e",   "Cross-thread e2e",     C_P50),
        ("soak_latency",   "Soak (sustained)",      C_WARN),
        ("process_text()", "Obsidian process_text()", C_STAGE[2]),
        ("read→update()",  "Onyx read→update()",    C_STAGE[5]),
    ]
    pctls = ["p50", "p75", "p90", "p95", "p99", "p999", "p9999", "max"]
    pctl_labels = ["p50", "p75", "p90", "p95", "p99", "p99.9", "p99.99", "max"]

    fig, ax = plt.subplots(figsize=(10, 5))
    plotted = 0

    for key, label, color in paths:
        if key not in bench_map: continue
        vals = [max(1, stat(bench_map, key, p)) for p in pctls]
        ax.plot(range(len(pctls)), vals, marker="o", markersize=5, linewidth=2,
                color=color, label=label, alpha=0.9)
        # Annotate key points
        for i in [0, 4, 6, 7]:  # p50, p99, p99.99, max
            if i < len(vals):
                ax.annotate(fmt_ns(vals[i]), (i, vals[i]),
                            textcoords="offset points", xytext=(6, 4),
                            fontsize=7, color=color, alpha=0.9)
        plotted += 1

    if plotted == 0:
        plt.close(fig); return

    ax.set_yscale("log")
    ax.set_xticks(range(len(pctls)))
    ax.set_xticklabels(pctl_labels, rotation=45, ha="right")
    ax.set_ylabel("Latency (ns)")
    ax.set_title("Latency Percentile Profile")
    ax.legend(loc="upper left")

    # Shade regions
    ylim = ax.get_ylim()
    ax.axhspan(ylim[0], 100,   color=C_GOOD, alpha=0.03)
    ax.axhspan(100,     1000,  color=C_WARN, alpha=0.03)
    ax.axhspan(1000,    ylim[1], color=C_BAD, alpha=0.03)
    ax.set_ylim(ylim)

    watermark(fig)
    save(fig, out, "04_latency_profile.png")


# ═══════════════════════════════════════════════════════════════════════════════
# 05  Tail Amplification  (ratio table: p99/p50, p99.9/p50, max/p50)
# ═══════════════════════════════════════════════════════════════════════════════

def plot_tail_amplification(bench_map, out):
    targets = [
        "process_text()",
        "pipeline e2e",
        "soak_latency",
    ]
    rows = [t for t in targets if t in bench_map and stat(bench_map, t, "p50") > 0]
    if not rows: return

    cols = ["p99/p50", "p99.9/p50", "max/p50"]
    matrix = []
    for r in rows:
        p50 = max(1, stat(bench_map, r, "p50"))
        matrix.append([
            stat(bench_map, r, "p99")  / p50,
            stat(bench_map, r, "p999") / p50,
            stat(bench_map, r, "max")  / p50,
        ])

    vmax = max(max(row) for row in matrix)

    fig, ax = plt.subplots(figsize=(8, max(3, 0.7 * len(rows))))

    cmap = LinearSegmentedColormap.from_list("amp", [PANEL, "#1B4F72", C_WARN, C_BAD])
    im = ax.imshow(matrix, cmap=cmap, aspect="auto", vmin=1.0, vmax=max(3.0, vmax))

    ax.set_xticks(range(len(cols)))
    ax.set_xticklabels(cols, fontsize=10)
    ax.set_yticks(range(len(rows)))
    ax.set_yticklabels(rows, fontsize=9)
    ax.set_title("Tail Amplification Factor")

    for i, row in enumerate(matrix):
        for j, v in enumerate(row):
            color = C_BAD if v > 10 else (C_WARN if v > 3 else FG_BRIGHT)
            ax.text(j, i, f"{v:.1f}x", ha="center", va="center",
                    fontsize=11, color=color, weight="bold")

    cbar = fig.colorbar(im, ax=ax, shrink=0.85, pad=0.03)
    cbar.set_label("Multiplier", color=FG)
    cbar.ax.yaxis.set_tick_params(color=FG_DIM)
    plt.setp(plt.getp(cbar.ax.axes, "yticklabels"), color=FG_DIM)
    watermark(fig)
    save(fig, out, "05_tail_amplification.png")


# ═══════════════════════════════════════════════════════════════════════════════
# 06  Soak Stability  (throughput over time with control bands)
# ═══════════════════════════════════════════════════════════════════════════════

def plot_soak_stability(data, bench_map, out):
    windows = (data.get("soak") or {}).get("windows") or []
    if not windows: return

    x = [to_int(w.get("second")) for w in windows]
    y = [to_float(w.get("throughput_meps")) for w in windows]
    if not y or max(y) == 0: return

    mean = sum(y) / len(y)
    var  = sum((v - mean)**2 for v in y) / len(y)
    sd   = math.sqrt(var)
    cv   = (sd / mean * 100) if mean > 0 else 0

    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(13, 4.5), gridspec_kw={"width_ratios": [2, 1]})

    # Left: throughput timeline
    ax1.fill_between(x, mean - sd, mean + sd, color=C_P50, alpha=0.08)
    ax1.axhline(mean, color=C_P50, linestyle="--", linewidth=1.2, alpha=0.6)

    for i in range(len(x)):
        dev = abs(y[i] - mean)
        c = C_GOOD if dev <= sd else (C_WARN if dev <= 2*sd else C_BAD)
        ax1.plot(x[i], y[i], "o", color=c, markersize=7, zorder=5)
    ax1.plot(x, y, linewidth=1.5, color=C_P50, alpha=0.4, zorder=4)

    ax1.set_xlabel("Second")
    ax1.set_ylabel("Throughput (M events/s)")
    ax1.set_title(f"Soak Throughput  (mean={mean:.2f} M/s, CV={cv:.1f}%)")

    # Right: latency summary card
    soak_stats = bench_map.get("soak_latency", {}).get("stats", {})
    if soak_stats:
        metrics = [
            ("p50",   to_int(soak_stats.get("p50")),   C_P50),
            ("p90",   to_int(soak_stats.get("p90")),   C_P90),
            ("p99",   to_int(soak_stats.get("p99")),   C_P99),
            ("p99.9", to_int(soak_stats.get("p999")),  C_P999),
            ("max",   to_int(soak_stats.get("max")),   C_MAX),
        ]
        ax2.set_xlim(0, 1)
        ax2.set_ylim(-0.5, len(metrics) - 0.5)
        ax2.invert_yaxis()
        ax2.axis("off")
        ax2.set_title("Soak Latency")
        for i, (label, val, color) in enumerate(metrics):
            ax2.text(0.15, i, label, fontsize=11, color=FG_DIM, va="center", family="monospace")
            ax2.text(0.55, i, fmt_ns(val), fontsize=13, color=color, va="center",
                     family="monospace", weight="bold")

    watermark(fig)
    save(fig, out, "06_soak_stability.png")


# ═══════════════════════════════════════════════════════════════════════════════
# 07  System & Pipeline Summary Card
# ═══════════════════════════════════════════════════════════════════════════════

def plot_summary_card(data, bench_map, out):
    sys_info = data.get("system", {})
    resources = data.get("resources", {})
    delta = resources.get("delta", {})

    fig, axes = plt.subplots(1, 3, figsize=(14, 4.5))
    for ax in axes:
        ax.axis("off")
        ax.set_xlim(0, 1)

    # Column 1: System
    ax = axes[0]
    ax.set_title("System", fontsize=12, color=FG_BRIGHT, weight="bold", loc="left", pad=10)
    sys_lines = [
        ("CPU",   sys_info.get("cpu_brand", "?")),
        ("Cores", str(sys_info.get("ncpu", "?"))),
        ("L1d",   fmt_bytes(sys_info.get("l1d_bytes", 0))),
        ("L2",    fmt_bytes(sys_info.get("l2_bytes", 0))),
        ("Line",  f"{sys_info.get('line_size', '?')}B"),
    ]
    for i, (k, v) in enumerate(sys_lines):
        ax.text(0.05, 0.85 - i*0.17, k, fontsize=10, color=FG_DIM, va="top")
        ax.text(0.35, 0.85 - i*0.17, v, fontsize=10, color=FG, va="top")

    # Column 2: Pipeline Latency
    ax = axes[1]
    ax.set_title("Pipeline Latency", fontsize=12, color=FG_BRIGHT, weight="bold", loc="left", pad=10)
    obs_p50 = stat(bench_map, "process_text()", "p50")
    onyx_p50 = stat(bench_map, "read→update()", "p50")
    lat_lines = [
        ("Obsidian p50",  obs_p50,  C_STAGE[2]),
        ("Onyx p50",      onyx_p50, C_STAGE[5]),
        ("Sum p50",       obs_p50 + onyx_p50 if obs_p50 and onyx_p50 else 0, C_P50),
        ("E2E p50",       stat(bench_map, "pipeline e2e", "p50"), C_P50),
        ("E2E p99",       stat(bench_map, "pipeline e2e", "p99"), C_P99),
    ]
    for i, (k, v, c) in enumerate(lat_lines):
        ax.text(0.05, 0.85 - i*0.17, k, fontsize=10, color=FG_DIM, va="top")
        ax.text(0.65, 0.85 - i*0.17, fmt_ns(v) if v > 0 else "-", fontsize=11,
                color=c, va="top", weight="bold")

    # Column 3: Resources
    ax = axes[2]
    ax.set_title("Resources", fontsize=12, color=FG_BRIGHT, weight="bold", loc="left", pad=10)
    rss = resources.get("end", {}).get("max_rss_bytes", 0)
    res_lines = [
        ("Peak RSS",     fmt_bytes(rss)),
        ("Minor faults", str(delta.get("minor_faults", "?"))),
        ("Ctx switches", str(delta.get("invol_ctx_switches", "?"))),
        ("User CPU",     f"{delta.get('user_time_us', 0)/1e6:.2f}s"),
        ("Sys CPU",      f"{delta.get('sys_time_us', 0)/1e6:.3f}s"),
    ]
    for i, (k, v) in enumerate(res_lines):
        ax.text(0.05, 0.85 - i*0.17, k, fontsize=10, color=FG_DIM, va="top")
        ax.text(0.60, 0.85 - i*0.17, v, fontsize=10, color=FG, va="top")

    fig.suptitle("Lithos Performance Summary", fontsize=14, color=FG_BRIGHT,
                 weight="bold", y=1.02)
    watermark(fig)
    save(fig, out, "07_summary.png")


def fmt_bytes(b):
    b = to_int(b)
    if b >= 1024**3: return f"{b/1024**3:.1f} GB"
    if b >= 1024**2: return f"{b/1024**2:.1f} MB"
    if b >= 1024:    return f"{b/1024:.1f} KB"
    return f"{b} B"


# ═══════════════════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════════════════

def main():
    apply_style()
    args = parse_args()

    report = args.report or find_latest_report(args.results_dir)
    if report is None:
        print("Error: no *_report.json found. Run perf_report first.")
        sys.exit(1)

    out = args.output_dir or (args.results_dir / "plots")
    out.mkdir(parents=True, exist_ok=True)
    removed = clean_dir(out)
    if removed > 0:
        print(f"  Cleaned {removed} old files")

    print(f"  Report: {report.name}")
    with open(report, "r") as f:
        data = json.load(f)

    benches = (data.get("component_benchmarks") or data.get("stage_benchmarks")
               or data.get("benchmarks") or [])
    bench_map = {b["name"]: b for b in benches if "name" in b}

    # Also add cross_thread stats
    cross = data.get("cross_thread", {}).get("stats")
    if cross and "pipeline e2e" not in bench_map:
        bench_map["pipeline e2e"] = {"name": "pipeline e2e", "stats": cross}

    print("  Charts:")
    plot_stage_breakdown(bench_map, out)
    plot_pipeline_waterfall(bench_map, out)
    plot_component_matrix(bench_map, out)
    plot_e2e_profile(bench_map, out)
    plot_tail_amplification(bench_map, out)
    plot_soak_stability(data, bench_map, out)
    plot_summary_card(data, bench_map, out)

    print(f"\n  All charts → {out}")


if __name__ == "__main__":
    main()
