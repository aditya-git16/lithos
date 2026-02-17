#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PERF_DIR="$(dirname "$SCRIPT_DIR")"
ROOT_DIR="$(dirname "$PERF_DIR")"
RESULTS_DIR="$PERF_DIR/results"

mkdir -p "$RESULTS_DIR"

echo "=== Lithos Performance Suite ==="
echo ""

FLAMEGRAPH=false
for arg in "$@"; do
    if [ "$arg" = "--flamegraph" ]; then
        FLAMEGRAPH=true
    fi
done

# 1. Build release
echo "[1/4] Building release binaries..."
cargo build --release -p lithos-perf
echo "     Done."
echo ""

# 2. Run perf_report (real pipeline instrumentation)
echo "[2/4] Running perf report (real pipeline)..."
REPORT_OUTPUT="$RESULTS_DIR/$(date +%Y%m%d_%H%M%S)_stdout.txt"
"$ROOT_DIR/target/release/perf_report" 2>&1 | tee "$REPORT_OUTPUT"
echo ""
echo "     Report saved to: $REPORT_OUTPUT"
echo ""

# 3. Run criterion benchmarks
echo "[3/4] Running criterion benchmarks..."
cargo bench -p lithos-perf 2>&1
echo ""

# 4. Plot results (optional)
echo "[4/4] Generating plots..."
if command -v python3 &>/dev/null; then
    if python3 -c "import matplotlib" 2>/dev/null; then
        python3 "$SCRIPT_DIR/plot_results.py"
        echo "     Plots saved to: $RESULTS_DIR/plots/"
    else
        echo "     Skipped: matplotlib not installed (pip3 install matplotlib)"
    fi
else
    echo "     Skipped: python3 not found"
fi

# Optional flamegraph
if [ "$FLAMEGRAPH" = true ]; then
    echo ""
    echo "[extra] Generating flamegraph (requires sudo for dtrace on macOS)..."
    if command -v cargo-flamegraph &>/dev/null || cargo install --list | grep -q flamegraph; then
        sudo cargo flamegraph --bin perf_report -p lithos-perf -o "$RESULTS_DIR/flamegraph.svg" -- 2>&1
        echo "     Flamegraph saved to: $RESULTS_DIR/flamegraph.svg"
    else
        echo "     Skipped: cargo-flamegraph not installed (cargo install flamegraph)"
    fi
fi

echo ""
echo "=== Results ==="
echo "  Perf report:      $REPORT_OUTPUT"
echo "  JSON results:     $RESULTS_DIR/*_report.json"
echo "  Criterion HTML:   $ROOT_DIR/target/criterion/"
if [ -d "$RESULTS_DIR/plots" ]; then
    echo "  Plots:            $RESULTS_DIR/plots/"
fi
echo ""
echo "Done."
