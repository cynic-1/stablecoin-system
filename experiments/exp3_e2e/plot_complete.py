#!/usr/bin/env python3
"""
Generate plots for the Complete E2E Experiment Suite.
Reads exp3_e2e_complete.csv (90 runs) from run_e2e_complete.py.

Produces 6 figures:
  1. Exp-A: Throughput vs Input Rate (line)
  2. Exp-A: Latency Breakdown (stacked bar)
  3. Exp-B: Conflict Pattern Sensitivity (grouped bar)
  4. Exp-C: Node Scalability (grouped bar)
  5. Exp-D: Contention × Rate Interaction — TPS heatmap lines
  6. Exp-D: LEAP Advantage (%) vs Rate
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
CSV_PATH = os.path.join(RAW_DIR, 'exp3_e2e_complete.csv')

plt.rcParams.update({
    'font.size': 13,
    'figure.figsize': (10, 6),
    'axes.grid': True,
    'grid.alpha': 0.3,
    'legend.fontsize': 11,
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
    print(f"ERROR: {CSV_PATH} not found.")
    print("Run: cd narwhal/benchmark && python3 run_e2e_complete.py")
    sys.exit(1)

# --- Load data ---
rows = []
with open(CSV_PATH) as f:
    for row in csv.DictReader(f):
        rows.append(row)

VALUE_KEYS = [
    'consensus_tps', 'consensus_bps', 'consensus_latency_ms',
    'e2e_tps', 'e2e_bps', 'e2e_latency_ms',
    'with_exec_tps', 'with_exec_bps', 'with_exec_latency_ms',
    'duration_s',
]


def avg_rows(row_list, group_keys):
    """Group by group_keys, average all VALUE_KEYS."""
    grouped = defaultdict(list)
    for r in row_list:
        key = tuple(r[k] for k in group_keys)
        grouped[key].append(r)
    result = []
    for key, runs in sorted(grouped.items()):
        avg = {k: v for k, v in zip(group_keys, key)}
        for vk in VALUE_KEYS:
            vals = [float(r[vk]) for r in runs if r.get(vk)]
            avg[vk] = sum(vals) / len(vals) if vals else 0
        avg['n_runs'] = len(runs)
        result.append(avg)
    return result


def get_exp(tag):
    return [r for r in rows if r['experiment'] == tag]


# ============================================================
# Figure 1: Exp-A — Throughput vs Input Rate
# ============================================================
exp_a = avg_rows(get_exp('Exp-A'), ['system', 'rate'])

fig, ax = plt.subplots(figsize=(10, 6))
systems_order = ['MP3+LEAP', 'Tusk+LeapBase', 'Tusk+Serial']
for sys_name in systems_order:
    pts = [r for r in exp_a if r['system'] == sys_name]
    if not pts:
        continue
    rates = [int(float(r['rate'])) / 1000 for r in pts]
    tps = [r['with_exec_tps'] / 1000 for r in pts]
    ls = '--' if 'Serial' in sys_name else '-'
    ax.plot(rates, tps, marker=MARKERS.get(sys_name, 'o'),
            linestyle=ls, color=COLORS.get(sys_name, '#333'),
            label=sys_name, linewidth=2.5, markersize=9)

ax.set_xlabel('Input Rate (K tx/s)')
ax.set_ylabel('With-Execution Throughput (K TPS)')
ax.set_title('Exp A: End-to-End Throughput vs Input Rate (Uniform)')
ax.legend()
fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp_a_throughput.png'), dpi=150)
plt.close(fig)
print("  [1/6] exp_a_throughput.png")

# ============================================================
# Figure 2: Exp-A — Latency Breakdown (Consensus + Execution overhead)
# ============================================================
fig, ax = plt.subplots(figsize=(12, 6))
for sys_name in systems_order:
    pts = [r for r in exp_a if r['system'] == sys_name]
    if not pts:
        continue
    rates_k = [int(float(r['rate'])) // 1000 for r in pts]
    cons_lat = [r['consensus_latency_ms'] for r in pts]
    exec_overhead = [max(0, r['with_exec_latency_ms'] - r['consensus_latency_ms']) for r in pts]
    total_lat = [r['with_exec_latency_ms'] for r in pts]
    color = COLORS.get(sys_name, '#333')
    ax.plot(rates_k, total_lat, marker=MARKERS.get(sys_name, 'o'),
            linestyle='-', color=color, label=f'{sys_name} (total)',
            linewidth=2.5, markersize=9)
    ax.plot(rates_k, cons_lat, marker=MARKERS.get(sys_name, 'o'),
            linestyle=':', color=color, label=f'{sys_name} (consensus)',
            linewidth=1.5, markersize=5, alpha=0.6)

ax.set_xlabel('Input Rate (K tx/s)')
ax.set_ylabel('Latency (ms)')
ax.set_title('Exp A: Latency Breakdown — Total vs Consensus-Only')
ax.legend(fontsize=9, ncol=2)
fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp_a_latency.png'), dpi=150)
plt.close(fig)
print("  [2/6] exp_a_latency.png")

# ============================================================
# Figure 3: Exp-B — Conflict Pattern Sensitivity (grouped bar)
# ============================================================
exp_b = avg_rows(get_exp('Exp-B'), ['system', 'variable'])

fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(16, 6))
systems_b = ['MP3+LEAP', 'Tusk+LeapBase']
patterns = []
for r in exp_b:
    if r['variable'] not in patterns:
        patterns.append(r['variable'])

n_pats = len(patterns)
width = 0.35
x = np.arange(n_pats)

for i, sys_name in enumerate(systems_b):
    pts = [r for r in exp_b if r['system'] == sys_name]
    pts_map = {r['variable']: r for r in pts}
    tps_vals = [pts_map[p]['with_exec_tps'] / 1000 for p in patterns]
    lat_vals = [pts_map[p]['with_exec_latency_ms'] for p in patterns]
    color = COLORS.get(sys_name, '#333')
    offset = (i - 0.5) * width

    bars = ax1.bar(x + offset, tps_vals, width, color=color, label=sys_name, alpha=0.85)
    for bar, val in zip(bars, tps_vals):
        ax1.text(bar.get_x() + bar.get_width() / 2, bar.get_height() + 0.3,
                 f'{val:.1f}K', ha='center', va='bottom', fontsize=8)

    ax2.bar(x + offset, lat_vals, width, color=color, label=sys_name, alpha=0.85)

pat_labels = [p.replace('_', '\n') for p in patterns]
ax1.set_xlabel('Conflict Pattern')
ax1.set_ylabel('With-Execution TPS (K TPS)')
ax1.set_title('Exp B: Throughput by Conflict Pattern (50K rate)')
ax1.set_xticks(x)
ax1.set_xticklabels(pat_labels, fontsize=10)
ax1.legend()

ax2.set_xlabel('Conflict Pattern')
ax2.set_ylabel('With-Execution Latency (ms)')
ax2.set_title('Exp B: Latency by Conflict Pattern (50K rate)')
ax2.set_xticks(x)
ax2.set_xticklabels(pat_labels, fontsize=10)
ax2.legend()

fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp_b_patterns.png'), dpi=150)
plt.close(fig)
print("  [3/6] exp_b_patterns.png")

# ============================================================
# Figure 4: Exp-C — Node Scalability (grouped bar)
# ============================================================
exp_c = avg_rows(get_exp('Exp-C'), ['system', 'nodes'])

fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(12, 5))
systems_c = ['MP3+LEAP', 'Tusk+LeapBase']
node_vals = sorted({int(float(r['nodes'])) for r in exp_c})
n_nodes = len(node_vals)
width = 0.35
x = np.arange(n_nodes)

for i, sys_name in enumerate(systems_c):
    pts = [r for r in exp_c if r['system'] == sys_name]
    pts_map = {int(float(r['nodes'])): r for r in pts}
    tps = [pts_map.get(n, {}).get('with_exec_tps', 0) / 1000 for n in node_vals]
    lat = [pts_map.get(n, {}).get('with_exec_latency_ms', 0) for n in node_vals]
    color = COLORS.get(sys_name, '#333')
    offset = (i - 0.5) * width

    ax1.bar(x + offset, tps, width, color=color, label=sys_name, alpha=0.85)
    ax2.bar(x + offset, lat, width, color=color, label=sys_name, alpha=0.85)

for ax, ylabel, title in [
    (ax1, 'With-Exec TPS (K TPS)', 'Exp C: Throughput vs Committee Size'),
    (ax2, 'With-Exec Latency (ms)', 'Exp C: Latency vs Committee Size'),
]:
    ax.set_xlabel('Number of Nodes')
    ax.set_ylabel(ylabel)
    ax.set_title(title)
    ax.set_xticks(x)
    ax.set_xticklabels([str(n) for n in node_vals])
    ax.legend()

fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp_c_scalability.png'), dpi=150)
plt.close(fig)
print("  [4/6] exp_c_scalability.png")

# ============================================================
# Figure 5: Exp-D — Contention × Rate (line charts per pattern)
# ============================================================
exp_d = avg_rows(get_exp('Exp-D'), ['system', 'variable'])

# Parse variable column: "Hotspot_50pct@100K"
exp_d_parsed = []
for r in exp_d:
    parts = r['variable'].split('@')
    if len(parts) == 2:
        r['pattern'] = parts[0]
        r['rate_k'] = int(parts[1].replace('K', ''))
        exp_d_parsed.append(r)

d_patterns = sorted({r['pattern'] for r in exp_d_parsed})

fig, axes = plt.subplots(1, len(d_patterns), figsize=(8 * len(d_patterns), 6))
if len(d_patterns) == 1:
    axes = [axes]

for ax, pattern in zip(axes, d_patterns):
    for sys_name in ['MP3+LEAP', 'Tusk+LeapBase']:
        pts = sorted(
            [r for r in exp_d_parsed if r['system'] == sys_name and r['pattern'] == pattern],
            key=lambda r: r['rate_k'],
        )
        if not pts:
            continue
        rates = [r['rate_k'] for r in pts]
        tps = [r['with_exec_tps'] / 1000 for r in pts]
        color = COLORS.get(sys_name, '#333')
        marker = MARKERS.get(sys_name, 'o')
        ax.plot(rates, tps, marker=marker, color=color, linewidth=2.5,
                markersize=9, label=sys_name)

    ax.set_xlabel('Input Rate (K tx/s)')
    ax.set_ylabel('With-Execution TPS (K TPS)')
    ax.set_title(f'Exp D: {pattern.replace("_", " ")}')
    ax.legend()

fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp_d_contention_rate.png'), dpi=150)
plt.close(fig)
print("  [5/6] exp_d_contention_rate.png")

# ============================================================
# Figure 6: Exp-D — LEAP Advantage (%) vs Rate
# ============================================================
fig, ax = plt.subplots(figsize=(10, 6))
line_styles = {'Hotspot_50pct': '-', 'Hotspot_90pct': '--'}
line_colors = {'Hotspot_50pct': '#2CA02C', 'Hotspot_90pct': '#D62728'}

for pattern in d_patterns:
    mp3_pts = sorted(
        [r for r in exp_d_parsed if r['system'] == 'MP3+LEAP' and r['pattern'] == pattern],
        key=lambda r: r['rate_k'],
    )
    tusk_pts = sorted(
        [r for r in exp_d_parsed if r['system'] == 'Tusk+LeapBase' and r['pattern'] == pattern],
        key=lambda r: r['rate_k'],
    )
    tusk_map = {r['rate_k']: r['with_exec_tps'] for r in tusk_pts}

    rates, advantages = [], []
    for r in mp3_pts:
        base = tusk_map.get(r['rate_k'])
        if base and base > 0:
            adv = (r['with_exec_tps'] - base) / base * 100
            rates.append(r['rate_k'])
            advantages.append(adv)

    label = pattern.replace('_', ' ')
    ax.plot(rates, advantages, marker='o', linewidth=2.5, markersize=9,
            linestyle=line_styles.get(pattern, '-'),
            color=line_colors.get(pattern, '#333'),
            label=label)

ax.axhline(y=0, color='gray', linestyle='-', linewidth=0.8)
ax.set_xlabel('Input Rate (K tx/s)')
ax.set_ylabel('MP3+LEAP Advantage over Tusk+LeapBase (%)')
ax.set_title('Exp D: Execution Advantage under Contention')
ax.legend()
fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, 'exp_d_advantage.png'), dpi=150)
plt.close(fig)
print("  [6/6] exp_d_advantage.png")

print(f"\nAll plots saved to {PLOTS_DIR}/")
print(f"Source data: {CSV_PATH} ({len(rows)} rows)")
