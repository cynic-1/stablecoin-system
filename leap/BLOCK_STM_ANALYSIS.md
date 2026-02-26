# Block-STM Source Code Analysis Report

## 1. Overall Architecture

### Entry Point and Call Chain
- **Parallel executor entry**: `ParallelTransactionExecutor::execute_transactions_parallel()` in `parallel-executor/src/executor.rs:240`
- Creates MVHashMap, OutcomeArray, Scheduler, TxnLastInputOutput
- Spawns `num_cpus` threads via Rayon `scope()`
- Main loop: `scheduler.next_task()` → match ExecutionTask/ValidationTask/NoTask/Done

### Core Components
| Component | File | Role |
|-----------|------|------|
| MVHashMap | `mvhashmap/src/lib.rs` | Multi-version key-value store (DashMap<K, BTreeMap<TxnIndex, WriteCell<V>>>) |
| Scheduler | `parallel-executor/src/scheduler.rs` | Lock-free task scheduler with atomic exec/val indices |
| Executor | `parallel-executor/src/executor.rs` | Orchestrates speculative execution + validation |
| Task traits | `parallel-executor/src/task.rs` | Transaction/ExecutorTask/TransactionOutput trait defs |
| TxnLastInputOutput | `parallel-executor/src/txn_last_input_output.rs` | Per-txn read/write set tracking |

### Thread Model
- Rayon thread pool: one thread per CPU core
- All threads share: MVHashMap, Scheduler, TxnLastInputOutput
- Each thread creates its own `ExecutorTask` via `E::init()`
- Work-stealing: threads compete for next task via atomic fetch_add on exec_idx/val_idx

## 2. Key Data Structures

### MVHashMap
```
DashMap<K, BTreeMap<TxnIndex, CachePadded<WriteCell<V>>>>
```
- Outer: DashMap provides per-key concurrent access
- Inner: BTreeMap stores versions ordered by txn_idx
- WriteCell: { flag: AtomicUsize (DONE=0/ESTIMATE=1), incarnation, data: Arc<V> }

### Scheduler State Machine
```
ReadyToExecute(i) → Executing(i) → Executed(i) → Aborting(i) → ReadyToExecute(i+1)
```
- `execution_idx`: AtomicUsize, monotonically advances (can be decreased)
- `validation_idx`: AtomicUsize, monotonically advances (can be decreased)
- `txn_status`: Vec<Mutex<TransactionStatus>> per transaction
- `txn_dependency`: Vec<Mutex<Vec<TxnIndex>>> dependency lists

### Type Aliases
- `TxnIndex = usize` — position in block
- `Incarnation = usize` — retry count
- `Version = (TxnIndex, Incarnation)`

## 3. Core Algorithm Flow

### Speculative Execution (executor.rs:110-199)
1. Pre-check read dependencies from previous incarnation
2. Create MVHashMapView for this txn
3. Call `executor.execute_transaction(&view, txn)`
4. If dependency encountered → return NoTask (scheduler will resume later)
5. Apply writes to MVHashMap, track writes_outside previous write set
6. Delete stale writes from previous incarnation
7. Call `scheduler.finish_execution()` → may return validation task

### Validation (executor.rs:201-238)
1. Load read set from previous execution
2. For each read descriptor, re-read from MVHashMap
3. Check version matches (MVHashMap read) or still from storage
4. If invalid → `scheduler.try_abort()` → mark writes as ESTIMATE → `finish_abort()`

### Scheduling (scheduler.rs:168-186)
```rust
loop {
    if done() { return Done; }
    if val_idx < exec_idx { try validation } else { try execution }
}
```
- Prioritizes validation over execution to reduce abort rate
- Both indices advanced via atomic fetch_add

### Dependency Resolution
- On ESTIMATE read: `try_add_dependency(txn_idx, dep_txn_idx)` → adds to dep list
- On dep completion: `finish_execution()` → `resume()` all dependents → decrease exec_idx

## 4. Transaction Abstraction

### Traits
```rust
trait Transaction: Sync + Send + 'static {
    type Key: PartialOrd + Send + Sync + Clone + Hash + Eq;
    type Value: Send + Sync;
}

trait ExecutorTask: Sync {
    type T: Transaction;
    type Output: TransactionOutput<T = Self::T>;
    type Error: Clone + Send + Sync;
    type Argument: Sync + Copy;
    fn init(args: Self::Argument) -> Self;
    fn execute_transaction(&self, view: &MVHashMapView<K,V>, txn: &Self::T) -> ExecutionStatus<...>;
}

trait TransactionOutput: Send + Sync {
    type T: Transaction;
    fn get_writes(&self) -> Vec<(K, V)>;
    fn skip_output() -> Self;
}
```

### Proptest Transaction Model
- `Transaction<K,V>` enum: Write { actual_writes, skipped_writes, reads } | SkipRest | Abort
- Generated via `TransactionGen<V>` with configurable write_keep_rate, universe_size
- `ExpectedOutput::generate_baseline()` computes sequential reference

## 5. Benchmark Analysis

### Real Benchmark (diem-transaction-benchmarks/src/main.rs)
- Uses P2PTransferGen from language_e2e_tests (full Diem Move VM)
- Parameters: accounts=[2,10,100,1000,10000], blocks=[1000,10000]
- 2 warmups + 10 measured runs, reports sorted TPS + average
- Thread count = num_cpus (not configurable directly; use taskset)

### Proptest Benchmark (parallel-executor/src/proptest_types/bencher.rs)
- Standalone: Transaction<K,V> with configurable read/write sizes + universe
- Criterion-based benchmarking
- Correctness verified against sequential baseline

### Conflict Creation
- Low account count → high conflict (accounts=[2,10])
- High account count → low conflict (accounts=[10000])
- Collision probability ∝ 1/num_accounts² for P2P

## 6. LEAP Improvement Cut-in Points

| LEAP Optimization | Block-STM Location | Modification |
|-------------------|---------------------|--------------|
| Domain-aware scheduling | scheduler.rs `next_task()` (line 168) | Add domain plan; prefer same-domain validation, cross-domain execution |
| Hot-Delta sharding | mvhashmap/src/lib.rs `write()/read()` | Add delta key routing for hot accounts; aggregate on read |
| Backpressure | scheduler.rs `try_execute_next_version()` (line 396) | Check exec_idx - val_idx < W before advancing |
| CADO ordering | executor.rs `execute_transactions_parallel()` (line 240) | Pre-sort txns before execution |
| Stablecoin model | New file: stablecoin.rs | Impl Transaction + ExecutorTask for StablecoinTx |
