#!/usr/bin/env python3
"""
E2E Thread Scaling Experiment — LEAP vs LeapBase at different thread counts.

Tests H90%@100K with LEAP_THREADS = 2, 4, 8 to show execution advantage
scaling with thread count in the full consensus+execution pipeline.

n=4 nodes, so LEAP_THREADS > 2 means oversubscription on 8-core machine,
but demonstrates the trend.
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
    run_single_benchmark,
    make_result_row,
    write_csv,
    print_summary,
)

RUNS = 2
NODES = 4
WORKERS = 1
RATE = 100_000
PATTERN = 'Hotspot_90pct'
THREAD_COUNTS = [2, 4, 8]

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp3_e2e', 'results', 'raw',
)

def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    csv_path = os.path.join(OUTPUT_DIR, 'exp3_e2e_threads.csv')

    results = []
    total = len(THREAD_COUNTS) * 2 * RUNS  # threads × systems × runs
    idx = 0

    for threads in THREAD_COUNTS:
        for sys_name, features, env_base in [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]:
            for run_id in range(1, RUNS + 1):
                idx += 1
                print(f"\n[Thread Exp: {idx}/{total}]")

                env = dict(env_base)
                env['LEAP_PATTERN'] = PATTERN
                env['BENCH_TX_SIZE'] = str(TX_SIZE)
                # Override thread count
                env['LEAP_THREADS'] = str(threads)
                env['RAYON_NUM_THREADS'] = str(threads)

                metrics = run_single_benchmark(
                    sys_name, NODES, WORKERS, RATE, run_id,
                    features, env,
                )
                if metrics['status'] == 'ok':
                    variable = f'H90%@100K_{threads}T'
                    results.append(make_result_row(
                        'Exp-Threads', sys_name, variable,
                        NODES, WORKERS, RATE, run_id, metrics,
                    ))
                time.sleep(3)

    write_csv(results, csv_path, FIELDNAMES)
    print(f"\nResults: {csv_path}")
    print(f"Total runs: {len(results)}/{total}")

    # Print summary
    from collections import defaultdict
    groups = defaultdict(list)
    for r in results:
        key = (r['system'], r['variable'])
        groups[key].append(r)

    print(f"\n{'System':<18} {'Config':<20} {'SC.TPS':>8} {'ExecLat':>8}")
    print("-" * 60)
    for (sys, var), runs in sorted(groups.items()):
        sc_tps = sum(float(r['stablecoin_tps']) for r in runs) / len(runs)
        exec_lat = sum(float(r['with_exec_latency_ms']) for r in runs) / len(runs)
        print(f"{sys:<18} {var:<20} {sc_tps:>8,.0f} {exec_lat:>8,.0f}")


if __name__ == '__main__':
    main()
