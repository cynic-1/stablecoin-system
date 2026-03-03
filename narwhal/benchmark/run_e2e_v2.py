#!/usr/bin/env python3
"""
Distributed E2E Experiment v2 — Redesigned for Thesis Chapter 5

Runs on real separate servers via StaticBench (SSH to hosts in hosts.json).

Calibrated to match Exp-1 insights:
  - Serial TPS ≈ 5000 (via LEAP_TARGET_TPS calibration on each server)
  - Fixed 10K txn execution blocks (via LEAP_BLOCK_SIZE)
  - 10K accounts (best LEAP advantage range from Exp-1)

Two experiments:
  A. Contention Scaling  — 2 systems × 5 patterns × 3 runs = 30
  B. Rate Scaling (H90%) — 2 systems × 4 rates × 3 runs    = 24

Total: 54 runs (~1 hour)
Output: experiments/exp3_e2e/results/raw/exp3_e2e_v2.csv

Usage:
  python3 run_e2e_v2.py                    # run all: A B
  python3 run_e2e_v2.py A                  # only Exp A
  python3 run_e2e_v2.py B --threads 32     # Exp B with 32 vCPUs/server
"""

import argparse
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
from benchmark.utils import BenchError

HOSTS_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'hosts.json')

OUTPUT_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    '..', '..', 'experiments', 'exp3_e2e', 'results', 'raw',
)

# ── Config ────────────────────────────────────────────────────────────────────

TX_SIZE  = 512
DURATION = 60
RUNS     = 3

NODE_PARAMS = {
    'header_size':      1_000,
    'max_header_delay': 200,
    'gc_depth':         50,
    'sync_retry_delay': 10_000,
    'sync_retry_nodes': 3,
    'batch_size':       500_000,
    'max_batch_delay':  200,
}

# Systems: (name, env_vars_base)
# Features compiled once as superset (e2e_exec,mp3bft). Protocol selection
# at runtime via env vars. LEAP_THREADS injected per run_single call.
SYSTEM_MP3_LEAP = (
    'MP3+LEAP',
    {
        'MP3BFT_K_SLOTS': '4',
        'LEAP_ENGINE': 'leap',
        'LEAP_TARGET_TPS': '5000',
        'LEAP_BLOCK_SIZE': '10000',
        'LEAP_ACCOUNTS': '10000',
    },
)

SYSTEM_TUSK_LEAPBASE = (
    'Tusk+LeapBase',
    {
        'CONSENSUS_PROTOCOL': 'tusk',
        'LEAP_ENGINE': 'leap_base',
        'LEAP_TARGET_TPS': '5000',
        'LEAP_BLOCK_SIZE': '10000',
        'LEAP_ACCOUNTS': '10000',
    },
)

# ── Experiment definitions ────────────────────────────────────────────────────

# Exp A: Contention Scaling — shows LEAP advantage grows with contention.
# At 10K accounts with serial TPS ≈ 5000, high contention patterns
# (Hotspot 70-90%) cause heavy conflicts in Block-STM.
# LEAP's Hot-Delta sharding resolves these, showing increasing advantage.
EXP_A_PATTERNS = ['Uniform', 'Hotspot_30pct', 'Hotspot_50pct', 'Hotspot_70pct', 'Hotspot_90pct']
EXP_A_RATE     = 50_000
EXP_A_NODES    = 4
EXP_A_SYSTEMS  = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

# Exp B: Rate Scaling under Hotspot 90% — shows throughput ceiling.
# At low rates, both systems keep up. As rate increases, execution becomes
# the bottleneck and LEAP maintains higher throughput via Hot-Delta sharding.
EXP_B_RATES    = [10_000, 30_000, 50_000, 100_000]
EXP_B_PATTERN  = 'Hotspot_90pct'
EXP_B_NODES    = 4
EXP_B_SYSTEMS  = [SYSTEM_MP3_LEAP, SYSTEM_TUSK_LEAPBASE]

# CSV columns — same as run_distributed_exp3.py for consistency.
FIELDNAMES = [
    'experiment', 'system', 'variable', 'nodes', 'workers', 'rate', 'run',
    'stablecoin_tps', 'stablecoin_latency_ms',
    'committed_txns', 'executed_txns', 'exec_ratio',
    'e2e_tps', 'e2e_bps', 'e2e_latency_ms',
    'with_exec_tps', 'with_exec_bps', 'with_exec_latency_ms',
    'duration_s',
]

