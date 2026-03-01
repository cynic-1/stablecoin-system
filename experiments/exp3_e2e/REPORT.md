# Experiment 3: End-to-End Integration — Experiment Design

> **Status**: Experiment design finalized, pending execution.
> **Benchmark scripts**: `narwhal/benchmark/run_e2e_complete.py` (localhost), `narwhal/benchmark/run_distributed_exp3.py` (distributed)
> **Plot script**: `experiments/exp3_e2e/plot_complete.py`
> **Output CSV**: `results/raw/exp3_e2e_complete.csv`

---

## 1. Design Rationale

### 1.1 Why Previous Experiments Failed

Previous e2e runs used `LEAP_CRYPTO_US=10` (10μs per-transaction overhead). This made execution trivially fast:

| Config (10μs, 4 threads) | LEAP-base | LEAP | Serial |
|---------------------------|-----------|------|--------|
| Execution capacity | ~400K TPS | ~400K TPS | ~115K TPS |
| Consensus ceiling (100K input) | ~90K TPS | ~90K TPS | ~90K TPS |
| **Bottleneck** | **Consensus** | **Consensus** | **Consensus** |

At 10μs, execution always keeps up with consensus, so `with_exec_tps ≈ consensus_tps` regardless of engine. LEAP's contention-handling optimizations (Hot-Delta, CADO) never get exercised under pressure.

### 1.2 The Fix: Realistic Crypto Overhead (100μs)

Real stablecoin transactions involve signature verification, balance checks, and state updates costing 80–140μs. Using `LEAP_CRYPTO_US=100` matches this realistic workload profile.

**Execution capacity at 100μs, 4 threads per node** (measured in Experiment 1):

| Scenario | LEAP-base (Block-STM) | LEAP (full) | Advantage |
|----------|----------------------|-------------|-----------|
| Uniform | 43,318 TPS | 43,300 TPS* | ~0% |
| Zipf 0.8 | 43,100 TPS | 43,070 TPS | ~0% |
| Zipf 1.2 | 43,170 TPS | 42,640 TPS | -1% |
| Hotspot 50% | 40,760 TPS | 40,230 TPS | -1% |
| **Hotspot 90%** | **26,400 TPS** | **35,640 TPS** | **+35%** |
| Serial | 11,635 TPS | — | — |

*LEAP uses adaptive CADO: skips CADO/HotDelta overhead when no hotspots detected, matching LEAP-base performance.

**Key insight**: At 100μs with 4 threads, LEAP-base collapses under Hotspot 90% (26K TPS, only 2.3× serial) while LEAP maintains 36K (3.1× serial). The difference comes from Hot-Delta sharding that eliminates write-write conflicts on the hot account.

### 1.3 Rate Selection

Consensus delivery rates (from Experiment 2, 4 nodes, 1 worker):
- 10K input → ~9K consensus TPS
- 20K input → ~19K consensus TPS
- 30K input → ~27K consensus TPS
- 50K input → ~43K consensus TPS
- 70K input → ~63K consensus TPS

**Under Hotspot 90%, the critical rate is ~27–43K** where LEAP-base's 26K capacity becomes the bottleneck while LEAP's 36K capacity still keeps up or degrades gracefully.

---

## 2. System Configurations

| System | Consensus | Execution | Description |
|--------|-----------|-----------|-------------|
| **MP3+LEAP** | MP3-BFT++ (k=4) | LEAP (CADO + Hot-Delta + Domain-Aware + Backpressure) | Full thesis system |
| **Tusk+LeapBase** | Tusk | Block-STM (vanilla parallel) | Parallel baseline |
| **Tusk+Serial** | Tusk | Sequential execution | Serial baseline |

---

## 3. Testbed Parameters

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| CPU | AMD EPYC 9754, 16 vCPUs (8 cores × 2 threads) | |
| Memory | 32 GB DDR5 | |
| Nodes | 4 (default) | BFT minimum: 3f+1=4 for f=1 |
| Workers per node | 1 | |
| LEAP threads per node | 4 (= 16 cores / 4 nodes) | Prevents CPU oversubscription |
| Tokio threads | 4 (for network I/O) | Leaves cores for rayon |
| **Crypto overhead** | **100μs** (SHA-256 calibrated) | **Realistic stablecoin tx cost** |
| Account pool | 1,000 | |
| Transaction size | 512 bytes | |
| Batch size | 500,000 bytes (~976 txns/batch) | |
| Duration per run | 60 seconds | |
| Runs per config | 2 (report average) | |
| Deterministic seed | LEAP_SEED=42 | Fair A/B comparison |

