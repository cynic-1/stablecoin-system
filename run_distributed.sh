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
#   ./run_distributed.sh logs            # list saved remote logs
#   ./run_distributed.sh logs exp3 grep ExecStats  # search logs

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
    info "Installing on all servers via SSH ..."
    python3 - << 'PYEOF'
import sys, json, subprocess
sys.path.insert(0, '.')

with open('hosts.json') as f:
    data = json.load(f)

hosts  = data['hosts']
key    = data['key_path']
user   = data.get('username', 'ubuntu')
repo   = data['repo']['url']
branch = data['repo']['branch']

cmd = (
    "sudo apt-get update -qq && "
    "sudo apt-get install -y -qq build-essential cmake clang && "
    "curl --proto 'https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && "
    "source $HOME/.cargo/env && "
    "rustup default stable && "
    f"(git clone {repo} || (cd {data['repo']['name']} && git fetch && git checkout {branch} && git pull))"
)

failed = []
for ip in hosts:
    print(f"  [{ip}] installing ...", flush=True)
    r = subprocess.run(
        ['ssh', '-tt', '-i', key, '-o', 'StrictHostKeyChecking=no',
         '-o', 'ConnectTimeout=10', f'{user}@{ip}', cmd],
        capture_output=False
    )
    if r.returncode != 0:
        print(f"  [{ip}] FAILED (exit {r.returncode})")
        failed.append(ip)
    else:
        print(f"  [{ip}] OK")

if failed:
    print(f"\nFailed on: {failed}")
    sys.exit(1)
print("\nAll servers installed successfully.")
PYEOF
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
    logs)
        section "Saved Remote Logs"
        SAVED_LOGS="$BENCHMARK_DIR/saved_logs"
        if [ ! -d "$SAVED_LOGS" ]; then
            warn "No saved logs yet. Run exp2 or exp3 first."
            exit 0
        fi
        FILTER="${1:-}"
        SEARCH="${2:-}"
        if [ -n "$FILTER" ] && [ -n "$SEARCH" ]; then
            # e.g. ./run_distributed.sh logs exp3 "ExecStats"
            info "Searching '$SEARCH' in saved_logs/$FILTER/"
            grep -r "$SEARCH" "$SAVED_LOGS/$FILTER/" 2>/dev/null || warn "No matches."
        elif [ -n "$FILTER" ]; then
            info "Logs in saved_logs/$FILTER/:"
            ls -1d "$SAVED_LOGS/$FILTER"/*/ 2>/dev/null || warn "No logs for $FILTER."
        else
            info "All saved logs:"
            for d in "$SAVED_LOGS"/*/; do
                exp=$(basename "$d")
                count=$(find "$d" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
                echo "  $exp: $count runs"
            done
            echo ""
            echo "Usage:"
            echo "  $0 logs                              # list all"
            echo "  $0 logs exp3                         # list exp3 runs"
            echo "  $0 logs exp3 ExecStats               # grep ExecStats in exp3 logs"
            echo "  $0 logs exp3 'recv_ms=\|run_ms='     # grep timing fields"
        fi
        ;;
    help|--help|-h)
        sed -n '2,21p' "$0" | sed 's/^# //'
        ;;
    *)
        error "Unknown command: $COMMAND. Run '$0 help' for usage."
        ;;
esac