# ── Helpers ───────────────────────────────────────────────────────────────────

def parse_summary(text):
    def extract(pattern):
        m = re.search(pattern, text)
        return float(m.group(1).replace(',', '')) if m else 0.0
    return {
        'stablecoin_tps':         extract(r'Stablecoin TPS:\s+([\d,]+)'),
        'stablecoin_latency_ms':  extract(r'Stablecoin latency:\s+([\d,]+)'),
        'committed_txns':         extract(r'Committed transactions:\s+([\d,]+)'),
        'executed_txns':          extract(r'Executed transactions:\s+([\d,]+)'),
        'exec_ratio':             extract(r'Execution ratio:\s+([\d.]+)'),
        'e2e_tps':                extract(r'End-to-end TPS:\s+([\d,]+)'),
        'e2e_bps':                extract(r'End-to-end BPS:\s+([\d,]+)'),
        'e2e_latency_ms':         extract(r'End-to-end latency:\s+([\d,]+)'),
        'with_exec_tps':          extract(r'With-execution TPS:\s+([\d,]+)'),
        'with_exec_bps':          extract(r'With-execution BPS:\s+([\d,]+)'),
        'with_exec_latency_ms':   extract(r'With-execution latency:\s+([\d,]+)'),
        'duration_s':             extract(r'Execution time:\s+([\d,]+)'),
    }


SAVED_LOGS_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    'saved_logs', 'exp3_v2',
)


def save_logs(tag, sys_name, rate, run_id):
    """Copy logs/ to saved_logs/exp3_v2/{tag}_{sys}_{rate}_run{id}/ for later inspection."""
    src = 'logs'
    if not os.path.isdir(src):
        return
    slug = sys_name.replace('+', '_')
    dest = os.path.join(SAVED_LOGS_DIR, f'{tag}_{slug}_rate{rate}_run{run_id}')
    os.makedirs(dest, exist_ok=True)
    for f in os.listdir(src):
        shutil.copy2(os.path.join(src, f), os.path.join(dest, f))
    print(f"  -> Logs saved: {dest}")


def run_single(bench, sys_name, nodes, workers, rate, run_id, env_vars,
               leap_threads, tokio_threads=4, tag=''):
    bench_params = {
        'faults':    0,
        'nodes':     nodes,
        'workers':   workers,
        'collocate': True,
        'rate':      rate,
        'tx_size':   TX_SIZE,
        'duration':  DURATION,
        'runs':      1,
    }
    env = dict(env_vars)
    env['BENCH_TX_SIZE'] = str(TX_SIZE)
    # Fixed seed: same seed across systems ensures identical txn sequences.
    env['LEAP_SEED'] = '42'
    # Limit tokio threads to avoid CPU contention with rayon.
    env['TOKIO_WORKER_THREADS'] = str(tokio_threads)
    # On real servers each node has its own machine -> reserve cores for rayon.
    if env.get('LEAP_ENGINE') not in ('serial',):
        env['LEAP_THREADS']      = str(leap_threads)
        env['RAYON_NUM_THREADS'] = str(leap_threads)

    print(f"\n{'='*70}")
    print(f"  {sys_name} | n={nodes} w={workers} rate={rate:,} | run={run_id}")
    print(f"  rayon={env.get('LEAP_THREADS', 'N/A')}, tokio={tokio_threads}")
    print(f"  target_tps={env.get('LEAP_TARGET_TPS', 'N/A')}, "
          f"block_size={env.get('LEAP_BLOCK_SIZE', 'N/A')}, "
          f"accounts={env.get('LEAP_ACCOUNTS', 'N/A')}")
    print(f"{'='*70}")
    try:
        result = bench.run(bench_params, NODE_PARAMS, debug=False,
                           skip_update=True, env_vars=env)
        if result is None:
            print("  ERROR: Benchmark failed (no results -- check remote logs)")
            return {'status': 'error'}
        save_logs(tag, sys_name, rate, run_id)
        summary = result.result()
        print(summary)
        metrics = parse_summary(summary)
        metrics['status'] = 'ok'
        return metrics
    except BenchError as e:
        print(f"  ERROR: {e}")
        return {'status': 'error'}