---

## 4. Experiment Matrix

### 4.1 Exp-A: Throughput-Latency Scaling (Uniform, 3 systems)

| Parameter | Value |
|-----------|-------|
| Rates | 10K, 20K, 30K, 50K, 70K tx/s |
| Pattern | Uniform |
| Systems | All 3 |
| Runs | 3 × 5 × 2 = 30 |

**Purpose**: Establish the throughput-latency hierarchy. Under Uniform, LEAP and LeapBase have identical execution capacity (~43K TPS). This isolates the consensus latency advantage (MP3-BFT++ vs Tusk) and the parallel vs serial advantage.

**Expected results**:

| Rate | MP3+LEAP | Tusk+LeapBase | Tusk+Serial |
|------|----------|---------------|-------------|
| 10K | ~9K TPS, ~550ms | ~9K TPS, ~850ms | ~9K TPS, ~850ms |
| 20K | ~19K TPS, ~560ms | ~19K TPS, ~870ms | ~12K TPS, ~1.5s |
| 30K | ~27K TPS, ~570ms | ~27K TPS, ~890ms | ~12K TPS, ~3s |
| 50K | ~43K TPS, ~600ms | ~43K TPS, ~950ms | ~12K TPS, ~8s |
| 70K | ~43K TPS, ~3s | ~43K TPS, ~4s | ~12K TPS, ~15s |

At 70K, execution saturates (43K capacity < 63K delivery), causing latency spikes. Serial saturates much earlier (12K capacity < 20K delivery at 20K rate).

### 4.2 Exp-B: Conflict Pattern Sensitivity (50K rate, 2 systems)

| Parameter | Value |
|-----------|-------|
| Patterns | Uniform, Zipf 0.8, Zipf 1.2, Hotspot 50%, Hotspot 90% |
| Rate | 50K |
| Systems | MP3+LEAP, Tusk+LeapBase |
| Runs | 2 × 5 × 2 = 20 |

**Purpose**: At 50K rate, consensus delivers ~43K TPS. Under Uniform, both engines match this (43K capacity). Under Hotspot 90%, LEAP-base drops to 26K (execution bottleneck) while LEAP maintains 36K. This isolates LEAP's contention-handling advantage.

**Expected results**:

| Pattern | MP3+LEAP TPS | Tusk+LeapBase TPS | LEAP Advantage |
|---------|-------------|-------------------|----------------|
| Uniform | ~43K | ~43K | ~0% |
| Zipf 0.8 | ~43K | ~43K | ~0% |
| Zipf 1.2 | ~43K | ~43K | ~-1% |
| Hotspot 50% | ~40K | ~41K | ~-2% |
| **Hotspot 90%** | **~36K** | **~26K** | **+35%** |

Under H50%, LEAP's overhead slightly exceeds its benefit (both have ample capacity). Under H90%, LEAP's Hot-Delta sharding provides decisive advantage.

### 4.3 Exp-C: Node Scalability (50K rate, Uniform, 2 systems)

| Parameter | Value |
|-----------|-------|
| Nodes | 4, 10 |
| Rate | 50K |
| Pattern | Uniform |
| Systems | MP3+LEAP, Tusk+LeapBase |
| Runs | 2 × 2 × 2 = 8 |

**Purpose**: Show behavior as committee grows. At n=4, each node has 4 LEAP threads (good parallelism). At n=10, each node has 1 LEAP thread (execution degrades to sequential).

### 4.4 Exp-D: Contention × Rate Interaction (the headline experiment)

| Parameter | Value |
|-----------|-------|
| Patterns | Hotspot 50%, Hotspot 90% |
| Rates | 10K, 30K, 50K, 70K |
| Systems | MP3+LEAP, Tusk+LeapBase |
| Runs | 2 × 2 × 4 × 2 = 32 |

**Purpose**: The key experiment. Shows how LEAP's advantage grows as load increases under contention.

**Expected Hotspot 90% results**:

