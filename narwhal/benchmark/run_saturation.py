#!/usr/bin/env python3
"""
Saturation curve: Tusk vs MP3-BFT++ k=4 at very high rates.

Rates tested: 100K (baseline), 150K, 200K, 300K, 500K, 750K, 1M
2 runs each -> 14 runs total, ~20-30 min.

Goal: find the data-plane saturation point and confirm TPS parity
between protocols across the full range.
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


TX_SIZE   = 512
DURATION  = 60
RUNS      = 2

NODE_PARAMS = {
    'header_size':     1_000,
    'max_header_delay': 200,
    'gc_depth':         50,
    'sync_retry_delay': 10_000,
    'sync_retry_nodes': 3,
    'batch_size':       500_000,
    'max_batch_delay':  200,
}

PROTOCOLS = [
    ('Tusk',     None,     {}),
    ('MP3-BFT++_k4', 'mp3bft', {'MP3BFT_K_SLOTS': '4'}),
]

# Extend beyond existing 100K to find the ceiling.
RATES = [100_000, 150_000, 200_000, 300_000, 500_000, 750_000, 1_000_000]

NODES   = 4
WORKERS = 1

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp2_consensus', 'results', 'raw',
)

FIELDNAMES = [
    'protocol', 'k', 'rate', 'run',
    'consensus_tps', 'consensus_bps', 'consensus_latency_ms',
    'e2e_tps', 'e2e_bps', 'e2e_latency_ms', 'duration_s',
]


def parse_summary(text):
    def get(pat):
        m = re.search(pat, text)
        return float(m.group(1).replace(',', '')) if m else 0.0
    return {
        'consensus_tps':        get(r'Consensus TPS:\s+([\d,]+)'),
        'consensus_bps':        get(r'Consensus BPS:\s+([\d,]+)'),
        'consensus_latency_ms': get(r'Consensus latency:\s+([\d,]+)'),
        'e2e_tps':              get(r'End-to-end TPS:\s+([\d,]+)'),
        'e2e_bps':              get(r'End-to-end BPS:\s+([\d,]+)'),
        'e2e_latency_ms':       get(r'End-to-end latency:\s+([\d,]+)'),
        'duration_s':           get(r'Execution time:\s+([\d,]+)'),
    }


def run_one(proto_name, rate, run_id, extra_feat, env_vars):
    bench_params = {
        'faults': 0, 'nodes': NODES, 'workers': WORKERS,
        'rate': rate, 'tx_size': TX_SIZE, 'duration': DURATION,
    }
    print(f"\n{'='*70}")
    print(f"  {proto_name} | rate={rate:,} | run={run_id}")
    print(f"{'='*70}")
    try:
        bench = LocalBench(bench_params, NODE_PARAMS,
                           extra_features=extra_feat,
                           env_vars=env_vars)
        result = bench.run(debug=True)
        summary = result.result()
        print(summary)
        m = parse_summary(summary)
        m['status'] = 'ok'
        return m
    except BenchError as e:
        print(f"  ERROR: {e}")
        return {'status': 'error'}


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    csv_path = os.path.join(OUTPUT_DIR, 'exp2_saturation.csv')

    total = len(PROTOCOLS) * len(RATES) * RUNS
    done  = 0
    results = []

    for proto_name, extra_feat, env in PROTOCOLS:
        k_val = env.get('MP3BFT_K_SLOTS', '0')
        for rate in RATES:
            for run_id in range(1, RUNS + 1):
                done += 1
                print(f"\n[{done}/{total}] ", end='')
                m = run_one(proto_name, rate, run_id, extra_feat, env)
                if m['status'] == 'ok':
                    results.append({
                        'protocol': proto_name,
                        'k': int(k_val),
                        'rate': rate,
                        'run': run_id,
                        **{k: m[k] for k in FIELDNAMES[4:]},
                    })
                    # Save incrementally after every run.
                    with open(csv_path, 'w', newline='') as f:
                        writer = csv.DictWriter(f, fieldnames=FIELDNAMES)
                        writer.writeheader()
                        writer.writerows(results)
                time.sleep(3)

    # Final summary table.
    print(f"\n\n{'='*80}")
    print("  SATURATION SUMMARY")
    print(f"{'='*80}")
    print(f"{'Protocol':<22} {'Rate':>10} {'TPS':>10} {'%offered':>9} {'Latency':>10}")
    print('-' * 65)

    grouped = defaultdict(list)
    for r in results:
        grouped[(r['protocol'], r['rate'])].append(r)

    for (proto, rate), runs in sorted(grouped.items(), key=lambda x: (x[0][0], x[0][1])):
        avg_tps = sum(r['consensus_tps'] for r in runs) / len(runs)
        avg_lat = sum(r['consensus_latency_ms'] for r in runs) / len(runs)
        pct     = avg_tps / rate * 100
        print(f"{proto:<22} {rate:>10,} {avg_tps:>10,.0f} {pct:>8.1f}% {avg_lat:>9,.0f}ms")

    print(f"\nResults saved to: {csv_path}")


if __name__ == '__main__':
    main()