def make_row(tag, sys_name, variable, nodes, workers, rate, run_id, metrics):
    return {
        'experiment':             tag,
        'system':                 sys_name,
        'variable':               variable,
        'nodes':                  nodes,
        'workers':                workers,
        'rate':                   rate,
        'run':                    run_id,
        'stablecoin_tps':         metrics.get('stablecoin_tps', 0),
        'stablecoin_latency_ms':  metrics.get('stablecoin_latency_ms', 0),
        'committed_txns':         metrics.get('committed_txns', 0),
        'executed_txns':          metrics.get('executed_txns', 0),
        'exec_ratio':             metrics.get('exec_ratio', 0),
        'e2e_tps':                metrics.get('e2e_tps', 0),
        'e2e_bps':                metrics.get('e2e_bps', 0),
        'e2e_latency_ms':         metrics.get('e2e_latency_ms', 0),
        'with_exec_tps':          metrics.get('with_exec_tps', 0),
        'with_exec_bps':          metrics.get('with_exec_bps', 0),
        'with_exec_latency_ms':   metrics.get('with_exec_latency_ms', 0),
        'duration_s':             metrics.get('duration_s', 0),
    }


def write_csv(results, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=FIELDNAMES, extrasaction='ignore')
        w.writeheader()
        w.writerows(results)
    print(f"  -> Saved: {path}")


def print_summary(results, group_keys):
    grouped = defaultdict(list)
    for r in results:
        grouped[tuple(r[k] for k in group_keys)].append(r)
    header = ''.join(f'{k:<20}' for k in group_keys)
    print(f"{header} {'SC.TPS':>10} {'SC.Lat':>10} {'Committed':>10} {'Executed':>10} {'ExecRate':>10}")
    print('-' * (20 * len(group_keys) + 54))
    for key, runs in sorted(grouped.items()):
        avg = lambda field: sum(r[field] for r in runs) / len(runs)
        row = ''.join(f'{str(v):<20}' for v in key)
        print(f"{row} {avg('stablecoin_tps'):>10,.0f} {avg('stablecoin_latency_ms'):>10,.0f}"
              f" {avg('committed_txns'):>10,.0f} {avg('executed_txns'):>10,.0f}"
              f" {avg('exec_ratio'):>10.4f}")


# ── Experiment runners ────────────────────────────────────────────────────────

def run_exp(bench, tag, systems, nodes_list, workers, rates, patterns, runs,
            leap_threads, tokio_threads=4):
    total = len(systems) * len(nodes_list) * len(rates) * len(patterns) * runs
    done, results = 0, []
    # Systems in innermost loop: each config runs MP3+LEAP then Tusk+LeapBase
    # back-to-back for immediate comparison and early Ctrl-C.
    for nodes in nodes_list:
        for pattern in patterns:
            for rate in rates:
                for run_id in range(1, runs + 1):
                    for sys_name, env_base in systems:
                        env = dict(env_base)
                        if pattern != 'Uniform':
                            env['LEAP_PATTERN'] = pattern
                        done += 1
                        print(f"\n[{tag}: {done}/{total}] ", end='')
                        if len(patterns) > 1 and len(rates) > 1:
                            variable = f'{pattern}@{rate // 1000}K'
                        elif len(patterns) > 1:
                            variable = pattern
                        elif len(nodes_list) > 1:
                            variable = nodes
                        else:
                            variable = rate
                        m = run_single(bench, sys_name, nodes, workers, rate,
                                       run_id, env, leap_threads,
                                       tokio_threads=tokio_threads, tag=tag)
                        if m['status'] == 'ok':
                            results.append(make_row(tag, sys_name, variable,
                                                    nodes, workers, rate, run_id, m))
                        time.sleep(3)
    return results


def run_exp_a(bench, leap_threads, tokio_threads):
    print(f"\n{'#'*70}")
    print(f"  Exp A: Contention Scaling")
    print(f"  systems={[s[0] for s in EXP_A_SYSTEMS]}")
    print(f"  patterns={EXP_A_PATTERNS}, rate={EXP_A_RATE}, runs={RUNS}")
    print(f"{'#'*70}")
    return run_exp(bench, 'Exp-A', EXP_A_SYSTEMS, [EXP_A_NODES], 1,
                   [EXP_A_RATE], EXP_A_PATTERNS, RUNS, leap_threads, tokio_threads)


