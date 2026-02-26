#!/usr/bin/env python3
"""
Complete E2E Experiment Suite — Thesis Chapter 5

Four orthogonal experiment dimensions:
  A. Throughput-Latency Scaling (Uniform)  — 3 systems × 5 rates × 2 runs = 30
  B. Conflict Pattern Sensitivity (50K)    — 2 systems × 5 patterns × 2 runs = 20
  C. Node Scalability (50K, Uniform)       — 2 systems × 2 node counts × 2 runs = 8
  D. Contention × Rate Interaction         — 2 systems × 2 patterns × 4 rates × 2 runs = 32

Total: 90 runs (~2 hours)
Output: experiments/exp3_e2e/results/raw/exp3_e2e_complete.csv

Usage:
  python3 run_e2e_complete.py          # run all 4 experiments
  python3 run_e2e_complete.py A        # run only Exp A
  python3 run_e2e_complete.py A B      # run Exp A and B
"""

import os
import sys
import time
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from run_e2e import (
    SYSTEM_MP3_LEAP,
    SYSTEM_TUSK_LEAPBASE,
    SYSTEM_TUSK_SERIAL,
    NODE_PARAMS,
    TX_SIZE,
    DURATION,
    FIELDNAMES,
    leap_threads_for_nodes,
    run_single_benchmark,
    make_result_row,
    write_csv,
    print_summary,
)


RUNS = 2  # 2 runs per config, report average

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp3_e2e', 'results', 'raw',
)

# =============================================
# Experiment A: Throughput-Latency Scaling
# =============================================
# Purpose: Characterize how the system behaves as input load increases.
#   Shows consensus TPS ceiling, execution backpressure, latency growth.
#   Three systems establish the full hierarchy: Serial < LeapBase ≤ LEAP.
EXP_A_RATES = [10_000, 50_000, 100_000, 150_000, 200_000]
EXP_A_NODES = 4
EXP_A_WORKERS = 1
EXP_A_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE, SYSTEM_TUSK_SERIAL]

# =============================================
# Experiment B: Conflict Pattern Sensitivity
# =============================================
# Purpose: Show how different access patterns affect execution performance.
#   At 50K rate execution is not saturated, so TPS differences reflect
#   pure conflict overhead. LEAP's Hot-Delta + domain-aware scheduling
#   should stabilize latency across patterns.
EXP_B_PATTERNS = ['Uniform', 'Zipf_0.8', 'Zipf_1.2', 'Hotspot_50pct', 'Hotspot_90pct']
EXP_B_RATE = 50_000
EXP_B_NODES = 4
EXP_B_WORKERS = 1
EXP_B_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

# =============================================
# Experiment C: Node Scalability
# =============================================
# Purpose: Show behavior with growing committee size.
#   On localhost, CPU is shared across nodes: n * LEAP_THREADS ≤ 16.
#   n=4 → 4 threads/node (good).  n=10 → 1 thread/node (constrained).
#   We skip n=20 as it produces 40 processes on 16 cores (meaningless).
EXP_C_NODES_LIST = [4, 10]
EXP_C_RATE = 50_000
EXP_C_WORKERS = 1
EXP_C_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

# =============================================
# Experiment D: Contention × Rate Interaction
# =============================================
# Purpose: The key experiment — push execution to saturation under
#   high-conflict patterns. At 150K+ input rate, execution capacity
#   becomes the bottleneck.  LEAP's contention handling should create
#   measurable TPS divergence over LeapBase.
EXP_D_PATTERNS = ['Hotspot_50pct', 'Hotspot_90pct']
EXP_D_RATES = [50_000, 100_000, 150_000, 200_000]
EXP_D_NODES = 4
EXP_D_WORKERS = 1
EXP_D_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]


def run_exp(tag, systems, nodes_list, workers, rates, patterns, runs):
    """Generic experiment runner."""
    total = len(systems) * len(nodes_list) * len(rates) * len(patterns) * runs
    completed = 0
    results = []

    for sys_name, features, env_base in systems:
        for nodes in nodes_list:
            for pattern in patterns:
                for rate in rates:
                    env = dict(env_base)
                    if pattern != 'Uniform':
                        env['LEAP_PATTERN'] = pattern
                    for run_id in range(1, runs + 1):
                        completed += 1
                        print(f"\n[{tag}: {completed}/{total}] ", end='')

                        # Build a human-readable variable column.
                        if len(patterns) > 1 and len(rates) > 1:
                            variable = f'{pattern}@{rate // 1000}K'
                        elif len(patterns) > 1:
                            variable = pattern
                        elif len(nodes_list) > 1:
                            variable = nodes
                        else:
                            variable = rate

                        metrics = run_single_benchmark(
                            sys_name, nodes, workers, rate, run_id,
                            features, env,
                        )
                        if metrics['status'] == 'ok':
                            results.append(make_result_row(
                                tag, sys_name, variable,
                                nodes, workers, rate, run_id, metrics,
                            ))
                        time.sleep(3)

    return results