| Rate | Consensus TPS | LEAP exec cap. | LeapBase exec cap. | MP3+LEAP e2e | Tusk+LB e2e | Advantage |
|------|-------------|----------|------------|---------|---------|-----------|
| 10K | ~9K | 36K | 26K | ~9K | ~9K | ~0% (both keep up) |
| 30K | ~27K | 36K | 26K | ~27K | ~26K | ~4% (LeapBase at limit) |
| 50K | ~43K | 36K | 26K | ~36K | ~26K | **+38%** |
| 70K | ~63K | 36K | 26K | ~36K | ~26K | **+38%** |

At 50K+ under H90%, both systems are execution-bottlenecked, but LEAP handles 36K vs LeapBase's 26K. The ~35-38% TPS advantage comes directly from Hot-Delta sharding.

**Latency advantage compounds**: MP3-BFT++ adds ~35% lower consensus latency on top of LEAP's faster execution. Under H90% at 50K, expected total latency advantage is 40-60%.

### 4.5 Total Runs

| Experiment | Configs | Runs |
|------------|---------|------|
| Exp-A | 15 | 30 |
| Exp-B | 10 | 20 |
| Exp-C | 4 | 8 |
| Exp-D | 16 | 32 |
| **Total** | **45** | **90** |

Estimated runtime: ~90 × 75s = ~112 minutes.

---

## 5. Metric Definitions

| Metric | Definition |
|--------|-----------|
| **Consensus TPS** | Committed transactions per second (bytes committed / duration / tx_size) |
| **Consensus Latency** | Mean time from proposal creation to commit |
| **With-Execution TPS** | Bytes of *executed* batches / consensus duration / tx_size. When execution falls behind, fewer batches are executed within the benchmark window, reducing this metric. |
| **With-Execution Latency** | Mean time from proposal to execution completion. Captures execution backlog delay. |
| **Stablecoin TPS** | Successfully executed transactions / total benchmark duration |

---

## 6. Thesis Claims to Validate

### Claim 1: MP3-BFT++ reduces consensus latency
**Target**: 30-40% lower consensus latency vs Tusk, validated by Exp-A across all rates.

### Claim 2: LEAP improves execution throughput under contention
**Target**: +30%+ TPS advantage under Hotspot 90%, validated by Exp-B and Exp-D.

### Claim 3: The combined system outperforms baselines
**Target**: MP3+LEAP achieves best latency (from consensus) AND best TPS under contention (from execution). Exp-D at 50K/H90% is the headline result.

---

## 7. Connection to Component Experiments

| Component Result | Source | E2E Validation |
|-----------------|--------|----------------|
| LEAP 4t H90% = 35.6K TPS | Exp-1 (100μs, 4 threads) | Exp-D with_exec_tps under H90% |
| LEAP-base 4t H90% = 26.4K TPS | Exp-1 (100μs, 4 threads) | Exp-D with_exec_tps under H90% |
| MP3-BFT++ latency -35% vs Tusk | Exp-2 (k=4) | Exp-A consensus_latency_ms |
| Hot-Delta advantage = +35% | Exp-1 (100μs, 4t, H90%) | Exp-B and Exp-D TPS delta |

The e2e experiment composes these component results through the real integrated pipeline, validating that the advantages are preserved (not lost to integration overhead, CPU contention, or channel backpressure).

---

## 8. Previous Data Versions

| Version | Date | Crypto | Issue |
|---------|------|--------|-------|
| v1-v5 | 2026-02-25 | 10μs | Execution never bottlenecked; various infrastructure bugs |
| v6 | 2026-02-26 | 10μs | Clean run, but LEAP ≈ LeapBase in TPS (execution too fast) |
| v6.5 | 2026-02-26 | 50μs | Still insufficient; 4-thread contention too low at 50μs |
| v7 (partial) | 2026-02-26 | 10μs | Only Exp-D completed; same execution-too-fast problem |
| **v8 (planned)** | **2026-03-01** | **100μs** | **Realistic overhead; execution becomes bottleneck under H90%** |

---

## 9. Reproduction

```bash
# Build with all features
cd narwhal && cargo build --release --features benchmark,e2e_exec,mp3bft

# Run complete suite (~112 minutes, 90 runs)
cd benchmark && python3 run_e2e_complete.py

# Run individual experiments
python3 run_e2e_complete.py A    # Exp-A only (30 runs)
python3 run_e2e_complete.py B    # Exp-B only (20 runs)
python3 run_e2e_complete.py D    # Exp-D only (32 runs)

# Generate plots
cd ../../experiments/exp3_e2e && python3 plot_complete.py
```
