# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Purpose

Master's thesis: "Design and Implementation of a High-Performance Parallel Blockchain System for Stablecoins." The system has two novel components:

1. **LEAP** — Parallel execution engine improving on Block-STM with domain-aware scheduling, Hot-Delta hot-spot sharding, and adaptive backpressure.
2. **MP3-BFT++** — Execution-aware multi-proposer BFT consensus with data/control plane separation and dual-layer certificates (SlotQC/MacroQC).

These are coupled via **CADO** (Conflict-Aware Deterministic Ordering). The full implementation plan, checkpoints, and experiment specifications are in `prd.md`.

## Repository Structure

```
narwhal/          # [Reference] Narwhal-Tusk DAG-based mempool + BFT consensus (Rust + Python)
Block-STM/        # [Reference] Block-STM parallel execution on Diem fork (137-crate Cargo workspace)
leap/             # [New] LEAP execution engine (fork of Block-STM core)
mp3bft/           # [New] MP3-BFT++ consensus protocol (reuses narwhal crypto/network/store)
experiments/      # [New] Three experiment suites (execution, consensus, end-to-end)
prd.md            # Implementation guide with step-by-step phases, checkpoints, and experiment specs
PROGRESS.md       # Progress tracking (must be updated after each step per prd.md §1.1)
```

---

## Narwhal

### Commands

All commands run from `narwhal/` directory. Rust 1.51+, edition 2018.

```bash
cargo build --all-targets
cargo build --release
cargo test --all-features --verbose
cargo fmt -- --check
cargo clippy --all-features --all-targets

# Single test by name
cargo test test_name_pattern
cargo test -p primary test_name_pattern
cargo test test_name -- --nocapture    # with stdout

# Local benchmark (requires clang + tmux)
cd benchmark && pip install -r requirements.txt && fab local
```

### Running a Node

```bash
cargo run --release -- generate_keys --filename=keys.json
cargo run --release -- run --keys=keys.json --committee=committee.json --store=/tmp/db primary
cargo run --release -- run --keys=keys.json --committee=committee.json --store=/tmp/db worker --id=0
```

### Architecture

8-crate workspace: `primary`, `worker`, `consensus`, `node`, `store`, `crypto`, `network`, `config`.

The protocol separates **mempool** (Narwhal) from **consensus** (Tusk):

- **Primary** (`primary/src/`): DAG management. `core.rs` (voting/DAG), `proposer.rs` (header creation), `synchronizer.rs` (peer sync), `aggregators.rs` (vote/cert aggregation), `garbage_collector.rs`.
- **Worker** (`worker/src/`): Transaction batches. `batch_maker.rs`, `quorum_waiter.rs`, `processor.rs`.
- **Consensus** (`consensus/src/`): Tusk DAG linearization. DAG is `HashMap<Round, HashMap<PublicKey, (Digest, Certificate)>>` with per-authority `last_committed` state and GC at `gc_depth`.
- **Store** (`store/`): Async RocksDB wrapper with `Write`/`Read`/`NotifyRead` commands over tokio channels.
- **Crypto** (`crypto/`): Ed25519 via `ed25519-dalek`. Types: `Digest([u8;32])`, `PublicKey([u8;32])`, `SecretKey([u8;64])`, `Signature`. Supports batch verification.
- **Config** (`config/`): `Committee` (BTreeMap<PublicKey, Authority>), `Parameters`, `KeyPair`. JSON import/export.

Key types: `Round = u64`. `Header` contains `(author, round, payload: BTreeMap<Digest, WorkerId>, parents: BTreeSet<Digest>)`. `Certificate` wraps `Header` + quorum of `(PublicKey, Signature)` votes. Quorum threshold: `2 * total_stake / 3 + 1`.

Network messages: `PrimaryMessage::{Header, Vote, Certificate, CertificatesRequest}`. Worker-primary communication via `PrimaryWorkerMessage` and `WorkerPrimaryMessage`.

Benchmark parameters (nodes, workers, rate, tx_size, faults, duration, header_size, batch_size, gc_depth, etc.) configured in `benchmark/fabfile.py`.

---

## Block-STM

### Setup and Commands

137-crate Diem workspace. Rust toolchain: **1.56.1** (pinned in `rust-toolchain`). Edition 2018. Ubuntu 20.04 recommended.

