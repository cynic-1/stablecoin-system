use crate::{
    executor::MVHashMapView,
    hot_delta::HotDeltaManager,
    task::{ExecutionStatus, ExecutorTask, Transaction, TransactionOutput},
};
use rand::Rng;
use rand::SeedableRng;
use rand_distr::{Distribution, Uniform, Zipf};
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Simulated crypto / VM overhead
// ---------------------------------------------------------------------------
//
// Real stablecoin transactions involve signature verification (~50-80μs for
// Ed25519), state Merkle proof verification (~10-20μs), and VM execution
// (~20-40μs). Total per-tx overhead: ~80-140μs, yielding ~7K-12K serial TPS.
//
// We simulate this with iterated SHA-256 hashing. On typical hardware,
// one SHA-256(32 bytes) ≈ 200-300ns, so ~400 iterations ≈ ~100μs.
//
// This constant is adjustable; set to 0 to disable overhead (unit-test mode).

/// Number of SHA-256 iterations per transaction to simulate crypto/VM work.
/// ~1600 iterations ≈ ~100μs on this hardware → ~10K serial TPS.
/// Calibrated: 1600 iters ≈ 99.5μs/tx on this machine (SHA-256 ~62ns/iter).
pub const CRYPTO_WORK_ITERS: u32 = 1600;

/// Simulate the cryptographic and VM overhead of a real stablecoin transaction.
/// Uses iterated SHA-256 to produce a deterministic, non-optimizable workload.
/// Returns the final hash (prevents compiler from optimizing away the work).
/// `iters` controls the number of SHA-256 iterations (0 = no overhead).
#[inline(never)]
pub fn simulate_tx_crypto_work(tx_hash: u64, iters: u32) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[..8].copy_from_slice(&tx_hash.to_le_bytes());
    for _ in 0..iters {
        let result = Sha256::digest(&buf);
        buf.copy_from_slice(&result);
    }
    buf
}

// ---------------------------------------------------------------------------
// State key representation
// ---------------------------------------------------------------------------

/// Keys in the stablecoin state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateKey {
    Balance(u64),
    Nonce(u64),
    TotalSupply,
    Frozen(u64),
    /// Hot-Delta shard: delta(account, shard_id)
    Delta(u64, u64),
}

/// Values are u64 encoded.
pub type StateValue = u64;

// ---------------------------------------------------------------------------
// Transaction types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum StablecoinTxType {
    Transfer {
        sender: u64,
        receiver: u64,
        amount: u64,
    },
    Mint {
        to: u64,
        amount: u64,
    },
    Burn {
        from: u64,
        amount: u64,
    },
    /// Initialize an account balance (no shared state touched — conflict-free).
    /// Used for funding accounts before benchmark transactions.
    InitBalance {
        account: u64,
        amount: u64,
    },
}

#[derive(Debug, Clone)]
pub struct StablecoinTx {
    pub tx_type: StablecoinTxType,
    pub nonce: u64,
    pub tx_hash: u64,
}

impl StablecoinTx {
    /// Derive the conflict domain for this transaction.
    pub fn conflict_domain(&self) -> u64 {
        match &self.tx_type {
            StablecoinTxType::Transfer { receiver, .. } => *receiver,
            StablecoinTxType::Mint { to, .. } => *to,
            StablecoinTxType::Burn { from, .. } => *from,
            StablecoinTxType::InitBalance { account, .. } => *account,
        }
    }

    /// Sender account (for ordering).
    pub fn sender(&self) -> u64 {
        match &self.tx_type {
            StablecoinTxType::Transfer { sender, .. } => *sender,
            StablecoinTxType::Mint { .. } => 0, // minter = account 0
            StablecoinTxType::Burn { .. } => 0, // burner = account 0
            StablecoinTxType::InitBalance { account, .. } => *account,
        }
    }
}