def run_exp_b(bench, leap_threads, tokio_threads):
    print(f"\n{'#'*70}")
    print(f"  Exp B: Rate Scaling (Hotspot 90%)")
    print(f"  systems={[s[0] for s in EXP_B_SYSTEMS]}")
    print(f"  rates={EXP_B_RATES}, pattern={EXP_B_PATTERN}, runs={RUNS}")
    print(f"{'#'*70}")
    return run_exp(bench, 'Exp-B', EXP_B_SYSTEMS, [EXP_B_NODES], 1,
                   EXP_B_RATES, [EXP_B_PATTERN], RUNS, leap_threads, tokio_threads)


# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('exps', nargs='*', default=['A', 'B'],
                        help='Experiments to run (A B)')
    parser.add_argument('--threads', type=int, default=16,
                        help='Total vCPUs per server (default: 16). Rayon gets threads-tokio, tokio gets 4.')
    parser.add_argument('--tokio-threads', type=int, default=4,
                        help='Tokio worker threads per process (default: 4). Rest goes to rayon.')
    args = parser.parse_args()

    exps = [e.upper() for e in args.exps]
    tokio_threads = args.tokio_threads
    rayon_threads = max(1, args.threads - tokio_threads)
    leap_threads = rayon_threads

    os.makedirs(OUTPUT_DIR, exist_ok=True)

    # Validate hosts.json before starting.
    try:
        manager = StaticInstanceManager(HOSTS_FILE)
        available = len(manager.hosts(flat=True))
    except Exception as e:
        print(f"ERROR: Cannot load {HOSTS_FILE}: {e}")
        print("       Edit hosts.json with your server IPs and SSH key path.")
        sys.exit(1)

    print(f"Distributed E2E Experiment v2 (Redesigned)")
    print(f"===========================================")
    print(f"Servers in hosts.json: {available}")
    print(f"vCPUs per server: {args.threads} (rayon={rayon_threads}, tokio={tokio_threads})")
    print(f"Duration per run: {DURATION}s, Runs per config: {RUNS}")
    print(f"Key settings: TARGET_TPS=5000, BLOCK_SIZE=10000, ACCOUNTS=10000")
    manager.print_info()

    if available < 4:
        print("ERROR: Need at least 4 servers.")
        sys.exit(1)

    exp_counts = {
        'A': len(EXP_A_SYSTEMS) * len(EXP_A_PATTERNS) * RUNS,
        'B': len(EXP_B_SYSTEMS) * len(EXP_B_RATES) * RUNS,
    }
    total_runs = sum(exp_counts.get(e, 0) for e in exps)
    print(f"Experiments: {', '.join(f'Exp-{e}({exp_counts.get(e, 0)})' for e in exps)}")
    print(f"Total benchmark runs: {total_runs}")
    print(f"Estimated time: ~{total_runs * (DURATION + 15) // 60} minutes\n")

    # Create ONE bench instance (reused for all runs — avoids fd leak).
    # Compile with superset features so binary supports all protocols.
    bench = StaticBench(extra_features='e2e_exec,mp3bft', hosts_file=HOSTS_FILE)

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

    exp_map = {
        'A': lambda: run_exp_a(bench, leap_threads, tokio_threads),
        'B': lambda: run_exp_b(bench, leap_threads, tokio_threads),
    }

    for exp_id in exps:
        if exp_id not in exp_map:
            print(f"WARNING: unknown experiment '{exp_id}', skipping.")
            continue
        r = exp_map[exp_id]()
        all_results.extend(r)
        # Save per-experiment CSV incrementally.
        if r:
            path = os.path.join(OUTPUT_DIR, f'exp3_v2_{exp_id}.csv')
            write_csv(r, path)

    elapsed = time.time() - start
    print(f"\n\nDone in {elapsed/60:.1f} min. Successful: {len(all_results)}/{total_runs} runs.")

    if all_results:
        combined = os.path.join(OUTPUT_DIR, 'exp3_e2e_v2.csv')
        write_csv(all_results, combined)

        summary_keys = {'A': ['system', 'variable'], 'B': ['system', 'rate']}
        for exp_id in exps:
            if exp_id not in summary_keys:
                continue
            tag = f'Exp-{exp_id}'
            sub = [r for r in all_results if r['experiment'] == tag]
            if sub:
                print(f"\n{'='*70}\n  {tag} Summary\n{'='*70}")
                print_summary(sub, summary_keys[exp_id])
    else:
        print("No successful runs. Check SSH access and hosts.json.")


if __name__ == '__main__':
    main()
