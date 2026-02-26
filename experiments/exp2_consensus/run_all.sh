#!/bin/bash
# Experiment 2: MP3-BFT++ Consensus Benchmark
# Default: runs real distributed Narwhal benchmark (4 nodes, TCP, Ed25519).
# For simulator-only quick test, use: ./run_simulator.sh
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results/raw"

mkdir -p "$RESULTS_DIR"

echo "=== Running real Narwhal distributed benchmark ==="
echo "This starts 4-node clusters with real TCP + Ed25519 crypto."
echo "Results will be saved to: $RESULTS_DIR/"
echo ""

cd "$PROJECT_DIR/narwhal/benchmark"
pip install -r requirements.txt -q 2>/dev/null || true
python3 run_comparison.py

echo ""
echo "=== Benchmark complete ==="
echo "Raw CSV data: $RESULTS_DIR/exp2_all_results.csv"
echo "To generate plots: cd $SCRIPT_DIR && python3 plot_narwhal.py"
