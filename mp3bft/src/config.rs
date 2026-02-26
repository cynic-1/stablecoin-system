use crate::types::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// Authority info within the committee.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Authority {
    pub id: NodeId,
    pub stake: u64,
}

/// Committee configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Committee {
    pub authorities: BTreeMap<NodeId, Authority>,
}

impl Committee {
    pub fn new(node_ids: Vec<NodeId>) -> Self {
        let mut authorities = BTreeMap::new();
        for id in node_ids {
            authorities.insert(id, Authority { id, stake: 1 });
        }
        Committee { authorities }
    }

    pub fn size(&self) -> usize {
        self.authorities.len()
    }

    pub fn total_stake(&self) -> u64 {
        self.authorities.values().map(|a| a.stake).sum()
    }

    /// Quorum threshold: 2f+1 where n = 3f+1.
    pub fn quorum_threshold(&self) -> usize {
        let n = self.size();
        2 * n / 3 + 1
    }

    /// Validity threshold: f+1.
    pub fn validity_threshold(&self) -> usize {
        let n = self.size();
        n / 3 + 1
    }

    /// Maximum tolerable faults.
    pub fn max_faults(&self) -> usize {
        (self.size() - 1) / 3
    }

    pub fn node_ids(&self) -> Vec<NodeId> {
        self.authorities.keys().cloned().collect()
    }

    pub fn contains(&self, node_id: &NodeId) -> bool {
        self.authorities.contains_key(node_id)
    }
}

/// MP3-BFT++ protocol parameters.
#[derive(Debug, Clone)]
pub struct MP3BFTConfig {
    pub k_slots: usize,
    pub n_buckets: usize,
    pub m_max: usize,
    pub s_batch: usize,
    pub delta_slot: Duration,
    pub delta_col: Duration,
    pub t_initial: Duration,
    pub t_max: Duration,
    pub rho: f64,
    pub ordering_rule_id: u32,
    /// Simulated network round-trip latency per consensus phase.
    pub network_latency: Duration,
}

impl Default for MP3BFTConfig {
    fn default() -> Self {
        Self {
            k_slots: 8,
            n_buckets: 16384, // 2^14
            m_max: 32,
            s_batch: 1000,
            delta_slot: Duration::from_millis(200),
            delta_col: Duration::from_millis(500),
            t_initial: Duration::from_secs(5),
            t_max: Duration::from_secs(60),
            rho: 1.5,
            ordering_rule_id: 1,
            network_latency: Duration::ZERO,
        }
    }
}
