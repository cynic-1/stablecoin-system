#!/usr/bin/env python3
"""
Tusk vs MP3-BFT++ Comprehensive Benchmark Comparison

Runs three experiment dimensions through the Narwhal benchmark framework
with real network, real Ed25519 crypto, and real distributed processes.

Experiment A: Rate scaling (4 nodes, 1 worker, variable rates)
Experiment B: Workers scaling (4 nodes, variable workers, fixed rate)
Experiment C: Committee scaling (variable nodes, 1 worker, fixed rate)

Results are saved to CSV for plotting.
"""

import csv
import os
import re
import sys
import time
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from benchmark.local import LocalBench
from benchmark.utils import BenchError, Print


# --- Configuration ---

TX_SIZE = 512
DURATION = 60  # seconds per run (increased from 30 for better steady-state)
RUNS = 3       # runs per configuration (increased from 2 for statistical confidence)

NODE_PARAMS = {
    'header_size': 1_000,
    'max_header_delay': 200,
    'gc_depth': 50,
    'sync_retry_delay': 10_000,
    'sync_retry_nodes': 3,
    'batch_size': 500_000,
    'max_batch_delay': 200,
}

# Protocols: (name, extra_features, env_vars)
PROTOCOLS_ALL = [
    ('Tusk',          None,     {}),
    ('MP3-BFT++_k1', 'mp3bft', {'MP3BFT_K_SLOTS': '1'}),
    ('MP3-BFT++_k2', 'mp3bft', {'MP3BFT_K_SLOTS': '2'}),
    ('MP3-BFT++_k4', 'mp3bft', {'MP3BFT_K_SLOTS': '4'}),
]

# For workers/committee scaling, use Tusk vs best MP3-BFT++ only.
PROTOCOLS_SCALING = [
    ('Tusk',          None,     {}),
    ('MP3-BFT++_k4', 'mp3bft', {'MP3BFT_K_SLOTS': '4'}),
]

# Experiment A: Rate scaling (same as before, but with fixed k=1).
EXP_A_RATES = [10_000, 30_000, 50_000, 70_000, 100_000]
EXP_A_NODES = 4
EXP_A_WORKERS = 1

# Experiment B: Workers scaling.
EXP_B_WORKERS_LIST = [1, 4, 7, 10]
EXP_B_NODES = 4
EXP_B_RATE = 50_000

# Experiment C: Committee scaling.
EXP_C_NODES_LIST = [4, 10, 20]
EXP_C_WORKERS = 1
EXP_C_RATE = 50_000

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp2_consensus', 'results', 'raw',
)


def parse_summary(summary_text):
    """Extract metrics from a SUMMARY text block."""
    def extract(pattern, text):
        m = re.search(pattern, text)
        return float(m.group(1).replace(',', '')) if m else 0.0

    return {
        'consensus_tps': extract(r'Consensus TPS:\s+([\d,]+)', summary_text),
        'consensus_bps': extract(r'Consensus BPS:\s+([\d,]+)', summary_text),
        'consensus_latency_ms': extract(r'Consensus latency:\s+([\d,]+)', summary_text),
        'e2e_tps': extract(r'End-to-end TPS:\s+([\d,]+)', summary_text),
        'e2e_bps': extract(r'End-to-end BPS:\s+([\d,]+)', summary_text),
        'e2e_latency_ms': extract(r'End-to-end latency:\s+([\d,]+)', summary_text),
        'duration_s': extract(r'Execution time:\s+([\d,]+)', summary_text),
    }


def run_single_benchmark(protocol_name, nodes, workers, rate, run_id,
                          extra_features, env_vars):
    """Run a single benchmark and return parsed metrics."""
    bench_params = {
        'faults': 0,
        'nodes': nodes,
        'workers': workers,
        'rate': rate,
        'tx_size': TX_SIZE,
        'duration': DURATION,
    }

    print(f"\n{'='*70}")
    print(f"  {protocol_name} | n={nodes} w={workers} rate={rate:,} | run={run_id}")
    print(f"{'='*70}")

    try:
        bench = LocalBench(bench_params, NODE_PARAMS,
                           extra_features=extra_features,
                           env_vars=env_vars)
        result = bench.run(debug=True)
        summary = result.result()
        print(summary)
        metrics = parse_summary(summary)
        metrics['status'] = 'ok'
        return metrics
    except BenchError as e:
        print(f"  ERROR: {e}")
        return {'status': 'error'}


