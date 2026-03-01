#!/usr/bin/env python3
"""
Experiment 3: Realistic End-to-End Pipeline Benchmark.

Composes real consensus data from Experiment 2 (Narwhal framework, TCP + Ed25519)
with measured execution data from the LEAP Rust binary.

Approach:
  - Phase 1 (Consensus): Real data from exp2_all_results.csv (68 runs)
  - Phase 2 (Execution): New Rust binary runs CADO + LEAP on realistically-sized blocks
  - Composition:
      block_size = consensus_tps * (consensus_latency_ms / 1000)
      e2e_tps = min(consensus_tps, execution_tps)
      e2e_latency_ms = consensus_latency_ms + execution_latency_ms
"""

import csv
import os
import subprocess
import sys
import statistics
from collections import defaultdict

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_DIR = os.path.join(SCRIPT_DIR, '..', '..')
EXP2_CSV = os.path.join(PROJECT_DIR, 'experiments', 'exp2_consensus', 'results', 'raw', 'exp2_all_results.csv')
RESULTS_DIR = os.path.join(SCRIPT_DIR, 'results', 'raw')
OUTPUT_CSV = os.path.join(RESULTS_DIR, 'exp3_e2e_realistic.csv')
RUST_BINARY = os.path.join(PROJECT_DIR, 'e2e', 'target', 'release', 'e2e_benchmark')

# Fixed execution parameters
EXEC_THREADS = 16
OVERHEAD_US = 100
ACCOUNTS = 1000
NUM_BLOCKS = 20
WARMUP_BLOCKS = 5  # handled inside the Rust binary


def load_exp2_data():
    """Load and average Experiment 2 consensus data (2 runs per config)."""
    if not os.path.exists(EXP2_CSV):
        print(f"ERROR: Exp 2 data not found: {EXP2_CSV}")
        print("Run experiments/exp2_consensus first.")
        sys.exit(1)

    rows = []
    with open(EXP2_CSV) as f:
        reader = csv.DictReader(f)
        for row in reader:
            rows.append(row)

    # Group by (experiment, protocol, k, nodes, workers, rate) and average over runs
    grouped = defaultdict(list)
    for r in rows:
        key = (r['experiment'], r['protocol'], r['k'], r['nodes'], r['workers'], r['rate'])
        grouped[key].append(r)

    averaged = {}
    for key, runs in grouped.items():
        avg_tps = statistics.mean(float(r['consensus_tps']) for r in runs)
        avg_lat = statistics.mean(float(r['consensus_latency_ms']) for r in runs)
        averaged[key] = {'tps': avg_tps, 'latency_ms': avg_lat}

    return averaged


def run_execution(block_size, pattern, engine, threads=EXEC_THREADS, overhead_us=OVERHEAD_US):
    """Run the Rust binary and return median execution metrics."""
    cmd = [
        RUST_BINARY,
        '--block-size', str(block_size),
        '--num-blocks', str(NUM_BLOCKS),
        '--pattern', pattern,
        '--accounts', str(ACCOUNTS),
        '--threads', str(threads),
        '--overhead-us', str(overhead_us),
        '--engine', engine,
    ]

    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"ERROR running: {' '.join(cmd)}")
        print(result.stderr)
        sys.exit(1)

    # Parse CSV from stdout
    lines = result.stdout.strip().split('\n')
    reader = csv.DictReader(lines)
    rows = list(reader)

    if not rows:
        return {'cado_us': 0, 'execution_us': 0, 'total_us': 0, 'tps': 0}

    # Take median of measured blocks
    tps_values = sorted(float(r['tps']) for r in rows)
    cado_values = sorted(int(r['cado_us']) for r in rows)
    exec_values = sorted(int(r['execution_us']) for r in rows)
    total_values = sorted(int(r['total_us']) for r in rows)

    mid = len(rows) // 2
    return {
        'cado_us': cado_values[mid],
        'execution_us': exec_values[mid],
        'total_us': total_values[mid],
        'tps': tps_values[mid],
    }