impl Transaction for StablecoinTx {
    type Key = StateKey;
    type Value = StateValue;
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Arguments for initializing a per-thread StablecoinExecutor.
#[derive(Clone)]
pub struct StablecoinExecArgs {
    pub crypto_work_iters: u32,
    pub hot_delta: Option<Arc<HotDeltaManager>>,
    /// Default balance for accounts not yet written in this block.
    /// Simulates pre-existing on-chain state (accounts are already funded).
    pub funded_balance: u64,
}

impl From<u32> for StablecoinExecArgs {
    fn from(iters: u32) -> Self {
        Self {
            crypto_work_iters: iters,
            hot_delta: None,
            funded_balance: 0,
        }
    }
}

pub struct StablecoinExecutor {
    crypto_work_iters: u32,
    hot_delta: Option<Arc<HotDeltaManager>>,
    funded_balance: u64,
}

/// Output holding writes.
#[derive(Debug, Clone)]
pub struct StablecoinOutput {
    pub writes: Vec<(StateKey, StateValue)>,
}

/// Execution outcome counts for benchmark metrics.
#[derive(Debug, Clone, Default)]
pub struct ExecCounts {
    pub total: usize,
    pub successful: usize,
}

/// Count successful vs failed outcomes from parallel execution outputs.
/// A transaction is successful if it produced non-empty writes.
pub fn count_parallel_outcomes(outputs: &[StablecoinOutput]) -> ExecCounts {
    let total = outputs.len();
    let successful = outputs.iter().filter(|o| !o.writes.is_empty()).count();
    ExecCounts { total, successful }
}

impl TransactionOutput for StablecoinOutput {
    type T = StablecoinTx;

    fn get_writes(&self) -> Vec<(StateKey, StateValue)> {
        self.writes.clone()
    }

    fn skip_output() -> Self {
        StablecoinOutput { writes: vec![] }
    }
}

impl ExecutorTask for StablecoinExecutor {
    type T = StablecoinTx;
    type Output = StablecoinOutput;
    type Error = String;
    type Argument = StablecoinExecArgs;

    fn init(args: Self::Argument) -> Self {
        StablecoinExecutor {
            crypto_work_iters: args.crypto_work_iters,
            hot_delta: args.hot_delta,
            funded_balance: args.funded_balance,
        }
    }

