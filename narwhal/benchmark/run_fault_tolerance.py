#!/usr/bin/env python3
"""
Exp 2D: Crash-Fault Tolerance Benchmark

Tests consensus performance under crash faults (nodes not started).
n=4 fixed, f in {0, 1}, Tusk vs MP3-BFT++ k=4, rates 10K/50K/100K.

BFT guarantee: n=4, f=1 is the maximum tolerable crash faults (3f+1=4).
Faulty nodes are simply not started — the framework skips the last f nodes.

Results saved to experiments/exp2_consensus/results/raw/exp2_fault_tolerance.csv
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
DURATION = 60
RUNS = 3

NODES = 4
WORKERS = 1
FAULTS_LIST = [0, 1]
RATES = [10_000, 50_000, 100_000]

NODE_PARAMS = {
    'header_size': 1_000,
    'max_header_delay': 200,
    'gc_depth': 50,
    'sync_retry_delay': 10_000,
    'sync_retry_nodes': 3,
    'batch_size': 500_000,
    'max_batch_delay': 200,
}

PROTOCOLS = [
    ('Tusk',          None,     {}),
    ('MP3-BFT++_k4', 'mp3bft', {'MP3BFT_K_SLOTS': '4'}),
]

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp2_consensus', 'results', 'raw',
)

FIELDNAMES = [
    'protocol', 'k', 'faults', 'nodes', 'workers', 'rate', 'run',
    'consensus_tps', 'consensus_bps', 'consensus_latency_ms',
    'e2e_tps', 'e2e_bps', 'e2e_latency_ms', 'duration_s',
]


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


def run_single(protocol_name, faults, rate, run_id, extra_features, env_vars):
    """Run a single benchmark and return parsed metrics."""
    bench_params = {
        'faults': faults,
        'nodes': NODES,
        'workers': WORKERS,
        'rate': rate,
        'tx_size': TX_SIZE,
        'duration': DURATION,
    }

    print(f"\n{'='*70}")
    print(f"  {protocol_name} | f={faults} rate={rate:,} | run={run_id}")
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


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)

    total = len(PROTOCOLS) * len(FAULTS_LIST) * len(RATES) * RUNS
    print(f"Exp 2D: Crash-Fault Tolerance Benchmark")
    print(f"========================================")
    print(f"Protocols: {[p[0] for p in PROTOCOLS]}")
    print(f"Faults: {FAULTS_LIST}, Rates: {RATES}")
    print(f"Total benchmark runs: {total}")
    print(f"Estimated time: ~{total * (DURATION + 15) // 60} minutes")
    print()

    start_time = time.time()
    completed = 0
    results = []

    for proto_name, extra_feat, env in PROTOCOLS:
        for faults in FAULTS_LIST:
            for rate in RATES:
                for run_id in range(1, RUNS + 1):
                    completed += 1
                    print(f"\n[{completed}/{total}] ", end='')
                    metrics = run_single(
                        proto_name, faults, rate, run_id,
                        extra_feat, env,
                    )
                    if metrics['status'] == 'ok':
                        k_val = env.get('MP3BFT_K_SLOTS', '0')
                        results.append({
                            'protocol': proto_name,
                            'k': int(k_val),
                            'faults': faults,
                            'nodes': NODES,
                            'workers': WORKERS,
                            'rate': rate,
                            'run': run_id,
                            'consensus_tps': metrics['consensus_tps'],
                            'consensus_bps': metrics['consensus_bps'],
                            'consensus_latency_ms': metrics['consensus_latency_ms'],
                            'e2e_tps': metrics['e2e_tps'],
                            'e2e_bps': metrics['e2e_bps'],
                            'e2e_latency_ms': metrics['e2e_latency_ms'],
                            'duration_s': metrics['duration_s'],
                        })
                    time.sleep(3)

    elapsed = time.time() - start_time
    print(f"\n\nAll benchmarks complete in {elapsed/60:.1f} minutes.")
    print(f"Successful runs: {len(results)}/{total}")

    if results:
        csv_path = os.path.join(OUTPUT_DIR, 'exp2_fault_tolerance.csv')
        with open(csv_path, 'w', newline='') as f:
            writer = csv.DictWriter(f, fieldnames=FIELDNAMES, extrasaction='ignore')
            writer.writeheader()
            writer.writerows(results)
        print(f"Results saved to: {csv_path}")

        # Print summary table
        print(f"\n{'='*70}")
        print(f"  SUMMARY (averaged across {RUNS} runs)")
        print(f"{'='*70}")
        print(f"{'Protocol':<18} {'Faults':<8} {'Rate':<10} {'Con.TPS':>10} {'Con.Lat':>10} {'E2E TPS':>10} {'E2E Lat':>10}")
        print('-' * 86)

        grouped = defaultdict(list)
        for r in results:
            key = (r['protocol'], r['faults'], r['rate'])
            grouped[key].append(r)

        for key in sorted(grouped.keys()):
            runs = grouped[key]
            proto, faults, rate = key
            avg_ctps = sum(r['consensus_tps'] for r in runs) / len(runs)
            avg_clat = sum(r['consensus_latency_ms'] for r in runs) / len(runs)
            avg_etps = sum(r['e2e_tps'] for r in runs) / len(runs)
            avg_elat = sum(r['e2e_latency_ms'] for r in runs) / len(runs)
            print(f"{proto:<18} {faults:<8} {rate:<10,} {avg_ctps:>10,.0f} {avg_clat:>10,.0f} {avg_etps:>10,.0f} {avg_elat:>10,.0f}")
    else:
        print("No successful runs. Check errors above.")


if __name__ == '__main__':
    main()
