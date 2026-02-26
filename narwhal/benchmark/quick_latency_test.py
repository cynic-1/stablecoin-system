#!/usr/bin/env python3
"""Quick latency comparison: Tusk vs MP3-BFT++ k=4, single run each."""
import os, sys, re, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from benchmark.local import LocalBench
from benchmark.utils import BenchError

NODE_PARAMS = {
    'header_size': 1_000, 'max_header_delay': 200, 'gc_depth': 50,
    'sync_retry_delay': 10_000, 'sync_retry_nodes': 3,
    'batch_size': 500_000, 'max_batch_delay': 200,
}

CONFIGS = [
    ('Tusk',          None,     {},                          50_000),
    ('MP3-BFT++_k1', 'mp3bft', {'MP3BFT_K_SLOTS': '1'},    50_000),
    ('MP3-BFT++_k4', 'mp3bft', {'MP3BFT_K_SLOTS': '4'},    50_000),
]

def extract(pattern, text):
    m = re.search(pattern, text)
    return float(m.group(1).replace(',', '')) if m else 0.0

print(f"{'Protocol':<20} {'Con.TPS':>10} {'Con.Lat':>10} {'E2E TPS':>10} {'E2E Lat':>10}")
print('-' * 72)

for name, feat, env, rate in CONFIGS:
    bench_params = {
        'faults': 0, 'nodes': 4, 'workers': 1,
        'rate': rate, 'tx_size': 512, 'duration': 30,
    }
    try:
        bench = LocalBench(bench_params, NODE_PARAMS, extra_features=feat, env_vars=env)
        summary = bench.run(debug=False).result()
        ctps = extract(r'Consensus TPS:\s+([\d,]+)', summary)
        clat = extract(r'Consensus latency:\s+([\d,]+)', summary)
        etps = extract(r'End-to-end TPS:\s+([\d,]+)', summary)
        elat = extract(r'End-to-end latency:\s+([\d,]+)', summary)
        print(f'{name:<20} {ctps:>10,.0f} {clat:>10,.0f}ms {etps:>10,.0f} {elat:>10,.0f}ms')
    except BenchError as e:
        print(f'{name:<20} ERROR: {e}')
    time.sleep(3)
