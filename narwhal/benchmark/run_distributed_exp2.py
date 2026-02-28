#!/usr/bin/env python3
"""
Distributed Exp2: Tusk vs MP3-BFT++ — runs on real separate servers.

Mirrors run_comparison.py but uses StaticBench (SSH to hosts in hosts.json)
instead of LocalBench (localhost simulation).

Key differences from localhost version:
  - LEAP_THREADS is not capped by node count (each server is dedicated)
  - Node count is limited by len(hosts) in hosts.json
  - No NUMA pinning (each node has a full machine)

Usage:
    python3 run_distributed_exp2.py              # all experiments: A B C
    python3 run_distributed_exp2.py A            # only Exp A (rate scaling)
    python3 run_distributed_exp2.py A B          # Exp A + B

Output:
    experiments/exp2_consensus/results/raw/exp2_distributed_*.csv
"""

import csv
import os
import re
import shutil
import sys
import time
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
# PathMaker uses relative paths (e.g. ../node) — must run from benchmark dir.
os.chdir(os.path.dirname(os.path.abspath(__file__)))

from benchmark.static import StaticBench, StaticInstanceManager
from benchmark.utils import BenchError, Print

HOSTS_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'hosts.json')

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp2_consensus', 'results', 'raw',
)

# ── Config ────────────────────────────────────────────────────────────────────

TX_SIZE   = 512
DURATION  = 60   # seconds per run
RUNS      = 3    # runs per configuration

NODE_PARAMS = {
    'header_size':      1_000,
    'max_header_delay': 200,
    'gc_depth':         50,
    'sync_retry_delay': 10_000,
    'sync_retry_nodes': 3,
    'batch_size':       500_000,
    'max_batch_delay':  200,
}

PROTOCOLS_ALL = [
    ('Tusk',          {'CONSENSUS_PROTOCOL': 'tusk'}),
    ('MP3-BFT++_k1', {'MP3BFT_K_SLOTS': '1'}),
    ('MP3-BFT++_k2', {'MP3BFT_K_SLOTS': '2'}),
    ('MP3-BFT++_k4', {'MP3BFT_K_SLOTS': '4'}),
]

PROTOCOLS_SCALING = [
    ('Tusk',          {'CONSENSUS_PROTOCOL': 'tusk'}),
    ('MP3-BFT++_k4', {'MP3BFT_K_SLOTS': '4'}),
]

# Exp A: rate scaling
EXP_A_RATES   = [10_000, 30_000, 50_000, 70_000, 100_000]
EXP_A_NODES   = 4
EXP_A_WORKERS = 1

# Exp B: workers scaling
EXP_B_WORKERS_LIST = [1, 4]
EXP_B_NODES        = 4
EXP_B_RATE         = 50_000

# Exp C: committee scaling (bounded by available servers)
EXP_C_NODES_LIST = [4]   # extended to [4, 10] if ≥10 servers in hosts.json
EXP_C_WORKERS    = 1
EXP_C_RATE       = 50_000

FIELDNAMES = [
    'experiment', 'protocol', 'k', 'nodes', 'workers', 'rate', 'run',
    'consensus_tps', 'consensus_bps', 'consensus_latency_ms',
    'e2e_tps', 'e2e_bps', 'e2e_latency_ms', 'duration_s',
]

# ── Helpers ────────────────────────────────────────────────────────────────────

def parse_summary(text):
    def extract(pattern):
        m = re.search(pattern, text)
        return float(m.group(1).replace(',', '')) if m else 0.0
    return {
        'consensus_tps':        extract(r'Consensus TPS:\s+([\d,]+)'),
        'consensus_bps':        extract(r'Consensus BPS:\s+([\d,]+)'),
        'consensus_latency_ms': extract(r'Consensus latency:\s+([\d,]+)'),
        'e2e_tps':              extract(r'End-to-end TPS:\s+([\d,]+)'),
        'e2e_bps':              extract(r'End-to-end BPS:\s+([\d,]+)'),
        'e2e_latency_ms':       extract(r'End-to-end latency:\s+([\d,]+)'),
        'duration_s':           extract(r'Execution time:\s+([\d,]+)'),
    }


SAVED_LOGS_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    'saved_logs', 'exp2',
)


def save_logs(tag, proto_name, rate, run_id):
    """Copy logs/ to saved_logs/exp2/{tag}_{proto}_{rate}_run{id}/ for later inspection."""
    src = 'logs'
    if not os.path.isdir(src):
        return
    slug = proto_name.replace('+', '_').replace(' ', '_')
    dest = os.path.join(SAVED_LOGS_DIR, f'{tag}_{slug}_rate{rate}_run{run_id}')
    os.makedirs(dest, exist_ok=True)
    for f in os.listdir(src):
        shutil.copy2(os.path.join(src, f), os.path.join(dest, f))
    print(f"  → Logs saved: {dest}")


