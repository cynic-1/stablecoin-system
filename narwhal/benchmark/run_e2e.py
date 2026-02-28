#!/usr/bin/env python3
"""
End-to-End Benchmark: Narwhal Consensus + LEAP Execution

Runs three experiment dimensions with real TCP, real Ed25519 crypto,
real multi-process consensus, and real LEAP parallel execution.

E2E-1: Input rate scaling (4 nodes, variable rates, 3 systems)
E2E-2: Conflict pattern impact (4 nodes, 50K rate, variable patterns)
E2E-3: Node scalability (variable nodes, 50K rate)

Results saved to experiments/exp3_e2e/results/raw/exp3_e2e_realistic.csv
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

# Detect physical CPU cores (not hyperthreaded logical cores) for thread allocation.
# On HT machines os.cpu_count() returns 2x physical cores, causing oversubscription.
def _physical_cores():
    try:
        import subprocess
        # thread_siblings_list shows siblings sharing a physical core, e.g. "0,64" or "0-3"
        with open('/sys/devices/system/cpu/cpu0/topology/thread_siblings_list') as f:
            siblings = f.read().strip()
        logical = os.cpu_count() or 16
        # If siblings list has a comma or hyphen, HT is enabled → halve logical count
        if ',' in siblings or '-' in siblings:
            return logical // 2
        return logical
    except Exception:
        return os.cpu_count() or 16

TOTAL_CORES = _physical_cores()

NODE_PARAMS = {
    'header_size': 1_000,
    'max_header_delay': 200,
    'gc_depth': 50,
    'sync_retry_delay': 10_000,
    'sync_retry_nodes': 3,
    'batch_size': 500_000,
    'max_batch_delay': 200,
}


def leap_threads_for_nodes(nodes):
    """Compute per-node LEAP thread count to avoid CPU oversubscription.

    On localhost all nodes share the same CPU.  Each node's primary runs
    LEAP execution with LEAP_THREADS rayon workers.  To prevent
    oversubscription:  nodes * LEAP_THREADS <= TOTAL_CORES.
    """
    return max(1, TOTAL_CORES // nodes)


# Systems: (name, extra_features, env_vars_base)
# LEAP_THREADS is set dynamically per run — not hardcoded here.
SYSTEM_MP3_LEAP = (
    'MP3+LEAP',
    'e2e_exec,mp3bft',
    {
        'MP3BFT_K_SLOTS': '4',
        'LEAP_ENGINE': 'leap',
        'LEAP_CRYPTO_US': '10',
        'LEAP_ACCOUNTS': '1000',
    },
)

SYSTEM_TUSK_LEAPBASE = (
    'Tusk+LeapBase',
    'e2e_exec',
    {
        'LEAP_ENGINE': 'leap_base',
        'LEAP_CRYPTO_US': '10',
        'LEAP_ACCOUNTS': '1000',
    },
)

SYSTEM_TUSK_SERIAL = (
    'Tusk+Serial',
    'e2e_exec',
    {
        'LEAP_ENGINE': 'serial',
        'LEAP_THREADS': '1',
        'LEAP_CRYPTO_US': '10',
        'LEAP_ACCOUNTS': '1000',
    },
)

# E2E-1: Rate scaling
EXP1_RATES = [10_000, 30_000, 50_000, 70_000, 100_000]
EXP1_NODES = 4
EXP1_WORKERS = 1
EXP1_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE, SYSTEM_TUSK_SERIAL]

# E2E-2: Conflict patterns
EXP2_PATTERNS = ['Uniform', 'Zipf_0.8', 'Zipf_1.2', 'Hotspot_50pct', 'Hotspot_90pct']
EXP2_NODES = 4
EXP2_WORKERS = 1
EXP2_RATE = 50_000
EXP2_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

# E2E-3: Node scalability
EXP3_NODES_LIST = [4, 10, 20]
EXP3_WORKERS = 1
EXP3_RATE = 50_000
EXP3_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp3_e2e', 'results', 'raw',
)

FIELDNAMES = [
    'experiment', 'system', 'variable', 'nodes', 'workers', 'rate', 'run',
    'stablecoin_tps', 'stablecoin_latency_ms', 'success_rate',
    'total_txns', 'successful_txns',
    'consensus_tps', 'consensus_bps', 'consensus_latency_ms',
    'e2e_tps', 'e2e_bps', 'e2e_latency_ms',
    'with_exec_tps', 'with_exec_bps', 'with_exec_latency_ms',
    'duration_s',
]


def parse_summary(summary_text):
    """Extract metrics from a SUMMARY text block."""
    def extract(pattern, text):
        m = re.search(pattern, text)
        return float(m.group(1).replace(',', '')) if m else 0.0

    return {
        'stablecoin_tps': extract(r'Stablecoin TPS:\s+([\d,]+)', summary_text),
        'stablecoin_latency_ms': extract(r'Stablecoin latency:\s+([\d,]+)', summary_text),
        'success_rate': extract(r'Success rate:\s+([\d.]+)', summary_text),
        'total_txns': extract(r'Total transactions:\s+([\d,]+)', summary_text),
        'successful_txns': extract(r'Successful transactions:\s+([\d,]+)', summary_text),
        'consensus_tps': extract(r'Consensus TPS:\s+([\d,]+)', summary_text),
        'consensus_bps': extract(r'Consensus BPS:\s+([\d,]+)', summary_text),
        'consensus_latency_ms': extract(r'Consensus latency:\s+([\d,]+)', summary_text),
        'e2e_tps': extract(r'End-to-end TPS:\s+([\d,]+)', summary_text),
        'e2e_bps': extract(r'End-to-end BPS:\s+([\d,]+)', summary_text),
        'e2e_latency_ms': extract(r'End-to-end latency:\s+([\d,]+)', summary_text),
        'with_exec_tps': extract(r'With-execution TPS:\s+([\d,]+)', summary_text),
        'with_exec_bps': extract(r'With-execution BPS:\s+([\d,]+)', summary_text),
        'with_exec_latency_ms': extract(r'With-execution latency:\s+([\d,]+)', summary_text),
        'duration_s': extract(r'Execution time:\s+([\d,]+)', summary_text),
    }


def run_single_benchmark(system_name, nodes, workers, rate, run_id,
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

    # Add tx_size to env vars for LEAP to compute txns per certificate.
    env = dict(env_vars)
    env['BENCH_TX_SIZE'] = str(TX_SIZE)

    # Prevent CPU oversubscription on localhost: each node process runs
    # LEAP_THREADS rayon workers, so nodes * threads must fit in TOTAL_CORES.
    # Skip if caller already set LEAP_THREADS (e.g. thread-scaling experiment).
    if env.get('LEAP_ENGINE') not in ('serial',) and 'LEAP_THREADS' not in env_vars:
        threads = leap_threads_for_nodes(nodes)
        env['LEAP_THREADS'] = str(threads)
        env['RAYON_NUM_THREADS'] = str(threads)

    print(f"\n{'='*70}")
    print(f"  {system_name} | n={nodes} w={workers} rate={rate:,} | run={run_id}")
    print(f"  features={extra_features}")
    print(f"  env={env}")
    print(f"{'='*70}")

    try:
        bench = LocalBench(bench_params, NODE_PARAMS,
                           extra_features=extra_features,
                           env_vars=env)
        result = bench.run(debug=True)
        summary = result.result()
        print(summary)
        metrics = parse_summary(summary)
        metrics['status'] = 'ok'
        return metrics
    except BenchError as e:
        print(f"  ERROR: {e}")
        return {'status': 'error'}


def make_result_row(experiment, system_name, variable, nodes, workers, rate,
                     run_id, metrics):
    """Build a result row dict."""
    return {
        'experiment': experiment,
        'system': system_name,
        'variable': variable,
        'nodes': nodes,
        'workers': workers,
        'rate': rate,
        'run': run_id,
        'stablecoin_tps': metrics.get('stablecoin_tps', 0),
        'stablecoin_latency_ms': metrics.get('stablecoin_latency_ms', 0),
        'success_rate': metrics.get('success_rate', 0),
        'total_txns': metrics.get('total_txns', 0),
        'successful_txns': metrics.get('successful_txns', 0),
        'consensus_tps': metrics.get('consensus_tps', 0),
        'consensus_bps': metrics.get('consensus_bps', 0),
        'consensus_latency_ms': metrics.get('consensus_latency_ms', 0),
        'e2e_tps': metrics.get('e2e_tps', 0),
        'e2e_bps': metrics.get('e2e_bps', 0),
        'e2e_latency_ms': metrics.get('e2e_latency_ms', 0),
        'with_exec_tps': metrics.get('with_exec_tps', 0),
        'with_exec_bps': metrics.get('with_exec_bps', 0),
        'with_exec_latency_ms': metrics.get('with_exec_latency_ms', 0),
        'duration_s': metrics.get('duration_s', 0),
    }


def write_csv(results, csv_path, fieldnames):
    """Write results to CSV."""
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
    print(f"{''.join(header_parts)} {'SC.TPS':>10} {'SC.Lat':>10} "
          f"{'Success%':>10} {'Con.TPS':>10} {'Con.Lat':>10}")
    print('-' * (16 * len(group_keys) + 54))

    for key, runs in sorted(grouped.items()):
        avg_sctps = sum(float(r.get('stablecoin_tps', 0)) for r in runs) / len(runs)
        avg_sclat = sum(float(r.get('stablecoin_latency_ms', 0)) for r in runs) / len(runs)
        avg_sr = sum(float(r.get('success_rate', 0)) for r in runs) / len(runs)
        avg_ctps = sum(float(r['consensus_tps']) for r in runs) / len(runs)
        avg_clat = sum(float(r['consensus_latency_ms']) for r in runs) / len(runs)
        parts = [f'{str(v):<16}' for v in key]
        print(f"{''.join(parts)} {avg_sctps:>10,.0f} {avg_sclat:>10,.0f} "
              f"{avg_sr:>9.1%} {avg_ctps:>10,.0f} {avg_clat:>10,.0f}")


def run_e2e_1():
    """E2E-1: Rate scaling — 4 nodes, 1 worker, variable rates, 3 systems."""
    print(f"\n{'#'*70}")
    print(f"  E2E-1: Rate Scaling")
    print(f"  nodes={EXP1_NODES}, workers={EXP1_WORKERS}, rates={EXP1_RATES}")
    print(f"{'#'*70}")

    total = len(EXP1_SYSTEMS) * len(EXP1_RATES) * RUNS
    completed = 0
    results = []

    for sys_name, features, env_base in EXP1_SYSTEMS:
        for rate in EXP1_RATES:
            for run_id in range(1, RUNS + 1):
                completed += 1
                print(f"\n[E2E-1: {completed}/{total}] ", end='')
                metrics = run_single_benchmark(
                    sys_name, EXP1_NODES, EXP1_WORKERS, rate, run_id,
                    features, env_base,
                )
                if metrics['status'] == 'ok':
                    results.append(make_result_row(
                        'E2E-1', sys_name, rate,
                        EXP1_NODES, EXP1_WORKERS, rate, run_id, metrics,
                    ))
                time.sleep(3)

    return results


def run_e2e_2():
    """E2E-2: Conflict patterns — 4 nodes, 50K rate, variable patterns."""
    print(f"\n{'#'*70}")
    print(f"  E2E-2: Conflict Patterns")
    print(f"  nodes={EXP2_NODES}, rate={EXP2_RATE}, patterns={EXP2_PATTERNS}")
    print(f"{'#'*70}")

    total = len(EXP2_SYSTEMS) * len(EXP2_PATTERNS) * RUNS
    completed = 0
    results = []

    for sys_name, features, env_base in EXP2_SYSTEMS:
        for pattern in EXP2_PATTERNS:
            env = dict(env_base)
            env['LEAP_PATTERN'] = pattern
            for run_id in range(1, RUNS + 1):
                completed += 1
                print(f"\n[E2E-2: {completed}/{total}] ", end='')
                metrics = run_single_benchmark(
                    sys_name, EXP2_NODES, EXP2_WORKERS, EXP2_RATE, run_id,
                    features, env,
                )
                if metrics['status'] == 'ok':
                    results.append(make_result_row(
                        'E2E-2', sys_name, pattern,
                        EXP2_NODES, EXP2_WORKERS, EXP2_RATE, run_id, metrics,
                    ))
                time.sleep(3)

    return results


def run_e2e_3():
    """E2E-3: Node scalability — variable nodes, 1 worker, 50K rate."""
    print(f"\n{'#'*70}")
    print(f"  E2E-3: Node Scalability")
    print(f"  nodes={EXP3_NODES_LIST}, workers={EXP3_WORKERS}, rate={EXP3_RATE}")
    print(f"{'#'*70}")

    total = len(EXP3_SYSTEMS) * len(EXP3_NODES_LIST) * RUNS
    completed = 0
    results = []

    for sys_name, features, env_base in EXP3_SYSTEMS:
        for nodes in EXP3_NODES_LIST:
            for run_id in range(1, RUNS + 1):
                completed += 1
                print(f"\n[E2E-3: {completed}/{total}] ", end='')
                metrics = run_single_benchmark(
                    sys_name, nodes, EXP3_WORKERS, EXP3_RATE, run_id,
                    features, env_base,
                )
                if metrics['status'] == 'ok':
                    results.append(make_result_row(
                        'E2E-3', sys_name, nodes,
                        nodes, EXP3_WORKERS, EXP3_RATE, run_id, metrics,
                    ))
                time.sleep(3)

    return results


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)

    start_time = time.time()

    # Determine which experiments to run.
    exps = sys.argv[1:] if len(sys.argv) > 1 else ['1', '2', '3']
    exps = [e.strip() for e in exps]

    total_1 = len(EXP1_SYSTEMS) * len(EXP1_RATES) * RUNS if '1' in exps else 0
    total_2 = len(EXP2_SYSTEMS) * len(EXP2_PATTERNS) * RUNS if '2' in exps else 0
    total_3 = len(EXP3_SYSTEMS) * len(EXP3_NODES_LIST) * RUNS if '3' in exps else 0
    total_runs = total_1 + total_2 + total_3

    print(f"E2E Benchmark: Consensus + LEAP Execution")
    print(f"==========================================")
    print(f"Experiments: E2E-{', E2E-'.join(exps)}")
    print(f"Total benchmark runs: {total_runs}")
    print(f"Estimated time: ~{total_runs * (DURATION + 15) // 60} minutes")
    print()

    all_results = []

    if '1' in exps:
        all_results.extend(run_e2e_1())

    if '2' in exps:
        all_results.extend(run_e2e_2())

    if '3' in exps:
        all_results.extend(run_e2e_3())

    elapsed = time.time() - start_time
    print(f"\n\nAll benchmarks complete in {elapsed/60:.1f} minutes.")
    print(f"Successful runs: {len(all_results)}/{total_runs}")

    # Write CSV.
    if all_results:
        csv_path = os.path.join(OUTPUT_DIR, 'exp3_e2e_realistic.csv')
        write_csv(all_results, csv_path, FIELDNAMES)

        # Print summaries.
        for exp_id in exps:
            tag = f'E2E-{exp_id}'
            exp_results = [r for r in all_results if r['experiment'] == tag]
            if exp_results:
                print(f"\n{'='*70}")
                print(f"  {tag} SUMMARY")
                print(f"{'='*70}")
                if exp_id == '1':
                    print_summary(exp_results, ['system', 'rate'])
                elif exp_id == '2':
                    print_summary(exp_results, ['system', 'variable'])
                elif exp_id == '3':
                    print_summary(exp_results, ['system', 'nodes'])
    else:
        print("No successful runs. Check errors above.")


if __name__ == '__main__':
    main()