```bash
# One-time setup (installs Rust, CMake, Clang, LLVM, etc.)
./scripts/dev_setup.sh

# Build
cargo build --release

# Block-STM parallel benchmark
cd diem-move/diem-transaction-benchmarks/src && cargo run --release main

# Sequential baseline
cd diem-move/diem-transaction-benchmarks/benches && cargo bench peer_to_peer

# Limit thread count (important for experiments)
taskset -c 0-7 cargo run --release main    # 8 threads

# Single crate tests
cargo test --package mvhashmap
cargo test --package diem-parallel-executor
cargo test --package mvhashmap -- test_name_pattern

# Diem workspace tools (if cargo-x installed)
cargo xtest        # full test suite
cargo xfmt         # format
cargo xclippy --all-targets
```

Benchmark parameters in `diem-move/diem-transaction-benchmarks/src/main.rs`:
```rust
let acts = [2, 10, 100, 1000, 10000];  // account counts
let txns = [1000, 10000];               // block sizes
let num_warmups = 2;
let num_runs = 10;
```

Sequential baseline parameters in `diem-move/diem-transaction-benchmarks/src/transactions.rs` (`DEFAULT_NUM_ACCOUNTS`, `DEFAULT_NUM_TRANSACTIONS`).

Alternative branches: `block_stm` (Diem P2P), `aptos` (Aptos P2P, 160k tps), `bohm`, `litm`.

### Architecture

The novel Block-STM code lives in two crates; everything else is stock Diem substrate:

- **`diem-move/mvhashmap/`** — Multi-version data store. `DashMap<K, BTreeMap<TxnIndex, CachePadded<WriteCell<V>>>>`. Each `WriteCell` has an atomic `flag` (DONE/ESTIMATE), `incarnation`, and `Arc<V>` data. Key operations: `write()`, `read()` (returns version or dependency), `mark_estimate()`, `delete()`.
- **`diem-move/parallel-executor/`** — Execution engine:
  - `executor.rs`: Rayon thread pool orchestrating speculative execution.
  - `scheduler.rs`: Lock-free scheduler with atomic `execution_idx`/`validation_idx`. Tasks: `ExecutionTask(Version)`, `ValidationTask(Version)`, `NoTask`, `Done`. Transaction states: `ReadyToExecute` → `Executing` → `Executed` → (optionally `Aborting` → back to `ReadyToExecute`).
  - `task.rs`: Core traits — `Transaction` (associated `Key`/`Value` types), `ExecutorTask` (implements `execute_transaction`), `TransactionOutput` (provides `get_writes()`).
  - `txn_last_input_output.rs`: Per-transaction read/write set tracking.
- **`diem-move/diem-transaction-benchmarks/`** — Benchmark harness.

Execution flow: speculative parallel execution → conflict detection via MVHashMap → conflicting transactions re-executed with incremented incarnation → committed when validation passes.

Type aliases: `TxnIndex = usize`, `Incarnation = usize`, `Version = (TxnIndex, Incarnation)`.

---

## Development Phases (from prd.md)

### Phase 1: LEAP Execution Engine
Fork Block-STM's `mvhashmap` + `parallel-executor` into `leap/` crate. Steps 1.1–1.9: analyze Block-STM → collect baselines → fork → add stablecoin tx model → CADO ordering → domain-aware scheduling → Hot-Delta sharding → adaptive backpressure → full integration.

**Critical checkpoints**: CP-1 (Block-STM multi-thread > single-thread), CP-2 (LEAP fork ≈ Block-STM ±5%), CP-3 (each optimization ≥ baseline), CP-4 (LEAP ≥ Block-STM all scenarios, monotonic scaling to 16 threads).

### Phase 2: MP3-BFT++ Consensus
Implement in `mp3bft/` reusing narwhal's crypto/network/store. Steps 2.1–2.10: Narwhal baselines → code analysis → types → data plane (reuse Narwhal workers) → anti-duplication → slot-level certification → macro-block finality → view change → CADO → benchmark integration.

**Critical checkpoint**: CP-6 (MP3-BFT++ TPS ≥ Tusk, scales with k).

### Phase 3: End-to-End Integration
MP3-BFT++ → CADO → LEAP pipeline. Experiments: throughput-latency curves, conflict patterns, node scalability.

### Hard Constraints
- Execution TPS must be monotonically non-decreasing with thread count (up to 16 threads).
- Consensus TPS must not be below Narwhal-Tusk baseline.
- Do not proceed to next phase until current phase checkpoints pass.
- Update `PROGRESS.md` after completing each step.
