#!/bin/bash
# Distributed Deployment Test Runner
# Runs Exp1 (locally), Exp2 and Exp3 on real remote servers via SSH.
#
# Prerequisites:
#   1. Fill in narwhal/benchmark/hosts.json with server IPs and SSH key
#   2. Run './run_distributed.sh install' once to set up all servers
#
# Usage:
#   ./run_distributed.sh install         # set up servers (run once)
#   ./run_distributed.sh check           # verify SSH + hosts.json
#   ./run_distributed.sh exp1            # LEAP benchmark (always local)
#   ./run_distributed.sh exp2            # consensus benchmark (distributed)
#   ./run_distributed.sh exp3            # E2E pipeline benchmark (distributed)
#   ./run_distributed.sh exp2 A          # only Exp2-A (rate scaling)
#   ./run_distributed.sh exp3 A B        # only Exp3-A and Exp3-B
#   ./run_distributed.sh exp3 --threads 32  # E2E with 32 threads/node
#   ./run_distributed.sh all             # run exp1 + exp2 + exp3

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCHMARK_DIR="$SCRIPT_DIR/narwhal/benchmark"
HOSTS_FILE="$BENCHMARK_DIR/hosts.json"

# ── Colors ────────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
info()    { echo -e "${GREEN}[INFO]${NC} $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }
section() { echo ""; echo -e "${GREEN}═══════════════════════════════════════════${NC}"; \
            echo -e "${GREEN}  $*${NC}"; \
            echo -e "${GREEN}═══════════════════════════════════════════${NC}"; }

# ── Helpers ────────────────────────────────────────────────────────────────────
check_hosts_json() {
    if [ ! -f "$HOSTS_FILE" ]; then
        error "hosts.json not found at $HOSTS_FILE\n       Create it using the template in the file."
    fi
    # Check for placeholder values
    if grep -q "SERVER_1_IP\|YOUR_USERNAME\|YOUR_REPO" "$HOSTS_FILE" 2>/dev/null; then
        error "hosts.json still has placeholder values. Edit it with real IPs and paths."
    fi
    info "hosts.json found."
}

check_ssh() {
    section "Checking SSH connectivity"
    cd "$BENCHMARK_DIR"
    python3 -c "
import sys, json
sys.path.insert(0, '.')
with open('hosts.json') as f:
    data = json.load(f)
hosts = data['hosts']
key   = data['key_path']
user  = data.get('username', 'ubuntu')
import subprocess, shutil

if not shutil.which('ssh'):
    print('ERROR: ssh not found in PATH')
    sys.exit(1)

ok, fail = [], []
for ip in hosts:
    r = subprocess.run(
        ['ssh', '-i', key, '-o', 'StrictHostKeyChecking=no',
         '-o', 'ConnectTimeout=5', '-o', 'BatchMode=yes',
         f'{user}@{ip}', 'echo OK'],
        capture_output=True, text=True, timeout=10
    )
    if r.returncode == 0:
        ok.append(ip)
        print(f'  OK   {ip}')
    else:
        fail.append(ip)
        print(f'  FAIL {ip}  ({r.stderr.strip()!r})')

print(f'\nReachable: {len(ok)}/{len(hosts)}')
if fail:
    print(f'Failed:    {fail}')
    sys.exit(1)
"
    info "All servers reachable."
}

install_servers() {
    section "Installing dependencies on all servers"
    check_hosts_json
    check_ssh
    cd "$BENCHMARK_DIR"
    info "Running fab static_install ..."
    fab static_install
    info "Installation complete."
}

run_exp1() {
    section "Exp1: LEAP Execution Engine (local — no distributed needed)"
    cd "$SCRIPT_DIR/experiments/exp1_execution"
    bash run_all.sh
}

run_exp2() {
    section "Exp2: Consensus Benchmark (distributed)"
    check_hosts_json
    local EXTRA_ARGS=("$@")

    # Build narwhal with both features so remote servers can run either protocol.
    info "Building narwhal locally (benchmark + mp3bft) ..."
    cd "$SCRIPT_DIR/narwhal"
    cargo build --quiet --release --features benchmark,mp3bft 2>&1

    info "Running distributed exp2 benchmarks ..."
    cd "$BENCHMARK_DIR"
    python3 run_distributed_exp2.py "${EXTRA_ARGS[@]}"

    info "Exp2 complete. Generating plots ..."
    cd "$SCRIPT_DIR/experiments/exp2_consensus"
    python3 plot_narwhal.py 2>&1 || warn "Plot failed (install matplotlib: pip3 install matplotlib)"
}

run_exp3() {
    section "Exp3: E2E Pipeline Benchmark (distributed)"
    check_hosts_json
    local EXTRA_ARGS=("$@")

    # Build narwhal with all E2E features.
    info "Building narwhal locally (benchmark + e2e_exec + mp3bft) ..."
    cd "$SCRIPT_DIR/narwhal"
    cargo build --quiet --release --features benchmark,e2e_exec,mp3bft 2>&1

    info "Running distributed exp3 benchmarks ..."
    cd "$BENCHMARK_DIR"
    python3 run_distributed_exp3.py "${EXTRA_ARGS[@]}"

    info "Exp3 complete. Generating plots ..."
    cd "$SCRIPT_DIR/experiments/exp3_e2e"
    python3 plot_complete.py 2>&1 || warn "Plot failed (install matplotlib: pip3 install matplotlib)"
}

# ── Dispatch ───────────────────────────────────────────────────────────────────
COMMAND="${1:-help}"
shift || true  # remaining args forwarded to sub-commands

case "$COMMAND" in
    install)
        install_servers
        ;;
    check)
        check_hosts_json
        check_ssh
        ;;
    exp1)
        run_exp1
        ;;
    exp2)
        run_exp2 "$@"
        ;;
    exp3)
        run_exp3 "$@"
        ;;
    all)
        run_exp1
        run_exp2
        run_exp3
        section "All experiments complete"
        echo "Results:"
        echo "  Exp1: experiments/exp1_execution/results/raw/"
        echo "  Exp2: experiments/exp2_consensus/results/raw/exp2_distributed_*.csv"
        echo "  Exp3: experiments/exp3_e2e/results/raw/exp3_distributed_*.csv"
        ;;
    help|--help|-h)
        sed -n '2,20p' "$0" | sed 's/^# //'
        ;;
    *)
        error "Unknown command: $COMMAND. Run '$0 help' for usage."
        ;;
esac
