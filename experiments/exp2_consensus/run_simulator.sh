#!/bin/bash
# Experiment 2 (Simulator): MP3-BFT++ single-process simulator.
# NOTE: This produces theoretical/simulated results, NOT real distributed benchmark data.
# For real benchmark data, use: ./run_all.sh
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results/raw"

mkdir -p "$RESULTS_DIR"

echo "=== Building mp3bft simulator ==="
cd "$PROJECT_DIR/mp3bft"
cargo build --release --bin mp3bft_benchmark 2>&1

echo ""
echo "=== Running MP3-BFT++ simulator benchmark ==="
echo "WARNING: This is a single-process simulation, not a real distributed benchmark."
cargo run --release --bin mp3bft_benchmark -- "$RESULTS_DIR/exp2_simulator.csv" 2>&1

echo ""
echo "=== Simulator benchmark complete ==="
echo "Raw CSV data: $RESULTS_DIR/exp2_simulator.csv"
