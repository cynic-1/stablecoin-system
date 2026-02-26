#!/usr/bin/env python3
"""
E2E High-Load Supplementary Benchmarks

E2E-4: High input rates (150K, 200K, 250K) with Uniform pattern
E2E-5: High-conflict rate scaling (Hotspot 50%, 90%) at 50K–200K

Reuses infrastructure from run_e2e.py. Output goes to a separate CSV
to keep v6 data intact.
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from run_e2e import (
    SYSTEM_MP3_LEAP,
    SYSTEM_TUSK_LEAPBASE,
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


# --- E2E-4: High Input Rates (Uniform) ---
EXP4_RATES = [150_000, 200_000, 250_000]
EXP4_NODES = 4
EXP4_WORKERS = 1
EXP4_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

# --- E2E-5: High-Conflict Rate Scaling ---
EXP5_RATES = [50_000, 100_000, 150_000, 200_000]
EXP5_PATTERNS = ['Hotspot_50pct', 'Hotspot_90pct']
EXP5_NODES = 4
EXP5_WORKERS = 1
EXP5_SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

RUNS = 1  # Single run per config for speed

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp3_e2e', 'results', 'raw',
)


def run_e2e_4():
    """E2E-4: High input rates — 4 nodes, Uniform, 150K/200K/250K."""
    print(f"\n{'#'*70}")
    print(f"  E2E-4: High Input Rates (Uniform)")
    print(f"  nodes={EXP4_NODES}, workers={EXP4_WORKERS}, rates={EXP4_RATES}")
    print(f"{'#'*70}")

    total = len(EXP4_SYSTEMS) * len(EXP4_RATES) * RUNS
    completed = 0
    results = []

    for sys_name, features, env_base in EXP4_SYSTEMS:
        for rate in EXP4_RATES:
            for run_id in range(1, RUNS + 1):
                completed += 1
                print(f"\n[E2E-4: {completed}/{total}] ", end='')
                metrics = run_single_benchmark(
                    sys_name, EXP4_NODES, EXP4_WORKERS, rate, run_id,
                    features, env_base,
                )
                if metrics['status'] == 'ok':
                    results.append(make_result_row(
                        'E2E-4', sys_name, rate,
                        EXP4_NODES, EXP4_WORKERS, rate, run_id, metrics,
                    ))
                time.sleep(3)

    return results


def run_e2e_5():
    """E2E-5: High-conflict rate scaling — Hotspot patterns at variable rates."""
    print(f"\n{'#'*70}")
    print(f"  E2E-5: High-Conflict Rate Scaling")
    print(f"  nodes={EXP5_NODES}, patterns={EXP5_PATTERNS}, rates={EXP5_RATES}")
    print(f"{'#'*70}")

    total = len(EXP5_SYSTEMS) * len(EXP5_PATTERNS) * len(EXP5_RATES) * RUNS
    completed = 0
    results = []

    for sys_name, features, env_base in EXP5_SYSTEMS:
        for pattern in EXP5_PATTERNS:
            for rate in EXP5_RATES:
                env = dict(env_base)
                env['LEAP_PATTERN'] = pattern
                for run_id in range(1, RUNS + 1):
                    completed += 1
                    print(f"\n[E2E-5: {completed}/{total}] ", end='')
                    metrics = run_single_benchmark(
                        sys_name, EXP5_NODES, EXP5_WORKERS, rate, run_id,
                        features, env,
                    )
                    if metrics['status'] == 'ok':
                        results.append(make_result_row(
                            'E2E-5', sys_name,
                            f'{pattern}@{rate//1000}K',
                            EXP5_NODES, EXP5_WORKERS, rate, run_id, metrics,
                        ))
                    time.sleep(3)

    return results


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)

    start_time = time.time()

    exps = sys.argv[1:] if len(sys.argv) > 1 else ['4', '5']
    exps = [e.strip() for e in exps]

    total_4 = len(EXP4_SYSTEMS) * len(EXP4_RATES) * RUNS if '4' in exps else 0
    total_5 = (len(EXP5_SYSTEMS) * len(EXP5_PATTERNS)
               * len(EXP5_RATES) * RUNS) if '5' in exps else 0
    total_runs = total_4 + total_5

    print(f"E2E High-Load Supplementary Benchmarks")
    print(f"=======================================")
    print(f"Experiments: E2E-{', E2E-'.join(exps)}")
    print(f"Total benchmark runs: {total_runs}")
    print(f"Estimated time: ~{total_runs * (DURATION + 15) // 60} minutes")
    print()

    all_results = []

    if '4' in exps:
        all_results.extend(run_e2e_4())

    if '5' in exps:
        all_results.extend(run_e2e_5())

    elapsed = time.time() - start_time
    print(f"\n\nAll benchmarks complete in {elapsed/60:.1f} minutes.")
    print(f"Successful runs: {len(all_results)}/{total_runs}")

    if all_results:
        csv_path = os.path.join(OUTPUT_DIR, 'exp3_e2e_highload.csv')
        write_csv(all_results, csv_path, FIELDNAMES)

        for exp_id in exps:
            tag = f'E2E-{exp_id}'
            exp_results = [r for r in all_results if r['experiment'] == tag]
            if exp_results:
                print(f"\n{'='*70}")
                print(f"  {tag} SUMMARY")
                print(f"{'='*70}")
                if exp_id == '4':
                    print_summary(exp_results, ['system', 'rate'])
                elif exp_id == '5':
                    print_summary(exp_results, ['system', 'variable'])
    else:
        print("No successful runs. Check errors above.")


if __name__ == '__main__':
    main()
