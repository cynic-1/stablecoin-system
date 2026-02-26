#!/usr/bin/env python3
"""
Generate plots for Experiment 2: MP3-BFT++ Consensus.
Reads raw CSV data from results/raw/exp2_simulator.csv.
"""
import csv
import os
import sys
from collections import defaultdict

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
RESULTS_DIR = os.path.join(SCRIPT_DIR, 'results')
RAW_DIR = os.path.join(RESULTS_DIR, 'raw')
PLOTS_DIR = os.path.join(RESULTS_DIR, 'plots')
CSV_PATH = os.path.join(RAW_DIR, 'exp2_simulator.csv')

plt.rcParams.update({
    'font.size': 14,
    'figure.figsize': (8, 5),
    'axes.grid': True,
    'grid.alpha': 0.3,
})

os.makedirs(PLOTS_DIR, exist_ok=True)

if not os.path.exists(CSV_PATH):
    print(f"ERROR: CSV file not found: {CSV_PATH}")
    print("Run './run_all.sh' first to generate benchmark data.")
    sys.exit(1)

# Read CSV: nodes, quorum, k, committed_blocks, committed_txns, time_s, tps
# Structure: tps_data[nodes][k] = tps
tps_data = defaultdict(dict)

with open(CSV_PATH) as f:
    reader = csv.DictReader(f)
    for row in reader:
        n = int(row['nodes'])
        k = int(row['k'])
        tps = float(row['tps'])
        tps_data[n][k] = tps

node_counts = sorted(tps_data.keys())
k_values = sorted({k for d in tps_data.values() for k in d.keys()})

# --- Exp 2C: TPS vs parallel slots k ---
NODE_COLORS = {4: '#ED7D31', 10: '#4472C4', 20: '#70AD47'}
NODE_MARKERS = {4: 'o', 10: 's', 20: '^'}

fig, ax = plt.subplots()
for n in node_counts:
    ks = sorted(tps_data[n].keys())
    tps_vals = [tps_data[n][k] / 1e3 for k in ks]
    ax.plot(ks, tps_vals,
            marker=NODE_MARKERS.get(n, 'o'),
            color=NODE_COLORS.get(n, '#333'),
            linestyle='-', linewidth=2, markersize=8,
            label=f'n={n}')
ax.set_xlabel('Parallel Slots (k)')
ax.set_ylabel('Throughput (K TPS)')
ax.set_title('Exp 2C: TPS vs Parallel Slots k (200ms RTT)')
ax.legend()
ax.set_xticks(k_values)
fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp2c_tps_vs_k.png'), dpi=150)
plt.close(fig)

# --- Exp 2A: TPS vs node count ---
fig, ax = plt.subplots()
for k_plot, style, color, label in [
    (1, '--', '#4472C4', 'k=1 (Tusk-like)'),
    (8, '-', '#ED7D31', 'k=8 (MP3-BFT++)'),
    (16, '-', '#70AD47', 'k=16 (MP3-BFT++)'),
]:
    ns = [n for n in node_counts if k_plot in tps_data[n]]
    tps_vals = [tps_data[n][k_plot] / 1e3 for n in ns]
    if ns:
        ax.plot(ns, tps_vals, marker='o', linestyle=style,
                color=color, linewidth=2, markersize=8, label=label)
ax.set_xlabel('Number of Nodes (n)')
ax.set_ylabel('Throughput (K TPS)')
ax.set_title('Exp 2A: TPS vs Node Count (200ms RTT)')
ax.legend()
ax.set_xticks(node_counts)
fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp2a_tps_vs_nodes.png'), dpi=150)
plt.close(fig)

print(f"Plots saved to {PLOTS_DIR}/")
print(f"Source data: {CSV_PATH}")