def run_single(bench, proto_name, nodes, workers, rate, run_id, env_vars,
               tag=''):
    bench_params = {
        'faults':   0,
        'nodes':    nodes,
        'workers':  workers,
        'collocate': True,
        'rate':     rate,
        'tx_size':  TX_SIZE,
        'duration': DURATION,
        'runs':     1,
    }
    print(f"\n{'='*70}")
    print(f"  {proto_name} | n={nodes} w={workers} rate={rate:,} | run={run_id}")
    print(f"{'='*70}")
    try:
        result = bench.run(bench_params, NODE_PARAMS, debug=False,
                           skip_update=True, env_vars=env_vars)
        if result is None:
            print("  ERROR: Benchmark failed (no results — check remote logs)")
            return {'status': 'error'}
        save_logs(tag, proto_name, rate, run_id)
        summary = result.result()
        print(summary)
        metrics = parse_summary(summary)
        metrics['status'] = 'ok'
        return metrics
    except BenchError as e:
        print(f"  ERROR: {e}")
        return {'status': 'error'}


def make_row(experiment, proto_name, env_vars, nodes, workers, rate, run_id, metrics):
    k = int(env_vars.get('MP3BFT_K_SLOTS', 0))
    return {
        'experiment':           experiment,
        'protocol':             proto_name,
        'k':                    k,
        'nodes':                nodes,
        'workers':              workers,
        'rate':                 rate,
        'run':                  run_id,
        'consensus_tps':        metrics['consensus_tps'],
        'consensus_bps':        metrics['consensus_bps'],
        'consensus_latency_ms': metrics['consensus_latency_ms'],
        'e2e_tps':              metrics['e2e_tps'],
        'e2e_bps':              metrics['e2e_bps'],
        'e2e_latency_ms':       metrics['e2e_latency_ms'],
        'duration_s':           metrics['duration_s'],
    }


def write_csv(results, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=FIELDNAMES, extrasaction='ignore')
        w.writeheader()
        w.writerows(results)
    print(f"  → Saved: {path}")


def print_summary(results, group_keys):
    grouped = defaultdict(list)
    for r in results:
        grouped[tuple(r[k] for k in group_keys)].append(r)
    header = ''.join(f'{k:<16}' for k in group_keys)
    print(f"{header} {'Con.TPS':>10} {'Con.Lat(ms)':>12} {'E2E TPS':>10} {'E2E Lat(ms)':>12}")
    print('-' * (16 * len(group_keys) + 46))
    for key, runs in sorted(grouped.items()):
        avg_ctps = sum(r['consensus_tps']        for r in runs) / len(runs)
        avg_clat = sum(r['consensus_latency_ms'] for r in runs) / len(runs)
        avg_etps = sum(r['e2e_tps']              for r in runs) / len(runs)
        avg_elat = sum(r['e2e_latency_ms']       for r in runs) / len(runs)
        row = ''.join(f'{str(v):<16}' for v in key)
        print(f"{row} {avg_ctps:>10,.0f} {avg_clat:>12,.0f} {avg_etps:>10,.0f} {avg_elat:>12,.0f}")


# ── Experiment runners ─────────────────────────────────────────────────────────

def run_exp_a(bench):
    print(f"\n{'#'*70}")
    print(f"  Exp A: Rate Scaling — n={EXP_A_NODES} w={EXP_A_WORKERS} rates={EXP_A_RATES}")
    print(f"{'#'*70}")
    total = len(PROTOCOLS_ALL) * len(EXP_A_RATES) * RUNS
    done, results = 0, []
    for proto, env in PROTOCOLS_ALL:
        for rate in EXP_A_RATES:
            for run_id in range(1, RUNS + 1):
                done += 1
                print(f"\n[A: {done}/{total}] ", end='')
                m = run_single(bench, proto, EXP_A_NODES, EXP_A_WORKERS,
                               rate, run_id, env, tag='A')
                if m['status'] == 'ok':
                    results.append(make_row('A_rate', proto, env,
                                            EXP_A_NODES, EXP_A_WORKERS, rate, run_id, m))
                time.sleep(3)
    return results


def run_exp_b(bench):
    print(f"\n{'#'*70}")
    print(f"  Exp B: Workers Scaling — n={EXP_B_NODES} workers={EXP_B_WORKERS_LIST} rate={EXP_B_RATE}")
    print(f"{'#'*70}")
    total = len(PROTOCOLS_SCALING) * len(EXP_B_WORKERS_LIST) * RUNS
    done, results = 0, []
    for proto, env in PROTOCOLS_SCALING:
        for w in EXP_B_WORKERS_LIST:
            for run_id in range(1, RUNS + 1):
                done += 1
                print(f"\n[B: {done}/{total}] ", end='')
                m = run_single(bench, proto, EXP_B_NODES, w,
                               EXP_B_RATE, run_id, env, tag='B')
                if m['status'] == 'ok':
                    results.append(make_row('B_workers', proto, env,
                                            EXP_B_NODES, w, EXP_B_RATE, run_id, m))
                time.sleep(3)
    return results