def run_exp_a():
    print(f"\n{'#' * 70}")
    print(f"  Experiment A: Throughput-Latency Scaling (Uniform)")
    print(f"  systems={[s[0] for s in EXP_A_SYSTEMS]}")
    print(f"  rates={EXP_A_RATES}, nodes={EXP_A_NODES}, runs={RUNS}")
    print(f"{'#' * 70}")
    return run_exp(
        'Exp-A', EXP_A_SYSTEMS, [EXP_A_NODES], EXP_A_WORKERS,
        EXP_A_RATES, ['Uniform'], RUNS,
    )


def run_exp_b():
    print(f"\n{'#' * 70}")
    print(f"  Experiment B: Conflict Pattern Sensitivity")
    print(f"  systems={[s[0] for s in EXP_B_SYSTEMS]}")
    print(f"  patterns={EXP_B_PATTERNS}, rate={EXP_B_RATE}, runs={RUNS}")
    print(f"{'#' * 70}")
    return run_exp(
        'Exp-B', EXP_B_SYSTEMS, [EXP_B_NODES], EXP_B_WORKERS,
        [EXP_B_RATE], EXP_B_PATTERNS, RUNS,
    )


def run_exp_c():
    print(f"\n{'#' * 70}")
    print(f"  Experiment C: Node Scalability")
    print(f"  systems={[s[0] for s in EXP_C_SYSTEMS]}")
    print(f"  nodes={EXP_C_NODES_LIST}, rate={EXP_C_RATE}, runs={RUNS}")
    print(f"{'#' * 70}")
    return run_exp(
        'Exp-C', EXP_C_SYSTEMS, EXP_C_NODES_LIST, EXP_C_WORKERS,
        [EXP_C_RATE], ['Uniform'], RUNS,
    )


def run_exp_d():
    print(f"\n{'#' * 70}")
    print(f"  Experiment D: Contention x Rate Interaction")
    print(f"  systems={[s[0] for s in EXP_D_SYSTEMS]}")
    print(f"  patterns={EXP_D_PATTERNS}, rates={EXP_D_RATES}, runs={RUNS}")
    print(f"{'#' * 70}")
    return run_exp(
        'Exp-D', EXP_D_SYSTEMS, [EXP_D_NODES], EXP_D_WORKERS,
        EXP_D_RATES, EXP_D_PATTERNS, RUNS,
    )


EXP_MAP = {
    'A': (run_exp_a, lambda: len(EXP_A_SYSTEMS) * len(EXP_A_RATES) * RUNS),
    'B': (run_exp_b, lambda: len(EXP_B_SYSTEMS) * len(EXP_B_PATTERNS) * RUNS),
    'C': (run_exp_c, lambda: len(EXP_C_SYSTEMS) * len(EXP_C_NODES_LIST) * RUNS),
    'D': (run_exp_d, lambda: len(EXP_D_SYSTEMS) * len(EXP_D_PATTERNS) * len(EXP_D_RATES) * RUNS),
}

SUMMARY_KEYS = {
    'A': ['system', 'rate'],
    'B': ['system', 'variable'],
    'C': ['system', 'nodes'],
    'D': ['system', 'variable'],
}


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)

    start_time = time.time()

    exps = sys.argv[1:] if len(sys.argv) > 1 else ['A', 'B', 'C', 'D']
    exps = [e.strip().upper() for e in exps]

    total_runs = sum(EXP_MAP[e][1]() for e in exps if e in EXP_MAP)

    print(f"Complete E2E Experiment Suite")
    print(f"============================")
    print(f"Experiments: {', '.join(f'Exp-{e}({EXP_MAP[e][1]()})' for e in exps)}")
    print(f"Total benchmark runs: {total_runs}")
    print(f"Duration per run: {DURATION}s + ~15s setup")
    print(f"Estimated time: ~{total_runs * (DURATION + 15) // 60} minutes")
    print()

    all_results = []
    for exp_id in exps:
        if exp_id in EXP_MAP:
            all_results.extend(EXP_MAP[exp_id][0]())

    elapsed = time.time() - start_time
    print(f"\n\nAll benchmarks complete in {elapsed / 60:.1f} minutes.")
    print(f"Successful runs: {len(all_results)}/{total_runs}")

    if all_results:
        csv_path = os.path.join(OUTPUT_DIR, 'exp3_e2e_complete.csv')
        write_csv(all_results, csv_path, FIELDNAMES)

        for exp_id in exps:
            tag = f'Exp-{exp_id}'
            exp_results = [r for r in all_results if r['experiment'] == tag]
            if exp_results:
                print(f"\n{'=' * 70}")
                print(f"  {tag} SUMMARY")
                print(f"{'=' * 70}")
                print_summary(exp_results, SUMMARY_KEYS.get(exp_id, ['system', 'variable']))
    else:
        print("No successful runs. Check errors above.")


if __name__ == '__main__':
    main()
