#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PERF_DIR="$(dirname "$SCRIPT_DIR")"
ROOT_DIR="$(dirname "$PERF_DIR")"
RESULTS_DIR="$PERF_DIR/results"

mkdir -p "$RESULTS_DIR"

usage() {
    cat <<EOF
Lithos Performance Suite

Usage: $(basename "$0") [mode] [options]

Modes:
  perf          Full suite: build + criterion + report + plot (default)
  bench         Criterion micro-benchmarks only
  plot          Generate plots from latest results

Options:
  --flamegraph  Generate flamegraph (requires sudo, perf mode only)
  -h, --help    Show this help

Examples:
  $(basename "$0")              # full suite
  $(basename "$0") perf         # full suite (explicit)
  $(basename "$0") bench        # criterion only
  $(basename "$0") plot         # plot latest results
EOF
    exit 0
}

# ── Parse args ──────────────────────────────────────────────────────────────

MODE="perf"
FLAMEGRAPH=false

for arg in "$@"; do
    case "$arg" in
        perf|bench|plot) MODE="$arg" ;;
        --flamegraph)    FLAMEGRAPH=true ;;
        -h|--help)       usage ;;
        *)               echo "Unknown argument: $arg"; usage ;;
    esac
done

# ── Helpers ─────────────────────────────────────────────────────────────────

build_release() {
    echo "[build] Compiling release binaries..."
    cargo build --release -p lithos-perf
    echo ""
}

run_report() {
    echo "[report] Running perf report (Obsidian + Onyx pipeline)..."
    "$ROOT_DIR/target/release/perf_report" 2>&1
    echo ""
}

run_criterion() {
    echo "[bench] Running criterion micro-benchmarks..."
    cargo bench -p lithos-perf 2>&1
    echo ""
}

run_criterion_hot_path() {
    echo "[bench] Running criterion bench_hot_path (for report inputs)..."
    cargo bench -p lithos-perf --bench bench_hot_path 2>&1
    echo ""
}

run_plots() {
    echo "[plot] Generating charts from latest results..."
    if ! command -v python3 &>/dev/null; then
        echo "  Skipped: python3 not found"
        return
    fi
    if ! python3 -c "import matplotlib" 2>/dev/null; then
        echo "  Skipped: matplotlib not installed (pip3 install matplotlib)"
        return
    fi
    python3 "$SCRIPT_DIR/plot_results.py"
    echo "  Charts saved to: $RESULTS_DIR/plots/"
    echo ""
}

run_flamegraph() {
    if [ "$FLAMEGRAPH" != true ]; then return; fi
    echo "[flamegraph] Generating flamegraph (requires sudo)..."
    if command -v cargo-flamegraph &>/dev/null || cargo install --list | grep -q flamegraph; then
        sudo cargo flamegraph --bin perf_report -p lithos-perf -o "$RESULTS_DIR/flamegraph.svg" -- 2>&1
        echo "  Saved: $RESULTS_DIR/flamegraph.svg"
    else
        echo "  Skipped: cargo-flamegraph not installed (cargo install flamegraph)"
    fi
    echo ""
}

print_summary() {
    echo "── Results ──────────────────────────────────────────────"
    [ -d "$RESULTS_DIR" ] && {
        LATEST_JSON=$(ls -t "$RESULTS_DIR"/*_report.json 2>/dev/null | head -1 || true)
        [ -n "$LATEST_JSON" ] && echo "  Report:    $LATEST_JSON"
    }
    [ -d "$ROOT_DIR/target/criterion" ] && echo "  Criterion: $ROOT_DIR/target/criterion/"
    [ -d "$RESULTS_DIR/plots" ] && echo "  Charts:    $RESULTS_DIR/plots/"
    echo ""
}

# ── Execute ─────────────────────────────────────────────────────────────────

echo ""
echo "=== Lithos Performance Suite [mode: $MODE] ==="
echo ""

case "$MODE" in
    perf)
        build_release
        echo "[note] Running bench_hot_path first — perf_report reads its criterion JSON"
        run_criterion_hot_path
        run_report
        run_plots
        run_flamegraph
        print_summary
        ;;
    bench)
        build_release
        run_criterion
        print_summary
        ;;
    plot)
        run_plots
        print_summary
        ;;
esac

echo "Done."
