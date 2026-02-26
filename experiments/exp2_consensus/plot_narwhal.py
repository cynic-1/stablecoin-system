#!/usr/bin/env python3
"""
Plot Tusk vs MP3-BFT++ comparison results from Narwhal benchmark framework.

Reads exp2_all_results.csv and generates:
  Experiment A: Throughput vs Rate, Latency vs Throughput, Bar comparisons
  Experiment B: Throughput vs Workers scaling
  Experiment C: Throughput vs Committee size scaling
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
RAW_DIR = os.path.join(SCRIPT_DIR, 'results', 'raw')
PLOT_DIR = os.path.join(SCRIPT_DIR, 'results', 'plots')

COLORS = {
    'Tusk': '#2196F3',
    'MP3-BFT++_k1': '#4CAF50',
    'MP3-BFT++_k2': '#FF9800',
    'MP3-BFT++_k4': '#E91E63',
}
LABELS = {
    'Tusk': 'Tusk (baseline)',
    'MP3-BFT++_k1': 'MP3-BFT++ (k=1)',
    'MP3-BFT++_k2': 'MP3-BFT++ (k=2)',
    'MP3-BFT++_k4': 'MP3-BFT++ (k=4)',
}
MARKERS = {
    'Tusk': 'o',
    'MP3-BFT++_k1': 's',
    'MP3-BFT++_k2': '^',
    'MP3-BFT++_k4': 'D',
}


def load_data(csv_path):
    """Load and average results across runs, grouped by experiment."""
    raw = defaultdict(list)

    with open(csv_path) as f:
        reader = csv.DictReader(f)
        for row in reader:
            exp = row.get('experiment', 'A_rate')
            key = (exp, row['protocol'], int(row.get('nodes', 4)),
                   int(row.get('workers', 1)), int(row['rate']))
            raw[key].append({
                'consensus_tps': float(row['consensus_tps']),
                'consensus_latency_ms': float(row['consensus_latency_ms']),
                'e2e_tps': float(row['e2e_tps']),
                'e2e_latency_ms': float(row['e2e_latency_ms']),
            })

    averaged = {}
    for key, runs in raw.items():
        averaged[key] = {
            metric: np.mean([r[metric] for r in runs])
            for metric in runs[0].keys()
        }

    return averaged


def load_compat_data(csv_path):
    """Load the backward-compatible CSV (exp2_narwhal_comparison.csv)."""
    raw = defaultdict(list)

    with open(csv_path) as f:
        reader = csv.DictReader(f)
        for row in reader:
            key = (row['protocol'], int(row['rate']))
            raw[key].append({
                'consensus_tps': float(row['consensus_tps']),
                'consensus_latency_ms': float(row['consensus_latency_ms']),
                'e2e_tps': float(row['e2e_tps']),
                'e2e_latency_ms': float(row['e2e_latency_ms']),
            })

    averaged = {}
    for key, runs in raw.items():
        averaged[key] = {
            metric: np.mean([r[metric] for r in runs])
            for metric in runs[0].keys()
        }

    return averaged


def _proto_order(p):
    order = list(LABELS.keys())
    return order.index(p) if p in order else 99


# === Experiment A plots ===

def plot_throughput_vs_rate(data):
    """Plot 1: Consensus TPS vs Input Rate."""
    fig, ax = plt.subplots(figsize=(10, 6))

    protocols = sorted(set(k[1] for k in data.keys() if k[0] == 'A_rate'), key=_proto_order)

    for proto in protocols:
        rates = sorted(set(k[4] for k in data.keys() if k[0] == 'A_rate' and k[1] == proto))
        tps = [data[('A_rate', proto, 4, 1, r)]['consensus_tps'] / 1000 for r in rates]
        rates_k = [r / 1000 for r in rates]

        ax.plot(rates_k, tps,
                marker=MARKERS.get(proto, 'o'),
                color=COLORS.get(proto, 'gray'),
                label=LABELS.get(proto, proto),
                linewidth=2, markersize=8)

    max_rate = max(k[4] for k in data.keys() if k[0] == 'A_rate') / 1000
    ax.plot([0, max_rate], [0, max_rate], 'k--', alpha=0.3, label='Ideal (TPS = Input Rate)')

    ax.set_xlabel('Input Rate (K tx/s)', fontsize=12)
    ax.set_ylabel('Consensus Throughput (K tx/s)', fontsize=12)
    ax.set_title('Consensus Throughput vs Input Rate\n(4 nodes, 1 worker, localhost, Ed25519)', fontsize=13)
    ax.legend(fontsize=10)
    ax.grid(True, alpha=0.3)
    ax.set_xlim(left=0)
    ax.set_ylim(bottom=0)

    path = os.path.join(PLOT_DIR, 'narwhal_throughput_vs_rate.png')
    fig.savefig(path, dpi=150, bbox_inches='tight')
    plt.close(fig)
    print(f'Saved: {path}')


def plot_latency_vs_throughput(data):
    """Plot 2: Latency vs Throughput tradeoff."""
    fig, ax = plt.subplots(figsize=(10, 6))

    protocols = sorted(set(k[1] for k in data.keys() if k[0] == 'A_rate'), key=_proto_order)

    for proto in protocols:
        rates = sorted(set(k[4] for k in data.keys() if k[0] == 'A_rate' and k[1] == proto))
        tps = [data[('A_rate', proto, 4, 1, r)]['e2e_tps'] / 1000 for r in rates]
        lat = [data[('A_rate', proto, 4, 1, r)]['e2e_latency_ms'] for r in rates]

        ax.plot(tps, lat,
                marker=MARKERS.get(proto, 'o'),
                color=COLORS.get(proto, 'gray'),
                label=LABELS.get(proto, proto),
                linewidth=2, markersize=8)

    ax.set_xlabel('End-to-End Throughput (K tx/s)', fontsize=12)
    ax.set_ylabel('End-to-End Latency (ms)', fontsize=12)
    ax.set_title('Throughput-Latency Tradeoff\n(4 nodes, 1 worker, localhost, Ed25519)', fontsize=13)
    ax.legend(fontsize=10)
    ax.grid(True, alpha=0.3)
    ax.set_xlim(left=0)
    ax.set_ylim(bottom=0)

    path = os.path.join(PLOT_DIR, 'narwhal_latency_vs_throughput.png')
    fig.savefig(path, dpi=150, bbox_inches='tight')
    plt.close(fig)
    print(f'Saved: {path}')


def plot_bar_comparison(data):
    """Plot 3: Bar chart comparison at each input rate."""
    protocols = sorted(set(k[1] for k in data.keys() if k[0] == 'A_rate'), key=_proto_order)
    rates = sorted(set(k[4] for k in data.keys() if k[0] == 'A_rate'))

    fig, ax = plt.subplots(figsize=(12, 6))

    n_proto = len(protocols)
    bar_width = 0.8 / n_proto
    x = np.arange(len(rates))

    for i, proto in enumerate(protocols):
        tps = []
        for r in rates:
            key = ('A_rate', proto, 4, 1, r)
            tps.append(data[key]['consensus_tps'] / 1000 if key in data else 0)

        offset = (i - n_proto / 2 + 0.5) * bar_width
        bars = ax.bar(x + offset, tps, bar_width,
                      color=COLORS.get(proto, 'gray'),
                      label=LABELS.get(proto, proto),
                      edgecolor='white', linewidth=0.5)

        for bar, val in zip(bars, tps):
            if val > 0:
                ax.text(bar.get_x() + bar.get_width() / 2, bar.get_height() + 0.5,
                        f'{val:.1f}K', ha='center', va='bottom', fontsize=7, rotation=45)

    ax.set_xlabel('Input Rate (tx/s)', fontsize=12)
    ax.set_ylabel('Consensus Throughput (K tx/s)', fontsize=12)
    ax.set_title('Protocol Comparison: Consensus Throughput\n(4 nodes, 1 worker, localhost, Ed25519)', fontsize=13)
    ax.set_xticks(x)
    ax.set_xticklabels([f'{r//1000}K' for r in rates])
    ax.legend(fontsize=10)
    ax.grid(True, alpha=0.3, axis='y')
    ax.set_ylim(bottom=0)

    path = os.path.join(PLOT_DIR, 'narwhal_bar_comparison.png')
    fig.savefig(path, dpi=150, bbox_inches='tight')
    plt.close(fig)
    print(f'Saved: {path}')


def plot_latency_bar(data):
    """Plot 4: Latency bar chart at each input rate."""
    protocols = sorted(set(k[1] for k in data.keys() if k[0] == 'A_rate'), key=_proto_order)
    rates = sorted(set(k[4] for k in data.keys() if k[0] == 'A_rate'))

    fig, ax = plt.subplots(figsize=(12, 6))

    n_proto = len(protocols)
    bar_width = 0.8 / n_proto
    x = np.arange(len(rates))

    for i, proto in enumerate(protocols):
        lat = []
        for r in rates:
            key = ('A_rate', proto, 4, 1, r)
            lat.append(data[key]['consensus_latency_ms'] if key in data else 0)

        offset = (i - n_proto / 2 + 0.5) * bar_width
        ax.bar(x + offset, lat, bar_width,
               color=COLORS.get(proto, 'gray'),
               label=LABELS.get(proto, proto),
               edgecolor='white', linewidth=0.5)

    ax.set_xlabel('Input Rate (tx/s)', fontsize=12)
    ax.set_ylabel('Consensus Latency (ms)', fontsize=12)
    ax.set_title('Protocol Comparison: Consensus Latency\n(4 nodes, 1 worker, localhost, Ed25519)', fontsize=13)
    ax.set_xticks(x)
    ax.set_xticklabels([f'{r//1000}K' for r in rates])
    ax.legend(fontsize=10)
    ax.grid(True, alpha=0.3, axis='y')
    ax.set_ylim(bottom=0)

    path = os.path.join(PLOT_DIR, 'narwhal_latency_bar.png')
    fig.savefig(path, dpi=150, bbox_inches='tight')
    plt.close(fig)
    print(f'Saved: {path}')


# === Experiment B plots ===

def plot_workers_scaling(data):
    """Plot 5: Throughput scaling with workers."""
    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(14, 6))

    protocols = sorted(set(k[1] for k in data.keys() if k[0] == 'B_workers'), key=_proto_order)

    for proto in protocols:
        workers_list = sorted(set(k[3] for k in data.keys()
                                  if k[0] == 'B_workers' and k[1] == proto))
        tps = [data[('B_workers', proto, 4, w, 50000)]['consensus_tps'] / 1000
               for w in workers_list]
        lat = [data[('B_workers', proto, 4, w, 50000)]['consensus_latency_ms']
               for w in workers_list]

        ax1.plot(workers_list, tps,
                 marker=MARKERS.get(proto, 'o'),
                 color=COLORS.get(proto, 'gray'),
                 label=LABELS.get(proto, proto),
                 linewidth=2, markersize=8)

        ax2.plot(workers_list, lat,
                 marker=MARKERS.get(proto, 'o'),
                 color=COLORS.get(proto, 'gray'),
                 label=LABELS.get(proto, proto),
                 linewidth=2, markersize=8)

    ax1.set_xlabel('Workers per Node', fontsize=12)
    ax1.set_ylabel('Consensus Throughput (K tx/s)', fontsize=12)
    ax1.set_title('Throughput vs Workers\n(4 nodes, rate=50K)', fontsize=13)
    ax1.legend(fontsize=10)
    ax1.grid(True, alpha=0.3)
    ax1.set_xlim(left=0)
    ax1.set_ylim(bottom=0)

    ax2.set_xlabel('Workers per Node', fontsize=12)
    ax2.set_ylabel('Consensus Latency (ms)', fontsize=12)
    ax2.set_title('Latency vs Workers\n(4 nodes, rate=50K)', fontsize=13)
    ax2.legend(fontsize=10)
    ax2.grid(True, alpha=0.3)
    ax2.set_xlim(left=0)
    ax2.set_ylim(bottom=0)

    fig.tight_layout()
    path = os.path.join(PLOT_DIR, 'narwhal_workers_scaling.png')
    fig.savefig(path, dpi=150, bbox_inches='tight')
    plt.close(fig)
    print(f'Saved: {path}')


# === Experiment C plots ===

def plot_committee_scaling(data):
    """Plot 6: Throughput and latency vs committee size."""
    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(14, 6))

    protocols = sorted(set(k[1] for k in data.keys() if k[0] == 'C_nodes'), key=_proto_order)

    for proto in protocols:
        nodes_list = sorted(set(k[2] for k in data.keys()
                                if k[0] == 'C_nodes' and k[1] == proto))
        tps = [data[('C_nodes', proto, n, 1, 50000)]['consensus_tps'] / 1000
               for n in nodes_list]
        lat = [data[('C_nodes', proto, n, 1, 50000)]['consensus_latency_ms']
               for n in nodes_list]

        ax1.plot(nodes_list, tps,
                 marker=MARKERS.get(proto, 'o'),
                 color=COLORS.get(proto, 'gray'),
                 label=LABELS.get(proto, proto),
                 linewidth=2, markersize=8)

        ax2.plot(nodes_list, lat,
                 marker=MARKERS.get(proto, 'o'),
                 color=COLORS.get(proto, 'gray'),
                 label=LABELS.get(proto, proto),
                 linewidth=2, markersize=8)

    ax1.set_xlabel('Committee Size (n)', fontsize=12)
    ax1.set_ylabel('Consensus Throughput (K tx/s)', fontsize=12)
    ax1.set_title('Throughput vs Committee Size\n(1 worker, rate=50K)', fontsize=13)
    ax1.legend(fontsize=10)
    ax1.grid(True, alpha=0.3)
    ax1.set_ylim(bottom=0)

    ax2.set_xlabel('Committee Size (n)', fontsize=12)
    ax2.set_ylabel('Consensus Latency (ms)', fontsize=12)
    ax2.set_title('Latency vs Committee Size\n(1 worker, rate=50K)', fontsize=13)
    ax2.legend(fontsize=10)
    ax2.grid(True, alpha=0.3)
    ax2.set_ylim(bottom=0)

    fig.tight_layout()
    path = os.path.join(PLOT_DIR, 'narwhal_committee_scaling.png')
    fig.savefig(path, dpi=150, bbox_inches='tight')
    plt.close(fig)
    print(f'Saved: {path}')


def print_summary_table(data):
    """Print a formatted summary table for rate-scaling data."""
    protocols = sorted(set(k[1] for k in data.keys() if k[0] == 'A_rate'), key=_proto_order)
    rates = sorted(set(k[4] for k in data.keys() if k[0] == 'A_rate'))

    print(f"\n{'='*90}")
    print(f"  NARWHAL BENCHMARK RESULTS (4 nodes, 1 worker, localhost)")
    print(f"{'='*90}")
    print(f"{'Protocol':<18} {'Rate':>8} {'Con.TPS':>10} {'Con.Lat':>10} {'E2E TPS':>10} {'E2E Lat':>10}")
    print(f"{'-'*18} {'-'*8} {'-'*10} {'-'*10} {'-'*10} {'-'*10}")

    for proto in protocols:
        for rate in rates:
            key = ('A_rate', proto, 4, 1, rate)
            if key in data:
                d = data[key]
                print(f"{LABELS.get(proto, proto):<18} {rate:>8,} {d['consensus_tps']:>10,.0f} "
                      f"{d['consensus_latency_ms']:>10,.0f} {d['e2e_tps']:>10,.0f} "
                      f"{d['e2e_latency_ms']:>10,.0f}")
        print()


def main():
    os.makedirs(PLOT_DIR, exist_ok=True)

    # Try combined CSV first, fall back to compat CSV.
    combined_csv = os.path.join(RAW_DIR, 'exp2_all_results.csv')
    compat_csv = os.path.join(RAW_DIR, 'exp2_narwhal_comparison.csv')

    if os.path.exists(combined_csv):
        data = load_data(combined_csv)
        print(f"Loaded {len(data)} data points from {combined_csv}")

        experiments = set(k[0] for k in data.keys())

        if 'A_rate' in experiments:
            print_summary_table(data)
            plot_throughput_vs_rate(data)
            plot_latency_vs_throughput(data)
            plot_bar_comparison(data)
            plot_latency_bar(data)

        if 'B_workers' in experiments:
            plot_workers_scaling(data)

        if 'C_nodes' in experiments:
            plot_committee_scaling(data)

    elif os.path.exists(compat_csv):
        compat_data = load_compat_data(compat_csv)
        print(f"Loaded {len(compat_data)} data points from {compat_csv}")

        # Convert to new format for plotting.
        data = {}
        for (proto, rate), vals in compat_data.items():
            data[('A_rate', proto, 4, 1, rate)] = vals

        print_summary_table(data)
        plot_throughput_vs_rate(data)
        plot_latency_vs_throughput(data)
        plot_bar_comparison(data)
        plot_latency_bar(data)

    else:
        print(f"ERROR: No CSV files found.")
        print(f"  Tried: {combined_csv}")
        print(f"  Tried: {compat_csv}")
        print("Run narwhal/benchmark/run_comparison.py first.")
        sys.exit(1)

    print("\nAll plots generated successfully.")


if __name__ == '__main__':
    main()
