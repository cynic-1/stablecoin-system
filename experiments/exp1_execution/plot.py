#!/usr/bin/env python3
"""
Generate plots for Experiment 1: LEAP Execution Engine — Multi-Dimension Benchmark.

CSV format: engine,scenario,accounts,overhead_us,threads,run,tps

Generates:
  1_scalability_{scenario}_{overhead}us.png  — Thread scalability per overhead level
  2_overhead_speedup.png                      — Overhead-speedup tradeoff (money chart)
  3_contention_intensity.png                  — Contention intensity (vary accounts)
  4_ablation_{scenario}.png                   — Ablation study
  5_realistic_{scenario}.png                  — Realistic scaling (100us)
"""
import csv
import os
import sys
from collections import defaultdict

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import numpy as np

# Unified style
plt.rcParams.update({
    'font.size': 14,
    'figure.figsize': (8, 5),
    'axes.grid': True,
    'grid.alpha': 0.3,
})

COLORS = {
    'Serial':          '#999999',
    'LEAP-base':       '#4472C4',
    'LEAP':            '#ED7D31',
    'LEAP-full':       '#ED7D31',
    'LEAP-noDomain':   '#70AD47',
    'LEAP-noHotDelta': '#FFC000',
    'LEAP-noBP':       '#5B9BD5',
}

MARKERS = {
    'Serial':          's',
    'LEAP-base':       'o',
    'LEAP':            '^',
    'LEAP-full':       '^',
    'LEAP-noDomain':   'D',
    'LEAP-noHotDelta': 'v',
    'LEAP-noBP':       'p',
}

LINESTYLES = {
    'Serial':          '--',
    'LEAP-base':       '-',
    'LEAP':            '-',
    'LEAP-full':       '-',
    'LEAP-noDomain':   '--',
    'LEAP-noHotDelta': '--',
    'LEAP-noBP':       '--',
}

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
RESULTS_DIR = os.path.join(SCRIPT_DIR, 'results')
RAW_DIR = os.path.join(RESULTS_DIR, 'raw')
PLOTS_DIR = os.path.join(RESULTS_DIR, 'plots')
CSV_PATH = os.path.join(RAW_DIR, 'exp1_execution_v5.csv')

os.makedirs(PLOTS_DIR, exist_ok=True)

# ---------------------------------------------------------------------------
# Read CSV
# ---------------------------------------------------------------------------
if not os.path.exists(CSV_PATH):
    print(f"ERROR: CSV file not found: {CSV_PATH}")
    print("Run './run_all.sh' first to generate benchmark data.")
    sys.exit(1)

# Structure: data[engine][scenario][accounts][overhead_us][threads] = [tps values]
data = defaultdict(lambda: defaultdict(lambda: defaultdict(
    lambda: defaultdict(lambda: defaultdict(list)))))

with open(CSV_PATH) as f:
    reader = csv.DictReader(f)
    for row in reader:
        engine = row['engine']
        scenario = row['scenario']
        accounts = int(row['accounts'])
        overhead_us = int(row['overhead_us'])
        threads = int(row['threads'])
        tps = float(row['tps'])
        data[engine][scenario][accounts][overhead_us][threads].append(tps)


def median(values):
    s = sorted(values)
    n = len(s)
    return s[n // 2] if n > 0 else 0


def get_median(engine, scenario, accounts, overhead_us, threads):
    """Get median TPS for a specific configuration."""
    values = data[engine][scenario][accounts][overhead_us][threads]
    return median(values) if values else 0


# ---------------------------------------------------------------------------
# Plot 1: Thread Scalability per Overhead Level (Part 1 data)
# ---------------------------------------------------------------------------
print("Generating Plot 1: Thread scalability per overhead level...")

for overhead_us in [0, 1, 3, 10, 50, 100]:
    for scenario in ['Uniform', 'Hotspot_90pct']:
        fig, ax = plt.subplots()
        for engine in ['LEAP-base', 'LEAP']:
            threads_data = data[engine][scenario][1000][overhead_us]
            if not threads_data:
                continue
            threads_list = sorted(threads_data.keys())
            medians = [median(threads_data[t]) for t in threads_list]
            tps_k = [m / 1000 for m in medians]
            ax.plot(threads_list, tps_k,
                    marker=MARKERS.get(engine, 'o'),
                    color=COLORS.get(engine, '#333'),
                    linestyle=LINESTYLES.get(engine, '-'),
                    label=engine, linewidth=2, markersize=8)

        ax.set_xlabel('Number of Threads')
        ax.set_ylabel('TPS (x1000)')
        ax.set_title(f'Thread Scalability -- {scenario} ({overhead_us}us overhead)')
        ax.legend()
        ax.set_xticks([1, 2, 4, 8, 16])
        fig.tight_layout()
        fig.savefig(os.path.join(PLOTS_DIR,
                                 f'1_scalability_{scenario}_{overhead_us}us.png'), dpi=150)
        plt.close(fig)

# ---------------------------------------------------------------------------
# Plot 2: Overhead-Speedup Tradeoff (the "money chart")
# ---------------------------------------------------------------------------
print("Generating Plot 2: Overhead-speedup tradeoff...")

fig, ax = plt.subplots()
overhead_values = [0, 1, 3, 10, 50, 100]

for scenario, color, marker in [('Uniform', '#4472C4', 'o'),
                                  ('Hotspot_90pct', '#ED7D31', '^')]:
    speedups = []
    valid_overheads = []
    for oh in overhead_values:
        # Find max thread count available
        leap_base_data = data['LEAP-base'][scenario][1000][oh]
        leap_data = data['LEAP'][scenario][1000][oh]
        if not leap_base_data or not leap_data:
            continue
        max_t = max(set(leap_base_data.keys()) & set(leap_data.keys()))
        base_tps = median(leap_base_data[max_t])
        leap_tps = median(leap_data[max_t])
        if base_tps > 0:
            speedups.append(leap_tps / base_tps)
            valid_overheads.append(oh)

    if speedups:
        ax.plot(valid_overheads, speedups,
                marker=marker, color=color, linewidth=2, markersize=10,
                label=scenario)

ax.axhline(y=1.0, color='gray', linestyle=':', alpha=0.5)
ax.set_xlabel('Per-Transaction Overhead (us)')
ax.set_ylabel('LEAP Speedup over LEAP-base')
ax.set_title('LEAP Speedup vs Compute Overhead (max threads)')
ax.legend()
ax.set_xticks(overhead_values)
fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, '2_overhead_speedup.png'), dpi=150)
plt.close(fig)