def write_csv(results, csv_path, fieldnames):
    """Write results to CSV, filtering to only specified fields."""
    with open(csv_path, 'w', newline='') as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames, extrasaction='ignore')
        writer.writeheader()
        writer.writerows(results)
    print(f"Results saved to: {csv_path}")


def print_summary(results, group_keys):
    """Print averaged summary table."""
    grouped = defaultdict(list)
    for r in results:
        key = tuple(r[k] for k in group_keys)
        grouped[key].append(r)

    header_parts = [f'{k:<16}' for k in group_keys]
    print(f"{''.join(header_parts)} {'Con.TPS':>10} {'Con.Lat':>10} {'E2E TPS':>10} {'E2E Lat':>10}")
    print('-' * (16 * len(group_keys) + 44))

    for key, runs in sorted(grouped.items()):
        avg_ctps = sum(r['consensus_tps'] for r in runs) / len(runs)
        avg_clat = sum(r['consensus_latency_ms'] for r in runs) / len(runs)
        avg_etps = sum(r['e2e_tps'] for r in runs) / len(runs)
        avg_elat = sum(r['e2e_latency_ms'] for r in runs) / len(runs)
        parts = [f'{str(v):<16}' for v in key]
        print(f"{''.join(parts)} {avg_ctps:>10,.0f} {avg_clat:>10,.0f} {avg_etps:>10,.0f} {avg_elat:>10,.0f}")


FIELDNAMES = [
    'experiment', 'protocol', 'k', 'nodes', 'workers', 'rate', 'run',
    'consensus_tps', 'consensus_bps', 'consensus_latency_ms',
    'e2e_tps', 'e2e_bps', 'e2e_latency_ms', 'duration_s',
]


def make_result_row(experiment, protocol_name, env_vars, nodes, workers, rate, run_id, metrics):
    """Build a result row dict."""
    k_val = env_vars.get('MP3BFT_K_SLOTS', '0')
    return {
        'experiment': experiment,
        'protocol': protocol_name,
        'k': int(k_val),
        'nodes': nodes,
        'workers': workers,
        'rate': rate,
        'run': run_id,
        'consensus_tps': metrics['consensus_tps'],
        'consensus_bps': metrics['consensus_bps'],
        'consensus_latency_ms': metrics['consensus_latency_ms'],
        'e2e_tps': metrics['e2e_tps'],
        'e2e_bps': metrics['e2e_bps'],
        'e2e_latency_ms': metrics['e2e_latency_ms'],
        'duration_s': metrics['duration_s'],
    }


def run_experiment_a():
    """Exp A: Rate scaling — 4 nodes, 1 worker, variable rates, all protocols."""
    print(f"\n{'#'*70}")
    print(f"  EXPERIMENT A: Rate Scaling")
    print(f"  nodes={EXP_A_NODES}, workers={EXP_A_WORKERS}, rates={EXP_A_RATES}")
    print(f"{'#'*70}")

    total = len(PROTOCOLS_ALL) * len(EXP_A_RATES) * RUNS
    completed = 0
    results = []

    for proto_name, extra_feat, env in PROTOCOLS_ALL:
        for rate in EXP_A_RATES:
            for run_id in range(1, RUNS + 1):
                completed += 1
                print(f"\n[A: {completed}/{total}] ", end='')
                metrics = run_single_benchmark(
                    proto_name, EXP_A_NODES, EXP_A_WORKERS, rate, run_id,
                    extra_feat, env,
                )
                if metrics['status'] == 'ok':
                    results.append(make_result_row(
                        'A_rate', proto_name, env,
                        EXP_A_NODES, EXP_A_WORKERS, rate, run_id, metrics,
                    ))
                time.sleep(3)

    # Also write the exp2_narwhal_comparison.csv for backward compatibility.
    compat_csv = os.path.join(OUTPUT_DIR, 'exp2_narwhal_comparison.csv')
    compat_fields = [
        'protocol', 'k', 'rate', 'run',
        'consensus_tps', 'consensus_bps', 'consensus_latency_ms',
        'e2e_tps', 'e2e_bps', 'e2e_latency_ms', 'duration_s',
    ]
    write_csv(results, compat_csv, compat_fields)

    return results


