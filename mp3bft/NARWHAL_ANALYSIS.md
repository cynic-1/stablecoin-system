# Narwhal Codebase Analysis for MP3-BFT++ Integration

## 1. Workspace Structure (8 crates)

| Crate | Purpose | Key Types |
|-------|---------|-----------|
| `primary` | DAG management, header creation, vote aggregation | `Header`, `Certificate`, `Vote`, `Primary` |
| `worker` | Transaction batching, mempool | `Batch`, `Transaction`, `WorkerMessage` |
| `consensus` | DAG linearization (Tusk + MP3-BFT++) | `Consensus`, `MP3Consensus`, `Dag` |
| `crypto` | Ed25519 signatures, hashing | `Digest`, `PublicKey`, `SecretKey`, `Signature` |
| `network` | TCP message transport | `SimpleSender`, `ReliableSender`, `Receiver` |
| `store` | Async RocksDB wrapper | `Store`, `StoreCommand` |
| `config` | Committee/Parameters JSON config | `Committee`, `Parameters`, `KeyPair` |
| `node` | Binary entry point | Orchestrates all components |

## 2. Architecture: Data Plane / Consensus Plane Separation

### Data Plane (Primary + Workers)

```
Client → Worker.BatchMaker (seals at batch_size or max_batch_delay)
  → Worker sends WorkerPrimaryMessage::OurBatch(Digest, WorkerId) to Primary
  → Primary.Proposer creates Header { author, round, payload: BTreeMap<Digest, WorkerId>, parents }
  → Primary broadcasts Header to peers, collects Vote signatures
  → Primary.Aggregator forms Certificate = Header + quorum signatures
```

Quorum threshold: `2 * total_stake / 3 + 1`.

### Consensus Plane

```
Primary sends Certificate → Consensus via tx_consensus channel
  → Tusk or MP3-BFT++ applies ordering rule on DAG
  → Ordered certificates sent to application via tx_output channel
  → Feedback to Primary for GC via tx_primary channel
```

DAG representation: `HashMap<Round, HashMap<PublicKey, (Digest, Certificate)>>` with per-authority `last_committed` state.

## 3. Primitives Reused by MP3-BFT++

### Crypto (`crypto/src/lib.rs`)
- Ed25519 via `ed25519-dalek`: `PublicKey([u8;32])`, `SecretKey([u8;64])`, `Signature`
- `Digest([u8;32])` with `Sha512`-based hashing
- `SignatureService` for async batch signing
- Batch signature verification

### Network (`network/src/lib.rs`)
- `ReliableSender`: TCP with retry and cancellation (primary-to-primary)
- `SimpleSender`: fire-and-forget (worker-to-primary)
- `Receiver`: async TCP listener with `MessageHandler` trait

### Store (`store/src/lib.rs`)
- Async RocksDB via tokio channel commands: `Write`, `Read`, `NotifyRead`
- Single instance per node, shared across primary/worker/consensus

### Config (`config/src/lib.rs`)
- `Committee`: `BTreeMap<PublicKey, Authority>` with stake-weighted operations
- `Parameters`: header_size, max_header_delay, gc_depth, batch_size, max_batch_delay

## 4. MP3-BFT++ Integration Points

**Location**: `consensus/src/mp3bft.rs` (feature-gated with `#[cfg(feature = "mp3bft")]`)

**Interface**: Drop-in replacement for Tusk — identical channel protocol:
```rust
MP3Consensus::spawn(
    committee: Committee,      // same
    gc_depth: u64,             // same
    k_slots: usize,            // NEW: parallel leaders per round
    rx_primary: Receiver<Certificate>,  // same
    tx_primary: Sender<Certificate>,    // same
    tx_output: Sender<Certificate>,     // same
)
```

**Switching mechanism** (`node/src/main.rs`):
- `#[cfg(not(feature = "mp3bft"))]` → `Consensus::spawn(...)` (Tusk)
- `#[cfg(feature = "mp3bft")]` → `MP3Consensus::spawn(...)` with `MP3BFT_K_SLOTS` env var

**What MP3-BFT++ changes vs Tusk**:
1. Leader election: single leader → k parallel slot leaders per round
2. Commit rule: Tusk's 2-chain → 3-chain with SlotQC/MacroQC certificates
3. All other Narwhal infrastructure (data plane, crypto, network, store) is unchanged

## 5. Key File Index

| Component | Path |
|-----------|------|
| Primary core | `narwhal/primary/src/core.rs` |
| Primary proposer | `narwhal/primary/src/proposer.rs` |
| Worker batch maker | `narwhal/worker/src/batch_maker.rs` |
| Tusk consensus | `narwhal/consensus/src/lib.rs` |
| MP3-BFT++ consensus | `narwhal/consensus/src/mp3bft.rs` |
| E2E LEAP integration | `narwhal/node/src/main.rs` (analyze function) |
| Benchmark runner | `narwhal/benchmark/run_comparison.py` |

## 6. Design Decision: Why Reuse Narwhal

MP3-BFT++ benefits from Narwhal's separation of data availability (workers + batching) from consensus ordering. This means:
- Throughput is bounded by the data plane (batch ingestion rate), not consensus
- MP3-BFT++ only changes the ordering logic, keeping the proven data plane intact
- Performance comparison with Tusk is apples-to-apples: same data plane, different commit rules
- k-slot parallelism in MP3-BFT++ amortizes ordering latency without affecting data throughput