def compose_e2e(cons_tps, cons_lat_ms, exec_metrics, block_size):
    """Compose end-to-end metrics from consensus + execution."""
    exec_lat_ms = exec_metrics['total_us'] / 1000.0
    e2e_lat_ms = cons_lat_ms + exec_lat_ms
    e2e_tps = block_size / (e2e_lat_ms / 1000.0)
    return {
        'e2e_tps': e2e_tps,
        'e2e_latency_ms': e2e_lat_ms,
        'exec_tps': exec_metrics['tps'],
        'exec_latency_ms': exec_lat_ms,
        'cado_us': exec_metrics['cado_us'],
    }


def main():
    os.makedirs(RESULTS_DIR, exist_ok=True)

    print("=== Experiment 3: Realistic E2E Pipeline ===")
    print(f"Exp 2 data: {EXP2_CSV}")
    print(f"Rust binary: {RUST_BINARY}")
    print(f"Exec params: threads={EXEC_THREADS}, overhead={OVERHEAD_US}us, accounts={ACCOUNTS}")
    print()

    exp2 = load_exp2_data()

    csv_rows = []
    header = [
        'experiment', 'system', 'variable',
        'consensus_tps', 'consensus_latency_ms',
        'block_size', 'exec_engine',
        'cado_us', 'exec_tps', 'exec_latency_ms',
        'e2e_tps', 'e2e_latency_ms',
    ]

    # === E2E-1: Throughput vs Input Rate ===
    print("--- E2E-1: Throughput vs Input Rate ---")
    input_rates = [10000, 30000, 50000, 70000, 100000]

    for rate in input_rates:
        # System 1: MP3-BFT++ k=4 + LEAP (full)
        key_mp3 = ('A_rate', 'MP3-BFT++_k4', '4', '4', '1', str(rate))
        if key_mp3 in exp2:
            cons = exp2[key_mp3]
            block_size = int(cons['tps'] * (cons['latency_ms'] / 1000.0))
            block_size = max(block_size, 100)  # floor

            exec_m = run_execution(block_size, 'Uniform', 'Leap')
            e2e = compose_e2e(cons['tps'], cons['latency_ms'], exec_m, block_size)

            csv_rows.append({
                'experiment': 'E2E-1', 'system': 'MP3-BFT+++LEAP', 'variable': rate,
                'consensus_tps': f"{cons['tps']:.0f}",
                'consensus_latency_ms': f"{cons['latency_ms']:.0f}",
                'block_size': block_size,
                'exec_engine': 'Leap',
                'cado_us': e2e['cado_us'],
                'exec_tps': f"{e2e['exec_tps']:.0f}",
                'exec_latency_ms': f"{e2e['exec_latency_ms']:.1f}",
                'e2e_tps': f"{e2e['e2e_tps']:.0f}",
                'e2e_latency_ms': f"{e2e['e2e_latency_ms']:.1f}",
            })
            print(f"  rate={rate:>6} MP3+LEAP      cons={cons['tps']:.0f} exec={e2e['exec_tps']:.0f} e2e={e2e['e2e_tps']:.0f} TPS  lat={e2e['e2e_latency_ms']:.0f}ms")

        # System 2: Tusk + LEAP-base
        key_tusk = ('A_rate', 'Tusk', '0', '4', '1', str(rate))
        if key_tusk in exp2:
            cons = exp2[key_tusk]
            block_size = int(cons['tps'] * (cons['latency_ms'] / 1000.0))
            block_size = max(block_size, 100)

            exec_m = run_execution(block_size, 'Uniform', 'LeapBase')
            e2e = compose_e2e(cons['tps'], cons['latency_ms'], exec_m, block_size)

            csv_rows.append({
                'experiment': 'E2E-1', 'system': 'Tusk+LEAP-base', 'variable': rate,
                'consensus_tps': f"{cons['tps']:.0f}",
                'consensus_latency_ms': f"{cons['latency_ms']:.0f}",
                'block_size': block_size,
                'exec_engine': 'LeapBase',
                'cado_us': e2e['cado_us'],
                'exec_tps': f"{e2e['exec_tps']:.0f}",
                'exec_latency_ms': f"{e2e['exec_latency_ms']:.1f}",
                'e2e_tps': f"{e2e['e2e_tps']:.0f}",
                'e2e_latency_ms': f"{e2e['e2e_latency_ms']:.1f}",
            })
            print(f"  rate={rate:>6} Tusk+LEAP-base cons={cons['tps']:.0f} exec={e2e['exec_tps']:.0f} e2e={e2e['e2e_tps']:.0f} TPS  lat={e2e['e2e_latency_ms']:.0f}ms")

        # System 3: Tusk + Serial
        if key_tusk in exp2:
            cons = exp2[key_tusk]
            block_size = int(cons['tps'] * (cons['latency_ms'] / 1000.0))
            block_size = max(block_size, 100)

            exec_m = run_execution(block_size, 'Uniform', 'Serial')
            e2e = compose_e2e(cons['tps'], cons['latency_ms'], exec_m, block_size)

            csv_rows.append({
                'experiment': 'E2E-1', 'system': 'Tusk+Serial', 'variable': rate,
                'consensus_tps': f"{cons['tps']:.0f}",
                'consensus_latency_ms': f"{cons['latency_ms']:.0f}",
                'block_size': block_size,
                'exec_engine': 'Serial',
                'cado_us': e2e['cado_us'],
                'exec_tps': f"{e2e['exec_tps']:.0f}",
                'exec_latency_ms': f"{e2e['exec_latency_ms']:.1f}",
                'e2e_tps': f"{e2e['e2e_tps']:.0f}",
                'e2e_latency_ms': f"{e2e['e2e_latency_ms']:.1f}",
            })
            print(f"  rate={rate:>6} Tusk+Serial    cons={cons['tps']:.0f} exec={e2e['exec_tps']:.0f} e2e={e2e['e2e_tps']:.0f} TPS  lat={e2e['e2e_latency_ms']:.0f}ms")

    # === E2E-2: Conflict Pattern Impact ===
    print("\n--- E2E-2: Conflict Pattern Impact ---")
    patterns = [
        ('Uniform', 'Uniform'),
        ('Zipf_0.8', 'Zipf_0.8'),
        ('Zipf_1.2', 'Zipf_1.2'),
        ('Hotspot_50pct', 'Hotspot_50pct'),
    ]

    # Use rate=50K consensus data as the fixed point
    key_mp3_50k = ('A_rate', 'MP3-BFT++_k4', '4', '4', '1', '50000')
    key_tusk_50k = ('A_rate', 'Tusk', '0', '4', '1', '50000')

    for pat_name, pat_arg in patterns:
        if key_mp3_50k in exp2:
            cons = exp2[key_mp3_50k]
            block_size = int(cons['tps'] * (cons['latency_ms'] / 1000.0))
            block_size = max(block_size, 100)

            exec_m = run_execution(block_size, pat_arg, 'Leap')
            e2e = compose_e2e(cons['tps'], cons['latency_ms'], exec_m, block_size)

            csv_rows.append({
                'experiment': 'E2E-2', 'system': 'MP3-BFT+++LEAP', 'variable': pat_name,
                'consensus_tps': f"{cons['tps']:.0f}",
                'consensus_latency_ms': f"{cons['latency_ms']:.0f}",
                'block_size': block_size,
                'exec_engine': 'Leap',
                'cado_us': e2e['cado_us'],
                'exec_tps': f"{e2e['exec_tps']:.0f}",
                'exec_latency_ms': f"{e2e['exec_latency_ms']:.1f}",
                'e2e_tps': f"{e2e['e2e_tps']:.0f}",
                'e2e_latency_ms': f"{e2e['e2e_latency_ms']:.1f}",
            })
            print(f"  {pat_name:<16} MP3+LEAP       exec={e2e['exec_tps']:.0f} e2e={e2e['e2e_tps']:.0f} TPS")

        if key_tusk_50k in exp2:
            cons = exp2[key_tusk_50k]
            block_size = int(cons['tps'] * (cons['latency_ms'] / 1000.0))
            block_size = max(block_size, 100)

            exec_m = run_execution(block_size, pat_arg, 'LeapBase')
            e2e = compose_e2e(cons['tps'], cons['latency_ms'], exec_m, block_size)

            csv_rows.append({
                'experiment': 'E2E-2', 'system': 'Tusk+LEAP-base', 'variable': pat_name,
                'consensus_tps': f"{cons['tps']:.0f}",
                'consensus_latency_ms': f"{cons['latency_ms']:.0f}",
                'block_size': block_size,
                'exec_engine': 'LeapBase',
                'cado_us': e2e['cado_us'],
                'exec_tps': f"{e2e['exec_tps']:.0f}",
                'exec_latency_ms': f"{e2e['exec_latency_ms']:.1f}",
                'e2e_tps': f"{e2e['e2e_tps']:.0f}",
                'e2e_latency_ms': f"{e2e['e2e_latency_ms']:.1f}",
            })
            print(f"  {pat_name:<16} Tusk+LEAP-base exec={e2e['exec_tps']:.0f} e2e={e2e['e2e_tps']:.0f} TPS")

    # === E2E-3: Node Scalability ===
    print("\n--- E2E-3: Node Scalability ---")
    node_counts = [4, 10, 20]

    for n in node_counts:
        key_mp3_n = ('C_nodes', 'MP3-BFT++_k4', '4', str(n), '1', '50000')
        key_tusk_n = ('C_nodes', 'Tusk', '0', str(n), '1', '50000')

        if key_mp3_n in exp2:
            cons = exp2[key_mp3_n]
            block_size = int(cons['tps'] * (cons['latency_ms'] / 1000.0))
            block_size = max(block_size, 100)

            exec_m = run_execution(block_size, 'Uniform', 'Leap')
            e2e = compose_e2e(cons['tps'], cons['latency_ms'], exec_m, block_size)

            csv_rows.append({
                'experiment': 'E2E-3', 'system': 'MP3-BFT+++LEAP', 'variable': n,
                'consensus_tps': f"{cons['tps']:.0f}",
                'consensus_latency_ms': f"{cons['latency_ms']:.0f}",
                'block_size': block_size,
                'exec_engine': 'Leap',
                'cado_us': e2e['cado_us'],
                'exec_tps': f"{e2e['exec_tps']:.0f}",
                'exec_latency_ms': f"{e2e['exec_latency_ms']:.1f}",
                'e2e_tps': f"{e2e['e2e_tps']:.0f}",
                'e2e_latency_ms': f"{e2e['e2e_latency_ms']:.1f}",
            })
            print(f"  n={n:<3} MP3+LEAP       cons={cons['tps']:.0f} e2e={e2e['e2e_tps']:.0f} TPS  lat={e2e['e2e_latency_ms']:.0f}ms")

        if key_tusk_n in exp2:
            cons = exp2[key_tusk_n]
            block_size = int(cons['tps'] * (cons['latency_ms'] / 1000.0))
            block_size = max(block_size, 100)

            exec_m = run_execution(block_size, 'Uniform', 'LeapBase')
            e2e = compose_e2e(cons['tps'], cons['latency_ms'], exec_m, block_size)

            csv_rows.append({
                'experiment': 'E2E-3', 'system': 'Tusk+LEAP-base', 'variable': n,
                'consensus_tps': f"{cons['tps']:.0f}",
                'consensus_latency_ms': f"{cons['latency_ms']:.0f}",
                'block_size': block_size,
                'exec_engine': 'LeapBase',
                'cado_us': e2e['cado_us'],
                'exec_tps': f"{e2e['exec_tps']:.0f}",
                'exec_latency_ms': f"{e2e['exec_latency_ms']:.1f}",
                'e2e_tps': f"{e2e['e2e_tps']:.0f}",
                'e2e_latency_ms': f"{e2e['e2e_latency_ms']:.1f}",
            })
            print(f"  n={n:<3} Tusk+LEAP-base cons={cons['tps']:.0f} e2e={e2e['e2e_tps']:.0f} TPS  lat={e2e['e2e_latency_ms']:.0f}ms")

    # Write output CSV
    with open(OUTPUT_CSV, 'w', newline='') as f:
        writer = csv.DictWriter(f, fieldnames=header)
        writer.writeheader()
        writer.writerows(csv_rows)

    print(f"\nOutput CSV: {OUTPUT_CSV}")
    print(f"Total data points: {len(csv_rows)}")


if __name__ == '__main__':
    main()
