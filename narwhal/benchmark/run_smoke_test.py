#!/usr/bin/env python3
"""
Quick smoke test: 2 systems x 1 run x 20s @ 50K rate, 4 nodes.
Verifies that the new stablecoin metrics (TPS, latency, success rate) work.
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from run_e2e import (
    NODE_PARAMS, TX_SIZE, FIELDNAMES,
    SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE,
    run_single_benchmark, make_result_row, parse_summary,
    leap_threads_for_nodes, print_summary, write_csv,
)

# Override globals for quick test
DURATION_OVERRIDE = 20  # seconds
RATE = 50_000
NODES = 4
WORKERS = 1

# Temporarily patch the module-level DURATION
import run_e2e
run_e2e.DURATION = DURATION_OVERRIDE

SYSTEMS = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

results = []
for sys_name, features, env_base in SYSTEMS:
    metrics = run_single_benchmark(
        sys_name, NODES, WORKERS, RATE, 1, features, env_base,
    )
    if metrics['status'] == 'ok':
        row = make_result_row(
            'smoke', sys_name, RATE, NODES, WORKERS, RATE, 1, metrics,
        )
        results.append(row)
        # Print key new metrics
        print(f"\n  >>> {sys_name} NEW METRICS:")
        print(f"      Stablecoin TPS:     {row['stablecoin_tps']}")
        print(f"      Stablecoin Latency: {row['stablecoin_latency_ms']} ms")
        print(f"      Success Rate:       {row['success_rate']}")
        print(f"      Total Txns:         {row['total_txns']}")
        print(f"      Successful Txns:    {row['successful_txns']}")
    else:
        print(f"  >>> {sys_name} FAILED")
    time.sleep(3)

if results:
    print(f"\n{'='*70}")
    print("  SMOKE TEST SUMMARY")
    print(f"{'='*70}")
    print_summary(results, ['system'])
