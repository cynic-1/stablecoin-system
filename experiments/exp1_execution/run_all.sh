#!/bin/bash
# Experiment 1: Execution Engine Comparison
# Runs LEAP benchmark and saves raw CSV data.
#
# NUMA TIP: On multi-socket machines, pin to one NUMA node for best results:
#   numactl --cpunodebind=0 --membind=0 bash run_all.sh
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results/raw"

mkdir -p "$RESULTS_DIR"

# Detect NUMA topology and warn if multi-socket.
NUMA_NODES=$(lscpu 2>/dev/null | grep "NUMA node(s):" | awk '{print $NF}' || echo "1")
if [ "$NUMA_NODES" -gt 1 ] 2>/dev/null; then
    echo "WARNING: $NUMA_NODES NUMA nodes detected. For best results:"
    echo "  numactl --cpunodebind=0 --membind=0 bash $0"
    echo ""
fi

echo "=== Building LEAP in release mode ==="
cd "$PROJECT_DIR/leap"
cargo build --release --bin leap_benchmark --bin exp1_accounts 2>&1

echo ""
echo "=== Running LEAP full benchmark suite ==="
cargo run --release --bin leap_benchmark -- "$RESULTS_DIR/exp1_execution.csv" 2>&1

echo ""
echo "=== Running LEAP account-sweep benchmark ==="
cargo run --release --bin exp1_accounts -- "$RESULTS_DIR/exp1_accounts.csv" 2>&1

echo ""
echo "=== Benchmark complete ==="
echo "Raw CSV data:"
echo "  $RESULTS_DIR/exp1_execution.csv"
echo "  $RESULTS_DIR/exp1_accounts.csv"
echo "To generate plots: cd $SCRIPT_DIR && python3 plot.py"