    fn execute_transaction(
        &self,
        view: &MVHashMapView<StateKey, StateValue>,
        txn: &StablecoinTx,
    ) -> ExecutionStatus<Self::Output, Self::Error> {
        // Simulate signature verification + VM execution overhead.
        let _work = simulate_tx_crypto_work(txn.tx_hash, self.crypto_work_iters);

        match &txn.tx_type {
            StablecoinTxType::Transfer {
                sender,
                receiver,
                amount,
            } => {
                // Read sender balance. If sender is a hot account (has delta
                // shards from receiving funds), aggregate deltas into the
                // balance and reset them. Block-STM's conflict detection
                // handles concurrent delta writes correctly via re-execution.
                let fb = self.funded_balance;
                let (sender_bal, sender_delta_resets) = if let Some(ref mgr) = self.hot_delta {
                    if mgr.is_hot(*sender) {
                        let aggregated = read_balance_with_deltas(view, *sender, mgr, fb);
                        let p = mgr.shard_count(*sender);
                        let resets: Vec<(StateKey, StateValue)> = (0..p)
                            .map(|s| (StateKey::Delta(*sender, s as u64), 0))
                            .collect();
                        (aggregated, resets)
                    } else {
                        (read_balance(view, *sender, fb), vec![])
                    }
                } else {
                    (read_balance(view, *sender, fb), vec![])
                };

                if sender_bal < *amount {
                    return ExecutionStatus::Success(StablecoinOutput { writes: vec![] });
                }
                let sender_nonce = read_u64(view, &StateKey::Nonce(*sender));

                // Hot-Delta: if receiver is hot, write to delta shard instead of Balance.
                let mut writes = if let Some(ref mgr) = self.hot_delta {
                    if mgr.is_hot(*receiver) {
                        let p = mgr.shard_count(*receiver);
                        let delta_key = HotDeltaManager::delta_key(*receiver, txn.tx_hash, p);
                        let old_delta = read_u64(view, &delta_key);
                        vec![
                            (StateKey::Balance(*sender), sender_bal - amount),
                            (delta_key, old_delta + amount),
                            (StateKey::Nonce(*sender), sender_nonce + 1),
                        ]
                    } else {
                        let receiver_bal = read_balance(view, *receiver, fb);
                        vec![
                            (StateKey::Balance(*sender), sender_bal - amount),
                            (StateKey::Balance(*receiver), receiver_bal + amount),
                            (StateKey::Nonce(*sender), sender_nonce + 1),
                        ]
                    }
                } else {
                    let receiver_bal = read_balance(view, *receiver, fb);
                    vec![
                        (StateKey::Balance(*sender), sender_bal - amount),
                        (StateKey::Balance(*receiver), receiver_bal + amount),
                        (StateKey::Nonce(*sender), sender_nonce + 1),
                    ]
                };
                // Reset sender's delta shards (already aggregated into Balance).
                writes.extend(sender_delta_resets);
                ExecutionStatus::Success(StablecoinOutput { writes })
            }
            StablecoinTxType::Mint { to, amount } => {
                let supply = read_u64(view, &StateKey::TotalSupply);
                let minter_nonce = read_u64(view, &StateKey::Nonce(0));

                // Hot-Delta: if target is hot, write to delta shard.
                let writes = if let Some(ref mgr) = self.hot_delta {
                    if mgr.is_hot(*to) {
                        let p = mgr.shard_count(*to);
                        let delta_key = HotDeltaManager::delta_key(*to, txn.tx_hash, p);
                        let old_delta = read_u64(view, &delta_key);
                        vec![
                            (delta_key, old_delta + amount),
                            (StateKey::TotalSupply, supply + amount),
                            (StateKey::Nonce(0), minter_nonce + 1),
                        ]
                    } else {
                        let bal = read_balance(view, *to, self.funded_balance);
                        vec![
                            (StateKey::Balance(*to), bal + amount),
                            (StateKey::TotalSupply, supply + amount),
                            (StateKey::Nonce(0), minter_nonce + 1),
                        ]
                    }
                } else {
                    let bal = read_balance(view, *to, self.funded_balance);
                    vec![
                        (StateKey::Balance(*to), bal + amount),
                        (StateKey::TotalSupply, supply + amount),
                        (StateKey::Nonce(0), minter_nonce + 1),
                    ]
                };
                ExecutionStatus::Success(StablecoinOutput { writes })
            }
            StablecoinTxType::Burn { from, amount } => {
                // For Burn, read the full balance (including delta aggregation if hot).
                let bal = if let Some(ref mgr) = self.hot_delta {
                    read_balance_with_deltas(view, *from, mgr, self.funded_balance)
                } else {
                    read_balance(view, *from, self.funded_balance)
                };
                if bal < *amount {
                    return ExecutionStatus::Success(StablecoinOutput { writes: vec![] });
                }
                let supply = read_u64(view, &StateKey::TotalSupply);
                let burner_nonce = read_u64(view, &StateKey::Nonce(0));

                let mut writes = vec![
                    (StateKey::Balance(*from), bal - amount),
                    (StateKey::TotalSupply, supply - amount),
                    (StateKey::Nonce(0), burner_nonce + 1),
                ];
                // Reset delta shards to 0 (balance was aggregated above).
                if let Some(ref mgr) = self.hot_delta {
                    if mgr.is_hot(*from) {
                        let p = mgr.shard_count(*from);
                        for s in 0..p {
                            writes.push((StateKey::Delta(*from, s as u64), 0));
                        }
                    }
                }
                ExecutionStatus::Success(StablecoinOutput { writes })
            }
            StablecoinTxType::InitBalance { account, amount } => {
                // Conflict-free funding: writes Balance(account).
                // No shared state (TotalSupply, Nonce) touched.
                // If the account has hot-delta shards, reset them to 0
                // (CADO may place transfers before InitBalance, and their
                // delta writes would otherwise survive the Balance overwrite).
                let mut writes = vec![(StateKey::Balance(*account), *amount)];
                if let Some(ref mgr) = self.hot_delta {
                    if mgr.is_hot(*account) {
                        let p = mgr.shard_count(*account);
                        for s in 0..p {
                            writes.push((StateKey::Delta(*account, s as u64), 0));
                        }
                    }
                }
                ExecutionStatus::Success(StablecoinOutput { writes })
            }
        }
    }
}

fn read_u64(view: &MVHashMapView<StateKey, StateValue>, key: &StateKey) -> u64 {
    match view.read(key) {
        Ok(Some(v)) => *v,
        Ok(None) => 0,
        Err(_) => 0, // dependency will be handled by scheduler
    }
}

/// Read a Balance key, returning `funded_balance` if never written in this block.
/// Ok(None) = key never written → pre-existing funded account.
/// Ok(Some(0)) = explicitly written 0 → drained account.
fn read_balance(view: &MVHashMapView<StateKey, StateValue>, account: u64, funded_balance: u64) -> u64 {
    match view.read(&StateKey::Balance(account)) {
        Ok(Some(v)) => *v,
        Ok(None) => funded_balance,
        Err(_) => funded_balance,
    }
}

/// Read the full balance of an account, aggregating delta shards if hot.
fn read_balance_with_deltas(
    view: &MVHashMapView<StateKey, StateValue>,
    account: u64,
    mgr: &HotDeltaManager,
    funded_balance: u64,
) -> u64 {
    let base = read_balance(view, account, funded_balance);
    if !mgr.is_hot(account) {
        return base;
    }
    let p = mgr.shard_count(account);
    let mut total = base;
    for s in 0..p {
        total += read_u64(view, &StateKey::Delta(account, s as u64));
    }
    total
}

// ---------------------------------------------------------------------------
// Workload generator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum HotspotConfig {
    /// Uniform random — all accounts equally likely as receiver.
    Uniform,
    /// Zipf distribution — alpha > 0, higher = more concentrated.
    Zipf { alpha: f64 },
    /// Explicit hotspots — a fraction of txns go to a few accounts.
    Explicit {
        num_hotspots: usize,
        hotspot_ratio: f64,
    },
}

