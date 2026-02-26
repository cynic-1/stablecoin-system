#!/bin/bash
# Experiment 3: Real End-to-End Pipeline Benchmark
# Runs Narwhal consensus + LEAP execution as an integrated system.
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results/raw"
BENCHMARK_DIR="$PROJECT_DIR/narwhal/benchmark"

mkdir -p "$RESULTS_DIR"

echo "=== Building narwhal with e2e_exec + mp3bft features ==="
cd "$PROJECT_DIR/narwhal"
cargo build --quiet --release --features benchmark,e2e_exec,mp3bft 2>&1

echo ""
echo "=== Running E2E benchmarks ==="
cd "$BENCHMARK_DIR"
python3 run_e2e.py "$@" 2>&1

echo ""
echo "=== Generating plots ==="
cd "$SCRIPT_DIR"
python3 plot.py 2>&1

echo ""
echo "=== Experiment 3 complete ==="
echo "Raw CSV data: $RESULTS_DIR/exp3_e2e_realistic.csv"
echo "Plots: $SCRIPT_DIR/results/plots/"
