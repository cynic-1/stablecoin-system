#!/bin/bash
set -e

cd "$(dirname "$0")/../../leap"

echo "Building exp1_accounts..."
cargo build --release --bin exp1_accounts

echo "Running exp1_accounts..."
cargo run --release --bin exp1_accounts -- ../experiments/exp1_execution/results/raw/exp1_accounts.csv