# ---------------------------------------------------------------------------
# Plot 3: Contention Intensity (vary accounts)
# ---------------------------------------------------------------------------
print("Generating Plot 3: Contention intensity...")

fig, ax = plt.subplots()
account_values = [50, 200, 1000]
scenario = 'Hotspot_90pct'
overhead_us = 10

# Find max thread count
speedups = []
valid_accounts = []
for accts in account_values:
    leap_base_data = data['LEAP-base'][scenario][accts][overhead_us]
    leap_data = data['LEAP'][scenario][accts][overhead_us]
    if not leap_base_data or not leap_data:
        continue
    max_t = max(set(leap_base_data.keys()) & set(leap_data.keys()))
    base_tps = median(leap_base_data[max_t])
    leap_tps = median(leap_data[max_t])
    if base_tps > 0:
        speedups.append(leap_tps / base_tps)
        valid_accounts.append(accts)

if speedups:
    ax.bar(range(len(valid_accounts)), speedups, color='#ED7D31', width=0.5)
    ax.set_xticks(range(len(valid_accounts)))
    ax.set_xticklabels([str(a) for a in valid_accounts])
    ax.axhline(y=1.0, color='gray', linestyle=':', alpha=0.5)

ax.set_xlabel('Number of Accounts')
ax.set_ylabel('LEAP Speedup over LEAP-base')
ax.set_title(f'Contention Intensity ({scenario}, {overhead_us}us, max threads)')
fig.tight_layout()
fig.savefig(os.path.join(PLOTS_DIR, '3_contention_intensity.png'), dpi=150)
plt.close(fig)

# ---------------------------------------------------------------------------
# Plot 4: Ablation Study (5us overhead)
# ---------------------------------------------------------------------------
print("Generating Plot 4: Ablation study...")

ablation_engines = ['LEAP-base', 'LEAP', 'LEAP-noDomain', 'LEAP-noHotDelta', 'LEAP-noBP']
ablation_overhead = 10

for scenario in ['Zipf_0.8', 'Hotspot_90pct']:
    fig, ax = plt.subplots()
    for engine in ablation_engines:
        threads_data = data[engine][scenario][1000][ablation_overhead]
        if not threads_data:
            continue
        display = 'LEAP-full' if engine == 'LEAP' else engine
        threads_list = sorted(threads_data.keys())
        medians = [median(threads_data[t]) for t in threads_list]
        tps_k = [m / 1000 for m in medians]
        ax.plot(threads_list, tps_k,
                marker=MARKERS.get(display, 'o'),
                color=COLORS.get(display, '#333'),
                linestyle=LINESTYLES.get(display, '-'),
                label=display, linewidth=2, markersize=8)

    ax.set_xlabel('Number of Threads')
    ax.set_ylabel('TPS (x1000)')
    ax.set_title(f'Ablation Study -- {scenario} ({ablation_overhead}us overhead)')
    ax.legend(fontsize=10)
    ax.set_xticks([1, 4, 8, 16])
    fig.tight_layout()
    fname = scenario.replace(' ', '_')
    fig.savefig(os.path.join(PLOTS_DIR, f'4_ablation_{fname}.png'), dpi=150)
    plt.close(fig)

# ---------------------------------------------------------------------------
# Plot 5: Realistic Scaling (100us overhead, all scenarios)
# ---------------------------------------------------------------------------
print("Generating Plot 5: Realistic scaling...")

realistic_overhead = 100
realistic_engines = ['Serial', 'LEAP-base', 'LEAP']

for scenario_name in ['Uniform', 'Zipf_0.8', 'Zipf_1.2', 'Hotspot_50pct', 'Hotspot_90pct']:
    fig, ax = plt.subplots()
    for engine in realistic_engines:
        threads_data = data[engine][scenario_name][1000][realistic_overhead]
        if not threads_data:
            continue
        threads_list = sorted(threads_data.keys())
        medians = [median(threads_data[t]) for t in threads_list]
        tps_k = [m / 1000 for m in medians]
        ax.plot(threads_list, tps_k,
                marker=MARKERS.get(engine, 'o'),
                color=COLORS.get(engine, '#333'),
                linestyle=LINESTYLES.get(engine, '-'),
                label=engine, linewidth=2, markersize=8)

    ax.set_xlabel('Number of Threads')
    ax.set_ylabel('TPS (x1000)')
    ax.set_title(f'Realistic Scaling -- {scenario_name} (100us overhead)')
    ax.legend()
    ax.set_xticks([1, 2, 4, 8, 16])
    fig.tight_layout()
    fname = scenario_name.replace(' ', '_')
    fig.savefig(os.path.join(PLOTS_DIR, f'5_realistic_{fname}.png'), dpi=150)
    plt.close(fig)

print(f"Plots saved to {PLOTS_DIR}/")
print(f"Source data: {CSV_PATH}")