def run_exp_c(bench, available_hosts):
    # Extend node list if we have enough servers.
    nodes_list = EXP_C_NODES_LIST[:]
    if available_hosts >= 10 and 10 not in nodes_list:
        nodes_list.append(10)
    if available_hosts >= 20 and 20 not in nodes_list:
        nodes_list.append(20)

    print(f"\n{'#'*70}")
    print(f"  Exp C: Committee Scaling — nodes={nodes_list} rate={EXP_C_RATE}")
    print(f"  (available servers: {available_hosts})")
    print(f"{'#'*70}")
    total = len(PROTOCOLS_SCALING) * len(nodes_list) * RUNS
    done, results = 0, []
    for proto, env in PROTOCOLS_SCALING:
        for nodes in nodes_list:
            for run_id in range(1, RUNS + 1):
                done += 1
                print(f"\n[C: {done}/{total}] ", end='')
                m = run_single(bench, proto, nodes, EXP_C_WORKERS,
                               EXP_C_RATE, run_id, env, tag='C')
                if m['status'] == 'ok':
                    results.append(make_row('C_nodes', proto, env,
                                            nodes, EXP_C_WORKERS, EXP_C_RATE, run_id, m))
                time.sleep(3)
    return results


# ── Main ───────────────────────────────────────────────────────────────────────

def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)

    # Validate hosts.json before starting any benchmarks.
    try:
        manager = StaticInstanceManager(HOSTS_FILE)
        available = len(manager.hosts(flat=True))
    except Exception as e:
        print(f"ERROR: Cannot load {HOSTS_FILE}: {e}")
        print("       Edit hosts.json with your server IPs and SSH key path.")
        sys.exit(1)

    print(f"Distributed Exp2: Tusk vs MP3-BFT++")
    print(f"=====================================")
    print(f"Servers in hosts.json: {available}")
    manager.print_info()

    if available < 4:
        print("ERROR: Need at least 4 servers for a 4-node BFT committee.")
        sys.exit(1)

    exps = [e.upper() for e in sys.argv[1:]] if len(sys.argv) > 1 else ['A', 'B', 'C']
    print(f"Running experiments: {exps}")
    print()

    # Create ONE bench instance (reused for all runs — avoids fd leak).
    # Compile with superset features so binary supports all protocols.
    bench = StaticBench(extra_features='mp3bft', hosts_file=HOSTS_FILE)

    # One-time update: git pull + compile on all remote servers.
    print("Updating remote servers (git pull + compile) ...")
    try:
        bench.update(manager.hosts(flat=True))
        print("Remote servers updated.\n")
    except BenchError as e:
        print(f"ERROR: Failed to update remote servers: {e}")
        sys.exit(1)

    start = time.time()
    all_results = []

    if 'A' in exps:
        r = run_exp_a(bench)
        all_results.extend(r)
        if r:
            write_csv(r, os.path.join(OUTPUT_DIR, 'exp2_distributed_A.csv'))

    if 'B' in exps:
        r = run_exp_b(bench)
        all_results.extend(r)
        if r:
            write_csv(r, os.path.join(OUTPUT_DIR, 'exp2_distributed_B.csv'))

    if 'C' in exps:
        r = run_exp_c(bench, available)
        all_results.extend(r)
        if r:
            write_csv(r, os.path.join(OUTPUT_DIR, 'exp2_distributed_C.csv'))

    elapsed = time.time() - start
    print(f"\n\nDone in {elapsed/60:.1f} min. Successful: {len(all_results)} runs.")

    if all_results:
        combined = os.path.join(OUTPUT_DIR, 'exp2_distributed_all.csv')
        write_csv(all_results, combined)

        for exp_id in exps:
            tag = {'A': 'A_rate', 'B': 'B_workers', 'C': 'C_nodes'}[exp_id]
            sub = [r for r in all_results if r['experiment'] == tag]
            if sub:
                print(f"\n{'='*70}\n  Exp {exp_id} Summary\n{'='*70}")
                keys = {'A': ['protocol', 'rate'], 'B': ['protocol', 'workers'],
                        'C': ['protocol', 'nodes']}[exp_id]
                print_summary(sub, keys)
    else:
        print("No successful runs. Check SSH access and hosts.json.")


if __name__ == '__main__':
    main()
