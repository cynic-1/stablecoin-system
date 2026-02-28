#!/usr/bin/env python3
"""
Quick verification: Tusk vs MP3-BFT++ k=4 on distributed servers.

Runs ONE rate point (50K), ONE run each, 20s duration.
Purpose: confirm CONSENSUS_PROTOCOL env var works — Tusk and MP3 should
show different latency (~580ms vs ~900ms if both were k=4 before fix).

Expected result after fix:
  - Tusk latency: ~900-1000ms (Tusk's 2-round leader cadence)
  - MP3 k=4 latency: ~550-650ms (pipeline cadence, 3-chain)
  - If both show ~same latency → fix didn't work

Usage:
    python3 run_quick_verify.py
    python3 run_quick_verify.py --rate 100000 --duration 30
"""

import argparse
import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from benchmark.static import StaticBench, StaticInstanceManager
from benchmark.utils import BenchError, Print

HOSTS_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'hosts.json')

NODE_PARAMS = {
    'header_size':      1_000,
    'max_header_delay': 200,
    'gc_depth':         50,
    'sync_retry_delay': 10_000,
    'sync_retry_nodes': 3,
    'batch_size':       500_000,
    'max_batch_delay':  200,
}

PROTOCOLS = [
    ('Tusk',          {'CONSENSUS_PROTOCOL': 'tusk'}),
    ('MP3-BFT++_k4', {'MP3BFT_K_SLOTS': '4'}),
]


def parse_summary(text):
    def extract(pattern):
        m = re.search(pattern, text)
        return float(m.group(1).replace(',', '')) if m else 0.0
    return {
        'consensus_tps':        extract(r'Consensus TPS:\s+([\d,]+)'),
        'consensus_latency_ms': extract(r'Consensus latency:\s+([\d,]+)'),
        'e2e_tps':              extract(r'End-to-end TPS:\s+([\d,]+)'),
        'e2e_latency_ms':       extract(r'End-to-end latency:\s+([\d,]+)'),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--rate', type=int, default=50_000, help='Input rate (default: 50000)')
    parser.add_argument('--duration', type=int, default=20, help='Seconds per run (default: 20)')
    parser.add_argument('--skip-update', action='store_true', help='Skip git pull + compile')
    args = parser.parse_args()

    try:
        manager = StaticInstanceManager(HOSTS_FILE)
        available = len(manager.hosts(flat=True))
    except Exception as e:
        print(f"ERROR: Cannot load {HOSTS_FILE}: {e}")
        sys.exit(1)

    if available < 4:
        print(f"ERROR: Need >= 4 servers, have {available}")
        sys.exit(1)

    print(f"Quick Verify: Tusk vs MP3-BFT++ k=4")
    print(f"====================================")
    print(f"Servers: {available}, Rate: {args.rate:,}, Duration: {args.duration}s")
    manager.print_info()
    print()

    bench = StaticBench(extra_features='mp3bft', hosts_file=HOSTS_FILE)

    if not args.skip_update:
        print("Updating remote servers (git pull + compile) ...")
        try:
            bench.update(manager.hosts(flat=True))
            print("Done.\n")
        except BenchError as e:
            print(f"ERROR: {e}")
            sys.exit(1)

    bench_params = {
        'faults':    0,
        'nodes':     4,
        'workers':   1,
        'collocate': True,
        'rate':      args.rate,
        'tx_size':   512,
        'duration':  args.duration,
        'runs':      1,
    }

    results = {}
    for proto_name, env_vars in PROTOCOLS:
        print(f"\n{'='*60}")
        print(f"  Running: {proto_name}")
        print(f"{'='*60}")
        try:
            result = bench.run(bench_params, NODE_PARAMS, debug=False,
                               skip_update=True, env_vars=env_vars)
            if result is None:
                print(f"  ERROR: No results for {proto_name}")
                continue
            summary = result.result()
            print(summary)
            results[proto_name] = parse_summary(summary)
        except BenchError as e:
            print(f"  ERROR: {e}")
        time.sleep(3)

    # Compare
    print(f"\n{'='*60}")
    print(f"  COMPARISON")
    print(f"{'='*60}")
    if len(results) < 2:
        print("  Not enough results to compare.")
        return

    fmt = "{:<16} {:>12} {:>12} {:>12} {:>12}"
    print(fmt.format('Protocol', 'Con.TPS', 'Con.Lat(ms)', 'E2E TPS', 'E2E Lat(ms)'))
    print('-' * 66)
    for name in ['Tusk', 'MP3-BFT++_k4']:
        if name in results:
            r = results[name]
            print(fmt.format(name,
                             f"{r['consensus_tps']:,.0f}",
                             f"{r['consensus_latency_ms']:,.0f}",
                             f"{r['e2e_tps']:,.0f}",
                             f"{r['e2e_latency_ms']:,.0f}"))

    tusk = results.get('Tusk', {})
    mp3 = results.get('MP3-BFT++_k4', {})
    if tusk and mp3 and tusk['consensus_latency_ms'] > 0:
        lat_diff = (mp3['consensus_latency_ms'] - tusk['consensus_latency_ms']) / tusk['consensus_latency_ms'] * 100
        print(f"\n  Latency diff: MP3 vs Tusk = {lat_diff:+.1f}%")
        if abs(lat_diff) < 5:
            print("  WARNING: Latency nearly identical — protocols may still be running same code!")
        elif lat_diff < -10:
            print("  OK: MP3-BFT++ shows significantly lower latency — fix is working.")
        else:
            print("  NOTE: Tusk has lower latency — unexpected, check logs.")


if __name__ == '__main__':
    main()
