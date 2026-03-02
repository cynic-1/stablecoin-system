use crate::stablecoin::{StateKey, StateValue, StablecoinTx, StablecoinTxType};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Manages Hot-Delta sharding for hotspot accounts.
///
/// When an account is identified as a hotspot (receives many transactions),
/// its balance writes are sharded: instead of writing to `Balance(account)`,
/// writers write to `Delta(account, shard_id)`. Readers aggregate all shards.
#[derive(Debug, Clone)]
pub struct HotDeltaManager {
    /// account → number of shards P(a).
    shard_counts: HashMap<u64, usize>,
    /// Total unique receiver accounts seen in detect_hotspots().
    unique_receivers: usize,
    /// Frequency threshold to start sharding (default: 10).
    pub theta_1: usize,
    /// Frequency threshold for max shards (default: 50).
    pub theta_2: usize,
    /// Maximum shard count (default: 8).
    pub p_max: usize,
}

impl HotDeltaManager {
    pub fn new(theta_1: usize, theta_2: usize, p_max: usize) -> Self {
        Self {
            shard_counts: HashMap::new(),
            unique_receivers: 0,
            theta_1,
            theta_2,
            p_max,
        }
    }

    /// Pre-scan transaction sequence to detect hotspot receiver accounts.
    /// An account receiving >= theta_1 transactions is sharded.
    pub fn detect_hotspots(&mut self, txs: &[StablecoinTx]) {
        let mut freq: HashMap<u64, usize> = HashMap::new();

        for tx in txs {
            match &tx.tx_type {
                StablecoinTxType::Transfer { receiver, .. } => {
                    *freq.entry(*receiver).or_default() += 1;
                }
                StablecoinTxType::Mint { to, .. } => {
                    *freq.entry(*to).or_default() += 1;
                }
                StablecoinTxType::Burn { from, .. } => {
                    *freq.entry(*from).or_default() += 1;
                }
                StablecoinTxType::InitBalance { .. } => {}
            }
        }

        self.unique_receivers = freq.len();
        self.shard_counts.clear();
        for (account, count) in freq {
            if count >= self.theta_1 {
                let p = if count >= self.theta_2 {
                    self.p_max
                } else {
                    // Linear interpolation between 2 and p_max.
                    let ratio = (count - self.theta_1) as f64
                        / (self.theta_2 - self.theta_1) as f64;
                    2 + ((self.p_max - 2) as f64 * ratio) as usize
                };
                self.shard_counts.insert(account, p.min(self.p_max).max(2));
            }
        }
    }

    /// Returns true if the workload has genuine skew (a small fraction of
    /// receivers are hot). When false, contention is uniform and HotDelta
    /// sharding hurts more than it helps (9 reads per hot sender vs 1).
    pub fn is_skewed(&self) -> bool {
        if self.unique_receivers == 0 || self.shard_counts.is_empty() {
            return false;
        }
        (self.shard_counts.len() as f64 / self.unique_receivers as f64) < 0.20
    }

    /// Returns the shard count for an account (1 = not sharded).
    pub fn shard_count(&self, account: u64) -> usize {
        *self.shard_counts.get(&account).unwrap_or(&1)
    }

    /// Returns true if this account is sharded.
    pub fn is_hot(&self, account: u64) -> bool {
        self.shard_counts.contains_key(&account)
    }

    /// Compute the delta key for a write to a hot account.
    /// shard_id = hash(tx_hash) % P(account)
    pub fn delta_key(account: u64, tx_hash: u64, shard_count: usize) -> StateKey {
        let mut hasher = DefaultHasher::new();
        tx_hash.hash(&mut hasher);
        let shard = hasher.finish() as usize % shard_count;
        StateKey::Delta(account, shard as u64)
    }

    /// Rewrite a transaction's output writes using Hot-Delta sharding.
    /// Only the receiver's balance write is sharded; sender writes remain direct.
    pub fn rewrite_writes(
        &self,
        tx: &StablecoinTx,
        writes: Vec<(StateKey, StateValue)>,
    ) -> Vec<(StateKey, StateValue)> {
        let receiver_account = match &tx.tx_type {
            StablecoinTxType::Transfer { receiver, .. } => Some(*receiver),
            StablecoinTxType::Mint { to, .. } => Some(*to),
            StablecoinTxType::Burn { .. } => None, // Burn debits, don't shard
            StablecoinTxType::InitBalance { .. } => None, // No sharding for init
        };

        let Some(recv) = receiver_account else {
            return writes;
        };

        let p = self.shard_count(recv);
        if p <= 1 {
            return writes;
        }

        writes
            .into_iter()
            .map(|(k, v)| {
                if k == StateKey::Balance(recv) {
                    // Replace with delta write.
                    let delta_k = Self::delta_key(recv, tx.tx_hash, p);
                    (delta_k, v)
                } else {
                    (k, v)
                }
            })
            .collect()
    }

    /// Get all accounts that are hot.
    pub fn hot_accounts(&self) -> Vec<(u64, usize)> {
        self.shard_counts.iter().map(|(&a, &p)| (a, p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stablecoin::{StablecoinTx, StablecoinTxType};

    fn make_transfer(sender: u64, receiver: u64, tx_hash: u64) -> StablecoinTx {
        StablecoinTx {
            tx_type: StablecoinTxType::Transfer {
                sender,
                receiver,
                amount: 10,
            },
            nonce: 0,
            tx_hash,
        }
    }

    #[test]
    fn test_detect_hotspots() {
        let mut manager = HotDeltaManager::new(3, 10, 8);
        let txs: Vec<_> = (0..5).map(|i| make_transfer(i + 100, 1, i)).collect();
        manager.detect_hotspots(&txs);

        assert!(manager.is_hot(1));
        assert!(!manager.is_hot(100));
        assert!(manager.shard_count(1) >= 2);
    }

    #[test]
    fn test_delta_key_distribution() {
        // Different tx_hashes should spread across shards.
        let p = 4;
        let shards: Vec<_> = (0..100)
            .map(|i| {
                if let StateKey::Delta(_, s) = HotDeltaManager::delta_key(1, i, p) {
                    s
                } else {
                    panic!("Expected Delta key");
                }
            })
            .collect();
        // All shards should be used.
        for s in 0..p as u64 {
            assert!(shards.contains(&s), "Shard {} not used", s);
        }
    }

    #[test]
    fn test_non_hot_accounts_unaffected() {
        let manager = HotDeltaManager::new(10, 50, 8);
        assert_eq!(manager.shard_count(42), 1);
        assert!(!manager.is_hot(42));
    }
}