pub struct StablecoinWorkloadGenerator {
    pub num_accounts: usize,
    pub hotspot_config: HotspotConfig,
}

impl StablecoinWorkloadGenerator {
    pub fn new(num_accounts: usize, hotspot: HotspotConfig) -> Self {
        Self {
            num_accounts,
            hotspot_config: hotspot,
        }
    }

    /// Generate n stablecoin transfer transactions (random seed).
    pub fn generate(&self, n: usize) -> Vec<StablecoinTx> {
        self.generate_with_rng(n, &mut rand::thread_rng())
    }

    /// Generate n stablecoin transfer transactions with a fixed seed.
    /// Same seed + same n = identical transaction sequence.
    pub fn generate_seeded(&self, n: usize, seed: u64) -> Vec<StablecoinTx> {
        self.generate_with_rng(n, &mut rand::rngs::StdRng::seed_from_u64(seed))
    }

    fn generate_with_rng<R: Rng>(&self, n: usize, rng: &mut R) -> Vec<StablecoinTx> {
        let mut txns = Vec::with_capacity(n);

        for i in 0..n {
            let sender = rng.gen_range(0..self.num_accounts) as u64;
            let receiver = self.pick_receiver(rng, sender);
            let amount = rng.gen_range(1..=100);

            let mut hasher = DefaultHasher::new();
            (i as u64).hash(&mut hasher);
            sender.hash(&mut hasher);
            receiver.hash(&mut hasher);
            let tx_hash = hasher.finish();

            txns.push(StablecoinTx {
                tx_type: StablecoinTxType::Transfer {
                    sender,
                    receiver,
                    amount,
                },
                nonce: i as u64,
                tx_hash,
            });
        }
        txns
    }