def run_experiment_b():
    """Exp B: Workers scaling — 4 nodes, variable workers, fixed rate."""
    print(f"\n{'#'*70}")
    print(f"  EXPERIMENT B: Workers Scaling")
    print(f"  nodes={EXP_B_NODES}, workers={EXP_B_WORKERS_LIST}, rate={EXP_B_RATE}")
    print(f"{'#'*70}")

    total = len(PROTOCOLS_SCALING) * len(EXP_B_WORKERS_LIST) * RUNS
    completed = 0
    results = []

    for proto_name, extra_feat, env in PROTOCOLS_SCALING:
        for workers in EXP_B_WORKERS_LIST:
            for run_id in range(1, RUNS + 1):
                completed += 1
                print(f"\n[B: {completed}/{total}] ", end='')
                metrics = run_single_benchmark(
                    proto_name, EXP_B_NODES, workers, EXP_B_RATE, run_id,
                    extra_feat, env,
                )
                if metrics['status'] == 'ok':
                    results.append(make_result_row(
                        'B_workers', proto_name, env,
                        EXP_B_NODES, workers, EXP_B_RATE, run_id, metrics,
                    ))
                time.sleep(3)

    return results


def run_experiment_c():
    """Exp C: Committee scaling — variable nodes, 1 worker, fixed rate."""
    print(f"\n{'#'*70}")
    print(f"  EXPERIMENT C: Committee Scaling")
    print(f"  nodes={EXP_C_NODES_LIST}, workers={EXP_C_WORKERS}, rate={EXP_C_RATE}")
    print(f"{'#'*70}")

    total = len(PROTOCOLS_SCALING) * len(EXP_C_NODES_LIST) * RUNS
    completed = 0
    results = []

    for proto_name, extra_feat, env in PROTOCOLS_SCALING:
        for nodes in EXP_C_NODES_LIST:
            for run_id in range(1, RUNS + 1):
                completed += 1
                print(f"\n[C: {completed}/{total}] ", end='')
                metrics = run_single_benchmark(
                    proto_name, nodes, EXP_C_WORKERS, EXP_C_RATE, run_id,
                    extra_feat, env,
                )
                if metrics['status'] == 'ok':
                    results.append(make_result_row(
                        'C_nodes', proto_name, env,
                        nodes, EXP_C_WORKERS, EXP_C_RATE, run_id, metrics,
                    ))
                time.sleep(3)

    return results


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)

    start_time = time.time()

    # Determine which experiments to run.
    exps = sys.argv[1:] if len(sys.argv) > 1 else ['A', 'B', 'C']
    exps = [e.upper() for e in exps]

    total_a = len(PROTOCOLS_ALL) * len(EXP_A_RATES) * RUNS if 'A' in exps else 0
    total_b = len(PROTOCOLS_SCALING) * len(EXP_B_WORKERS_LIST) * RUNS if 'B' in exps else 0
    total_c = len(PROTOCOLS_SCALING) * len(EXP_C_NODES_LIST) * RUNS if 'C' in exps else 0
    total_runs = total_a + total_b + total_c

    print(f"Tusk vs MP3-BFT++ Comprehensive Benchmark")
    print(f"==========================================")
    print(f"Experiments: {exps}")
    print(f"Total benchmark runs: {total_runs}")
    print(f"Estimated time: ~{total_runs * (DURATION + 15) // 60} minutes")
    print()

    all_results = []

    if 'A' in exps:
        all_results.extend(run_experiment_a())

    if 'B' in exps:
        all_results.extend(run_experiment_b())

    if 'C' in exps:
        all_results.extend(run_experiment_c())

    elapsed = time.time() - start_time
    print(f"\n\nAll benchmarks complete in {elapsed/60:.1f} minutes.")
    print(f"Successful runs: {len(all_results)}/{total_runs}")

    # Write combined CSV.
    if all_results:
        combined_csv = os.path.join(OUTPUT_DIR, 'exp2_all_results.csv')
        write_csv(all_results, combined_csv, FIELDNAMES)

        # Print summaries.
        for exp_name in exps:
            exp_tag = {'A': 'A_rate', 'B': 'B_workers', 'C': 'C_nodes'}[exp_name]
            exp_results = [r for r in all_results if r['experiment'] == exp_tag]
            if exp_results:
                print(f"\n{'='*70}")
                print(f"  EXPERIMENT {exp_name} SUMMARY")
                print(f"{'='*70}")
                if exp_name == 'A':
                    print_summary(exp_results, ['protocol', 'rate'])
                elif exp_name == 'B':
                    print_summary(exp_results, ['protocol', 'workers'])
                elif exp_name == 'C':
                    print_summary(exp_results, ['protocol', 'nodes'])
    else:
        print("No successful runs. Check errors above.")


if __name__ == '__main__':
    main()
