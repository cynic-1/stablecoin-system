#!/usr/bin/env python3
"""
Correctness Visualization: verify all LEAP engine variants produce identical
balances to serial execution.

Reads:
  leap/correctness_results/correctness_detail.csv
  leap/correctness_results/correctness_summary.csv

Generates:
  correctness_balances_{scenario}.png  — Per-account balance overlay
  correctness_heatmap_{scenario}.png   — Difference heatmap vs Serial
"""
import csv
import os
import sys
from collections import defaultdict

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import matplotlib.colors as mcolors
import numpy as np

# Match existing plot.py style
plt.rcParams.update({
    'font.size': 14,
    'figure.figsize': (10, 6),
    'axes.grid': True,
    'grid.alpha': 0.3,
})

COLORS = {
    'Serial':          '#999999',
    'Serial+CADO':     '#666666',
    'LEAP-base':       '#4472C4',
    'LEAP-base+CADO':  '#264478',
    'LEAP':            '#ED7D31',
    'LEAP-noDomain':   '#70AD47',
    'LEAP-noHotDelta': '#FFC000',
    'LEAP-noBP':       '#5B9BD5',
}

MARKERS = {
    'Serial':          's',
    'Serial+CADO':     'x',
    'LEAP-base':       'o',
    'LEAP-base+CADO':  'D',
    'LEAP':            '^',
    'LEAP-noDomain':   'v',
    'LEAP-noHotDelta': '<',
    'LEAP-noBP':       'p',
}

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, '..', '..'))
DETAIL_CSV = os.path.join(REPO_ROOT, 'leap', 'correctness_results', 'correctness_detail.csv')
SUMMARY_CSV = os.path.join(REPO_ROOT, 'leap', 'correctness_results', 'correctness_summary.csv')
PLOTS_DIR = os.path.join(SCRIPT_DIR, 'results', 'plots')

os.makedirs(PLOTS_DIR, exist_ok=True)

# ---------------------------------------------------------------------------
# Read detail CSV
# ---------------------------------------------------------------------------
if not os.path.exists(DETAIL_CSV):
    print(f"ERROR: Detail CSV not found: {DETAIL_CSV}")
    print("Run 'cd leap && cargo run --release --bin correctness_check' first.")
    sys.exit(1)

# Structure: balances[scenario][engine][account] = balance
balances = defaultdict(lambda: defaultdict(dict))

with open(DETAIL_CSV) as f:
    reader = csv.DictReader(f)
    for row in reader:
        scenario = row['scenario']
        engine = row['engine']
        account = int(row['account'])
        balance = int(row['balance'])
        balances[scenario][engine][account] = balance

# Read summary CSV
summary = []
if os.path.exists(SUMMARY_CSV):
    with open(SUMMARY_CSV) as f:
        reader = csv.DictReader(f)
        for row in reader:
            summary.append(row)

scenarios = sorted(balances.keys())
print(f"Scenarios: {scenarios}")

# Canonical engine order
ENGINE_ORDER = ['Serial', 'Serial+CADO', 'LEAP-base', 'LEAP-base+CADO', 'LEAP',
                'LEAP-noDomain', 'LEAP-noHotDelta', 'LEAP-noBP']