    /// Generate transactions WITH initial funding.
    /// Prepends InitBalance transactions to fund every sender account.
    /// InitBalance txns are conflict-free (each writes only Balance(account)),
    /// so they execute in parallel without abort cascading.
    pub fn generate_with_funding(&self, n: usize, initial_balance: u64) -> Vec<StablecoinTx> {
        let transfers = self.generate(n);

        // Collect unique senders.
        let senders: Vec<u64> = transfers
            .iter()
            .filter_map(|tx| match &tx.tx_type {
                StablecoinTxType::Transfer { sender, .. } => Some(*sender),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<u64>>()
            .into_iter()
            .collect();

        // Create InitBalance txns to fund each sender (conflict-free).
        // Use high nonce range (u64::MAX - i) to avoid collision with Transfer nonces
        // (which are 0..n) during CADO dedup by (sender, nonce).
        let mut all_txns = Vec::with_capacity(senders.len() + n);
        for (i, acct) in senders.iter().enumerate() {
            let mut hasher = DefaultHasher::new();
            (u64::MAX - i as u64).hash(&mut hasher);
            acct.hash(&mut hasher);
            all_txns.push(StablecoinTx {
                tx_type: StablecoinTxType::InitBalance {
                    account: *acct,
                    amount: initial_balance,
                },
                nonce: u64::MAX - i as u64,
                tx_hash: hasher.finish(),
            });
        }
        all_txns.extend(transfers);
        all_txns
    }

    fn pick_receiver(&self, rng: &mut impl Rng, sender: u64) -> u64 {
        match &self.hotspot_config {
            HotspotConfig::Uniform => {
                let dist = Uniform::new(0, self.num_accounts);
                loop {
                    let r = dist.sample(rng) as u64;
                    if r != sender {
                        return r;
                    }
                }
            }
            HotspotConfig::Zipf { alpha } => {
                let dist = Zipf::new(self.num_accounts as u64, *alpha).unwrap();
                loop {
                    let r = (dist.sample(rng) as u64).saturating_sub(1); // Zipf is 1-based
                    let r = r.min((self.num_accounts - 1) as u64);
                    if r != sender {
                        return r;
                    }
                }
            }
            HotspotConfig::Explicit {
                num_hotspots,
                hotspot_ratio,
            } => {
                if rng.gen_bool(*hotspot_ratio) {
                    // Pick a hotspot account (first num_hotspots accounts).
                    let r = rng.gen_range(0..*num_hotspots) as u64;
                    if r != sender {
                        return r;
                    }
                    // Fallback to uniform if same as sender.
                    return (r + 1) % self.num_accounts as u64;
                }
                // Non-hotspot: uniform.
                let dist = Uniform::new(0, self.num_accounts);
                loop {
                    let r = dist.sample(rng) as u64;
                    if r != sender {
                        return r;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Serial executor for correctness verification
// ---------------------------------------------------------------------------

/// Execute transactions sequentially, returning final state.
pub fn serial_execute(txns: &[StablecoinTx], crypto_work_iters: u32) -> std::collections::HashMap<StateKey, StateValue> {
    serial_execute_with_balance(txns, crypto_work_iters, 0)
}

pub fn serial_execute_with_balance(txns: &[StablecoinTx], crypto_work_iters: u32, funded_balance: u64) -> std::collections::HashMap<StateKey, StateValue> {
    let mut state = std::collections::HashMap::new();

    for txn in txns {
        // Simulate signature verification + VM execution overhead (same as parallel path).
        let _work = simulate_tx_crypto_work(txn.tx_hash, crypto_work_iters);

        match &txn.tx_type {
            StablecoinTxType::Transfer {
                sender,
                receiver,
                amount,
            } => {
                let sender_bal = *state.get(&StateKey::Balance(*sender)).unwrap_or(&funded_balance);
                if sender_bal >= *amount {
                    let receiver_bal = *state.get(&StateKey::Balance(*receiver)).unwrap_or(&funded_balance);
                    let sender_nonce = *state.get(&StateKey::Nonce(*sender)).unwrap_or(&0);
                    state.insert(StateKey::Balance(*sender), sender_bal - amount);
                    state.insert(StateKey::Balance(*receiver), receiver_bal + amount);
                    state.insert(StateKey::Nonce(*sender), sender_nonce + 1);
                }
            }
            StablecoinTxType::Mint { to, amount } => {
                let bal = *state.get(&StateKey::Balance(*to)).unwrap_or(&funded_balance);
                let supply = *state.get(&StateKey::TotalSupply).unwrap_or(&0);
                let nonce = *state.get(&StateKey::Nonce(0)).unwrap_or(&0);
                state.insert(StateKey::Balance(*to), bal + amount);
                state.insert(StateKey::TotalSupply, supply + amount);
                state.insert(StateKey::Nonce(0), nonce + 1);
            }
            StablecoinTxType::Burn { from, amount } => {
                let bal = *state.get(&StateKey::Balance(*from)).unwrap_or(&funded_balance);
                if bal >= *amount {
                    let supply = *state.get(&StateKey::TotalSupply).unwrap_or(&0);
                    let nonce = *state.get(&StateKey::Nonce(0)).unwrap_or(&0);
                    state.insert(StateKey::Balance(*from), bal - amount);
                    state.insert(StateKey::TotalSupply, supply - amount);
                    state.insert(StateKey::Nonce(0), nonce + 1);
                }
            }
            StablecoinTxType::InitBalance { account, amount } => {
                state.insert(StateKey::Balance(*account), *amount);
            }
        }
    }
    state
}

/// Execute transactions sequentially with success counting.
/// Same logic as serial_execute but also returns ExecCounts.
pub fn serial_execute_counted(txns: &[StablecoinTx], crypto_work_iters: u32, funded_balance: u64) -> (std::collections::HashMap<StateKey, StateValue>, ExecCounts) {
    let mut state = std::collections::HashMap::new();
    let mut counts = ExecCounts { total: txns.len(), successful: 0 };

    for txn in txns {
        let _work = simulate_tx_crypto_work(txn.tx_hash, crypto_work_iters);

        let success = match &txn.tx_type {
            StablecoinTxType::Transfer { sender, receiver, amount } => {
                let sender_bal = *state.get(&StateKey::Balance(*sender)).unwrap_or(&funded_balance);
                if sender_bal >= *amount {
                    let receiver_bal = *state.get(&StateKey::Balance(*receiver)).unwrap_or(&funded_balance);
                    let sender_nonce = *state.get(&StateKey::Nonce(*sender)).unwrap_or(&0);
                    state.insert(StateKey::Balance(*sender), sender_bal - amount);
                    state.insert(StateKey::Balance(*receiver), receiver_bal + amount);
                    state.insert(StateKey::Nonce(*sender), sender_nonce + 1);
                    true
                } else {
                    false
                }
            }
            StablecoinTxType::Mint { to, amount } => {
                let bal = *state.get(&StateKey::Balance(*to)).unwrap_or(&funded_balance);
                let supply = *state.get(&StateKey::TotalSupply).unwrap_or(&0);
                let nonce = *state.get(&StateKey::Nonce(0)).unwrap_or(&0);
                state.insert(StateKey::Balance(*to), bal + amount);
                state.insert(StateKey::TotalSupply, supply + amount);
                state.insert(StateKey::Nonce(0), nonce + 1);
                true
            }
            StablecoinTxType::Burn { from, amount } => {
                let bal = *state.get(&StateKey::Balance(*from)).unwrap_or(&funded_balance);
                if bal >= *amount {
                    let supply = *state.get(&StateKey::TotalSupply).unwrap_or(&0);
                    let nonce = *state.get(&StateKey::Nonce(0)).unwrap_or(&0);
                    state.insert(StateKey::Balance(*from), bal - amount);
                    state.insert(StateKey::TotalSupply, supply - amount);
                    state.insert(StateKey::Nonce(0), nonce + 1);
                    true
                } else {
                    false
                }
            }
            StablecoinTxType::InitBalance { account, amount } => {
                state.insert(StateKey::Balance(*account), *amount);
                true
            }
        };
        if success {
            counts.successful += 1;
        }
    }
    (state, counts)
}

/// Execute transactions in parallel via LEAP and extract final state from outputs.
pub fn parallel_execute_to_state(
    txns: Vec<StablecoinTx>,
    num_threads: usize,
    crypto_work_iters: u32,
) -> std::collections::HashMap<StateKey, StateValue> {
    use crate::config::LeapConfig;
    use crate::executor::ParallelTransactionExecutor;

    let config = LeapConfig {
        num_workers: num_threads,
        enable_backpressure: false,
        enable_domain_aware: false,
        enable_hot_delta: false,
        ..LeapConfig::default()
    };

    let args = StablecoinExecArgs {
        crypto_work_iters,
        hot_delta: None,
        funded_balance: 0,
    };

    let executor =
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config);
    let outputs = executor
        .execute_transactions_parallel(args, txns)
        .expect("Parallel execution should succeed");

    // Reconstruct state by applying writes in order.
    let mut state = std::collections::HashMap::new();
    for output in &outputs {
        for (k, v) in output.get_writes() {
            state.insert(k, v);
        }
    }
    state
}

/// Execute transactions in parallel with Hot-Delta enabled and aggregate deltas.
pub fn parallel_execute_to_state_with_hot_delta(
    txns: Vec<StablecoinTx>,
    num_threads: usize,
    crypto_work_iters: u32,
    hot_delta: Arc<HotDeltaManager>,
) -> std::collections::HashMap<StateKey, StateValue> {
    use crate::config::LeapConfig;
    use crate::executor::ParallelTransactionExecutor;

    let config = LeapConfig {
        num_workers: num_threads,
        enable_backpressure: false,
        enable_domain_aware: false,
        enable_hot_delta: true,
        ..LeapConfig::default()
    };

    let args = StablecoinExecArgs {
        crypto_work_iters,
        hot_delta: Some(hot_delta.clone()),
        funded_balance: 0,
    };

    let executor =
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config);
    let outputs = executor
        .execute_transactions_parallel(args, txns)
        .expect("Parallel execution should succeed");

    // Reconstruct state: apply writes in order, then aggregate deltas into balances.
    let mut state = std::collections::HashMap::new();
    for output in &outputs {
        for (k, v) in output.get_writes() {
            state.insert(k, v);
        }
    }

    // Post-process: aggregate Delta(account, shard) into Balance(account).
    let delta_keys: Vec<(StateKey, u64)> = state
        .iter()
        .filter_map(|(k, &v)| {
            if let StateKey::Delta(_, _) = k {
                Some((k.clone(), v))
            } else {
                None
            }
        })
        .collect();

    for (k, delta_val) in &delta_keys {
        if let StateKey::Delta(account, _) = k {
            let bal = state.entry(StateKey::Balance(*account)).or_insert(0);
            *bal += delta_val;
        }
    }

    // Remove delta keys from state (they've been folded into balances).
    for (k, _) in &delta_keys {
        state.remove(k);
    }

    state
}
