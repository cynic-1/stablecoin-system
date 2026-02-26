#!/usr/bin/env python3
"""
Generate plots for Experiment 3: Real End-to-End Pipeline.
Reads data from results/raw/exp3_e2e_realistic.csv produced by
narwhal/benchmark/run_e2e.py (real integrated benchmarks).
"""
import csv
import os
import sys
from collections import defaultdict

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import numpy as np

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
RESULTS_DIR = os.path.join(SCRIPT_DIR, 'results')
RAW_DIR = os.path.join(RESULTS_DIR, 'raw')
PLOTS_DIR = os.path.join(RESULTS_DIR, 'plots')
CSV_PATH = os.path.join(RAW_DIR, 'exp3_e2e_realistic.csv')

plt.rcParams.update({
    'font.size': 14,
    'figure.figsize': (8, 5),
    'axes.grid': True,
    'grid.alpha': 0.3,
})

COLORS = {
    'MP3+LEAP':      '#ED7D31',
    'Tusk+LeapBase': '#4472C4',
    'Tusk+Serial':   '#999999',
}

MARKERS = {
    'MP3+LEAP':      'o',
    'Tusk+LeapBase': 's',
    'Tusk+Serial':   '^',
}

os.makedirs(PLOTS_DIR, exist_ok=True)

if not os.path.exists(CSV_PATH):
    print(f"ERROR: CSV file not found: {CSV_PATH}")
    print("Run 'cd narwhal/benchmark && python3 run_e2e.py' first.")
    sys.exit(1)

# Read CSV and average across runs
rows = []
with open(CSV_PATH) as f:
    reader = csv.DictReader(f)
    for row in reader:
        rows.append(row)


def average_rows(row_list, group_keys, value_keys):
    """Group rows by group_keys and average value_keys."""
    grouped = defaultdict(list)
    for r in row_list:
        key = tuple(r[k] for k in group_keys)
        grouped[key].append(r)
    result = []
    for key, runs in sorted(grouped.items()):
        avg = {k: v for k, v in zip(group_keys, key)}
        for vk in value_keys:
            vals = [float(r[vk]) for r in runs if r.get(vk)]
            avg[vk] = sum(vals) / len(vals) if vals else 0
        result.append(avg)
    return result


VALUE_KEYS = [
    'consensus_tps', 'consensus_latency_ms',
    'with_exec_tps', 'with_exec_latency_ms',
    'e2e_tps', 'e2e_latency_ms',
]

# ============================================================
# E2E-1: Throughput-Latency vs Input Rate (2 subplots)
# ============================================================
e2e1_raw = [r for r in rows if r['experiment'] == 'E2E-1']
e2e1 = average_rows(e2e1_raw, ['system', 'variable'], VALUE_KEYS)

e2e1_by_system = defaultdict(lambda: {'rates': [], 'with_exec_tps': [],
                                       'cons_lat': [], 'exec_lat': []})
for r in e2e1:
    sys_name = r['system']
    e2e1_by_system[sys_name]['rates'].append(int(float(r['variable'])) // 1000)
    e2e1_by_system[sys_name]['with_exec_tps'].append(r['with_exec_tps'])
    e2e1_by_system[sys_name]['cons_lat'].append(r['consensus_latency_ms'])
    # Execution overhead = with_exec_latency - consensus_latency
    exec_overhead = max(0, r['with_exec_latency_ms'] - r['consensus_latency_ms'])
    e2e1_by_system[sys_name]['exec_lat'].append(exec_overhead)

fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(14, 5))

# Subplot 1: TPS vs rate
for sys_name, d in e2e1_by_system.items():
    color = COLORS.get(sys_name, '#333')
    marker = MARKERS.get(sys_name, 'o')
    ls = '-' if 'Serial' not in sys_name else '--'
    ax1.plot(d['rates'], [t / 1000 for t in d['with_exec_tps']],
             marker=marker, linestyle=ls, color=color, label=sys_name,
             linewidth=2, markersize=8)

ax1.set_xlabel('Input Rate (K tx/s)')
ax1.set_ylabel('With-Execution Throughput (K TPS)')
ax1.set_title('E2E-1: Throughput vs Input Rate')
ax1.legend(fontsize=11)

# Subplot 2: Latency breakdown (stacked bar)
systems_order = ['MP3+LEAP', 'Tusk+LeapBase', 'Tusk+Serial']
systems_present = [s for s in systems_order if s in e2e1_by_system]

if systems_present:
    rates = e2e1_by_system[systems_present[0]]['rates']
    n_rates = len(rates)
    n_sys = len(systems_present)
    width = 0.8 / n_sys
    x = np.arange(n_rates)

    for i, sys_name in enumerate(systems_present):
        d = e2e1_by_system[sys_name]
        color = COLORS.get(sys_name, '#333')
        offset = (i - n_sys / 2 + 0.5) * width

        cons_lat = d['cons_lat'][:n_rates]
        exec_lat = d['exec_lat'][:n_rates]

        ax2.bar(x + offset, cons_lat, width, color=color, alpha=0.7,
                label=f'{sys_name} (consensus)')
        ax2.bar(x + offset, exec_lat, width, bottom=cons_lat, color=color,
                alpha=0.4, hatch='//', label=f'{sys_name} (execution)')

    ax2.set_xlabel('Input Rate (K tx/s)')
    ax2.set_ylabel('Latency (ms)')
    ax2.set_title('E2E-1: Latency Breakdown')
    ax2.set_xticks(x)
    ax2.set_xticklabels([f'{r}K' for r in rates])
    handles, labels = ax2.get_legend_handles_labels()
    by_label = dict(zip(labels, handles))
    ax2.legend(by_label.values(), by_label.keys(), fontsize=9, ncol=2)

fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp3_e2e1_throughput_latency.png'), dpi=150)
plt.close(fig)

# ============================================================
# E2E-2: Conflict Pattern Impact (2 subplots)
# ============================================================
e2e2_raw = [r for r in rows if r['experiment'] == 'E2E-2']
e2e2 = average_rows(e2e2_raw, ['system', 'variable'], VALUE_KEYS)

e2e2_by_system = defaultdict(lambda: {'patterns': [], 'with_exec_tps': [],
                                       'consensus_tps': []})
for r in e2e2:
    e2e2_by_system[r['system']]['patterns'].append(r['variable'])
    e2e2_by_system[r['system']]['with_exec_tps'].append(r['with_exec_tps'])
    e2e2_by_system[r['system']]['consensus_tps'].append(r['consensus_tps'])

fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(16, 5))

systems_e2e2 = [s for s in ['MP3+LEAP', 'Tusk+LeapBase'] if s in e2e2_by_system]
if systems_e2e2:
    pats = e2e2_by_system[systems_e2e2[0]]['patterns']
    n_pats = len(pats)
    n_sys = len(systems_e2e2)
    width = 0.35
    x = np.arange(n_pats)

    # Subplot 1: With-exec TPS by pattern
    for i, sys_name in enumerate(systems_e2e2):
        d = e2e2_by_system[sys_name]
        color = COLORS.get(sys_name, '#333')
        offset = (i - n_sys / 2 + 0.5) * width
        bars = ax1.bar(x + offset,
                       [t / 1000 for t in d['with_exec_tps'][:n_pats]],
                       width, color=color, label=sys_name, alpha=0.85)
        for bar, val in zip(bars, d['with_exec_tps'][:n_pats]):
            ax1.text(bar.get_x() + bar.get_width() / 2,
                     bar.get_height() + 0.5,
                     f'{val / 1000:.1f}K', ha='center', va='bottom',
                     fontsize=9)

    ax1.set_xlabel('Conflict Pattern')
    ax1.set_ylabel('With-Execution TPS (K TPS)')
    ax1.set_title('E2E-2: With-Exec TPS by Conflict Pattern')
    ax1.set_xticks(x)
    pat_labels = [p.replace('_', '\n') for p in pats]
    ax1.set_xticklabels(pat_labels, fontsize=10)
    ax1.legend(fontsize=11)

    # Subplot 2: Consensus TPS comparison (unaffected by pattern)
    for i, sys_name in enumerate(systems_e2e2):
        d = e2e2_by_system[sys_name]
        color = COLORS.get(sys_name, '#333')
        offset = (i - n_sys / 2 + 0.5) * width
        ax2.bar(x + offset,
                [t / 1000 for t in d['consensus_tps'][:n_pats]],
                width, color=color, label=sys_name, alpha=0.85)

    ax2.set_xlabel('Conflict Pattern')
    ax2.set_ylabel('Consensus TPS (K TPS)')
    ax2.set_title('E2E-2: Consensus TPS by Pattern')
    ax2.set_xticks(x)
    ax2.set_xticklabels(pat_labels, fontsize=10)
    ax2.legend(fontsize=11)

fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp3_e2e2_conflict_patterns.png'), dpi=150)
plt.close(fig)

# ============================================================
# E2E-3: Node Scalability (line plot)
# ============================================================
e2e3_raw = [r for r in rows if r['experiment'] == 'E2E-3']
e2e3 = average_rows(e2e3_raw, ['system', 'variable'], VALUE_KEYS)

e2e3_by_system = defaultdict(lambda: {'nodes': [], 'with_exec_tps': [],
                                       'with_exec_lat': []})
for r in e2e3:
    e2e3_by_system[r['system']]['nodes'].append(int(float(r['variable'])))
    e2e3_by_system[r['system']]['with_exec_tps'].append(r['with_exec_tps'])
    e2e3_by_system[r['system']]['with_exec_lat'].append(r['with_exec_latency_ms'])

fig, ax = plt.subplots()
for sys_name, d in e2e3_by_system.items():
    color = COLORS.get(sys_name, '#333')
    marker = MARKERS.get(sys_name, 'o')
    ax.plot(d['nodes'], [t / 1000 for t in d['with_exec_tps']], marker=marker,
            color=color, linewidth=2, markersize=8, label=sys_name)

ax.set_xlabel('Number of Nodes (n)')
ax.set_ylabel('With-Execution Throughput (K TPS)')
ax.set_title('E2E-3: Node Scalability')
if e2e3:
    ax.set_xticks(sorted({int(float(r['variable'])) for r in e2e3}))
ax.legend()
fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp3_e2e3_node_scalability.png'), dpi=150)
plt.close(fig)

print(f"Plots saved to {PLOTS_DIR}/")
print(f"Source data: {CSV_PATH}")
