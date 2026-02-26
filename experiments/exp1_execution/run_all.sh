#!/bin/bash
# Experiment 1: Execution Engine Comparison
# Runs LEAP benchmark and saves raw CSV data.
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results/raw"

mkdir -p "$RESULTS_DIR"

echo "=== Building LEAP in release mode ==="
cd "$PROJECT_DIR/leap"
cargo build --release --bin leap_benchmark 2>&1

echo ""
echo "=== Running LEAP benchmark suite ==="
cargo run --release --bin leap_benchmark -- "$RESULTS_DIR/exp1_execution_v5.csv" 2>&1

echo ""
echo "=== Benchmark complete ==="
echo "Raw CSV data: $RESULTS_DIR/exp1_execution_v5.csv"
echo "To generate plots: cd $SCRIPT_DIR && python3 plot.py"