for scenario in scenarios:
    engines_data = balances[scenario]
    engines = [e for e in ENGINE_ORDER if e in engines_data]
    if not engines:
        continue

    # Collect all accounts across all engines.
    all_accounts = sorted(set().union(*(engines_data[e].keys() for e in engines)))
    n_accounts = len(all_accounts)
    acct_to_idx = {a: i for i, a in enumerate(all_accounts)}

    print(f"\n--- {scenario}: {len(engines)} engines, {n_accounts} accounts ---")

    # -----------------------------------------------------------------------
    # Plot A: Per-Account Balance Comparison (overlay)
    # -----------------------------------------------------------------------
    fig, ax = plt.subplots(figsize=(12, 6))

    for engine in engines:
        accts = sorted(engines_data[engine].keys())
        xs = [acct_to_idx[a] for a in accts]
        ys = [engines_data[engine][a] for a in accts]

        # Use smaller markers, slight alpha to see overlaps.
        ax.plot(xs, ys,
                marker=MARKERS.get(engine, 'o'),
                color=COLORS.get(engine, '#333333'),
                linestyle='none',
                label=engine,
                markersize=5,
                alpha=0.7)

    ax.set_xlabel('Account Index')
    ax.set_ylabel('Final Balance')
    ax.set_title(f'Correctness: Per-Account Balance -- {scenario}')
    ax.legend(fontsize=10, loc='upper right', ncol=2)

    # Show a subset of x-ticks if many accounts.
    if n_accounts > 50:
        tick_step = max(1, n_accounts // 20)
        ax.set_xticks(range(0, n_accounts, tick_step))
        ax.set_xticklabels([str(all_accounts[i]) for i in range(0, n_accounts, tick_step)],
                           rotation=45, fontsize=8)
    fig.tight_layout()
    fname = os.path.join(PLOTS_DIR, f'correctness_balances_{scenario}.png')
    fig.savefig(fname, dpi=150)
    plt.close(fig)
    print(f"  Saved {fname}")

    # -----------------------------------------------------------------------
    # Plot B: Difference Heatmap vs Correct Serial Reference
    # -----------------------------------------------------------------------
    serial_bals = engines_data.get('Serial', {})
    serial_cado_bals = engines_data.get('Serial+CADO', {})
    # CADO engines compare against Serial+CADO; non-CADO against Serial.
    CADO_ENGINES = {'Serial+CADO', 'LEAP-base+CADO', 'LEAP', 'LEAP-noDomain',
                    'LEAP-noHotDelta', 'LEAP-noBP'}
    parallel_engines = [e for e in engines if e not in ('Serial', 'Serial+CADO')]
    if not parallel_engines:
        continue

    # Build difference matrix: rows = engines, cols = accounts.
    diff_matrix = np.zeros((len(parallel_engines), n_accounts))
    for i, engine in enumerate(parallel_engines):
        ref = serial_cado_bals if engine in CADO_ENGINES else serial_bals
        for j, acct in enumerate(all_accounts):
            ref_val = ref.get(acct, 0)
            engine_val = engines_data[engine].get(acct, 0)
            diff_matrix[i, j] = engine_val - ref_val

    # Determine color range.
    abs_max = max(np.abs(diff_matrix).max(), 1)  # At least 1 to avoid zero-range.

    fig, ax = plt.subplots(figsize=(14, max(3, len(parallel_engines) * 0.6 + 1.5)))
    cmap = plt.cm.RdBu_r
    norm = mcolors.TwoSlopeNorm(vmin=-abs_max, vcenter=0, vmax=abs_max)

    im = ax.imshow(diff_matrix, aspect='auto', cmap=cmap, norm=norm, interpolation='nearest')

    # Label each row with its reference.
    ylabels = []
    for e in parallel_engines:
        ref_label = "CADO" if e in CADO_ENGINES else "orig"
        ylabels.append(f"{e} (vs {ref_label})")
    ax.set_yticks(range(len(parallel_engines)))
    ax.set_yticklabels(ylabels, fontsize=10)
    ax.set_xlabel('Account Index')
    ax.set_title(f'Balance Diff vs Serial Reference -- {scenario}')

    # X-ticks.
    if n_accounts > 50:
        tick_step = max(1, n_accounts // 20)
        ax.set_xticks(range(0, n_accounts, tick_step))
        ax.set_xticklabels([str(all_accounts[i]) for i in range(0, n_accounts, tick_step)],
                           rotation=45, fontsize=8)

    cbar = fig.colorbar(im, ax=ax, shrink=0.8)
    cbar.set_label('Balance Difference (engine - serial)')

    # Annotate: if all zeros, add a text overlay.
    total_nonzero = np.count_nonzero(diff_matrix)
    if total_nonzero == 0:
        ax.text(n_accounts / 2, len(parallel_engines) / 2,
                'ALL MATCH (diff = 0)',
                ha='center', va='center',
                fontsize=20, fontweight='bold',
                color='green', alpha=0.6,
                bbox=dict(boxstyle='round,pad=0.5', facecolor='white', alpha=0.8))

    fig.tight_layout()
    fname = os.path.join(PLOTS_DIR, f'correctness_heatmap_{scenario}.png')
    fig.savefig(fname, dpi=150)
    plt.close(fig)
    print(f"  Saved {fname}")

# ---------------------------------------------------------------------------
# Print summary table
# ---------------------------------------------------------------------------
if summary:
    print("\n=== CORRECTNESS SUMMARY ===")
    print(f"{'Scenario':<16} {'Engine':<20} {'Match?':<8} {'Mismatches':<12} {'MaxDiff':<10}")
    print("-" * 66)
    for row in summary:
        print(f"{row['scenario']:<16} {row['engine']:<20} {row['match_serial']:<8} "
              f"{row['num_mismatches']:<12} {row['max_diff']:<10}")

    all_match = all(row['match_serial'] == 'true' for row in summary)
    print()
    if all_match:
        print("RESULT: ALL ENGINES MATCH SERIAL -- correctness verified!")
    else:
        mismatched = [f"{r['engine']}/{r['scenario']}" for r in summary if r['match_serial'] != 'true']
        print(f"RESULT: MISMATCHES DETECTED in: {', '.join(mismatched)}")

print(f"\nPlots saved to {PLOTS_DIR}/")
