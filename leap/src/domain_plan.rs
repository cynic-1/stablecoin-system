use crate::stablecoin::StablecoinTx;
use std::collections::HashSet;

/// A contiguous segment of transactions belonging to the same conflict domain.
#[derive(Debug, Clone)]
pub struct DomainSegment {
    pub start: usize,
    pub end: usize,
    pub domain: u64,
    pub write_keys: HashSet<u64>,
}

/// An execution plan built from CADO-sorted transactions.
/// `par_bound[j]` is true if segment j can execute in parallel with segment j-1.
#[derive(Debug, Clone)]
pub struct DomainPlan {
    pub segments: Vec<DomainSegment>,
    pub par_bound: Vec<bool>,
}

/// Build a domain-aware execution plan from CADO-sorted transactions.
///
/// Segments are formed by grouping consecutive transactions with the same conflict domain,
/// splitting when a segment exceeds `l_max` transactions.
///
/// Two adjacent segments can run in parallel (`par_bound[j] = true`) if they have
/// disjoint write key sets (weak independence).
pub fn build_domain_plan(txs: &[StablecoinTx], l_max: usize) -> DomainPlan {
    if txs.is_empty() {
        return DomainPlan {
            segments: vec![],
            par_bound: vec![],
        };
    }

    let mut segments = Vec::new();
    let mut seg_start = 0;

    while seg_start < txs.len() {
        let domain = txs[seg_start].conflict_domain();
        let mut seg_end = seg_start + 1;

        // Extend segment while same domain and under l_max.
        while seg_end < txs.len()
            && txs[seg_end].conflict_domain() == domain
            && (seg_end - seg_start) < l_max
        {
            seg_end += 1;
        }

        // Collect write keys for this segment.
        let mut write_keys = HashSet::new();
        for tx in &txs[seg_start..seg_end] {
            collect_write_accounts(tx, &mut write_keys);
        }

        segments.push(DomainSegment {
            start: seg_start,
            end: seg_end,
            domain,
            write_keys,
        });

        seg_start = seg_end;
    }

    // Compute parallel bounds.
    let mut par_bound = vec![false; segments.len()];
    for j in 1..segments.len() {
        if segments[j].domain == segments[j - 1].domain {
            // Same-domain l_max split: always parallel. Intra-domain contention
            // is handled by Hot-Delta (for hot accounts) and Block-STM OCC
            // (for all accounts). Throttling here is counterproductive.
            par_bound[j] = true;
        } else {
            // Cross-domain boundary: check disjoint write sets (weak independence).
            par_bound[j] = segments[j]
                .write_keys
                .is_disjoint(&segments[j - 1].write_keys);
        }
    }

    DomainPlan {
        segments,
        par_bound,
    }
}

impl DomainPlan {
    /// Extract segment boundaries for the scheduler: (start, end_exclusive, par_bound).
    pub fn segment_bounds(&self) -> Vec<(usize, usize, bool)> {
        self.segments
            .iter()
            .enumerate()
            .map(|(i, seg)| (seg.start, seg.end, self.par_bound.get(i).copied().unwrap_or(false)))
            .collect()
    }

    /// Build O(1) txn-to-segment lookup array. segment_of[txn_idx] = segment index.
    /// Used by the scheduler to avoid binary search on every next_task() call.
    pub fn txn_to_segment(&self, num_txns: usize) -> Vec<usize> {
        let num_segments = self.segments.len();
        let mut result = vec![num_segments; num_txns]; // default: beyond all segments
        for (seg_idx, seg) in self.segments.iter().enumerate() {
            for i in seg.start..seg.end.min(num_txns) {
                result[i] = seg_idx;
            }
        }
        result
    }
}

/// Collect the **conflict-domain keys** that a transaction writes to.
///
/// For par_bound (weak independence) checks, we only include the conflict domain
/// key — the receiver/target account that CADO groups by. Sender accounts are
/// excluded because sender overlap between domains is incidental (randomly
/// distributed), not structural. Block-STM's OCC handles incidental conflicts
/// via abort+re-execute; throttling for them adds overhead without benefit.
fn collect_write_accounts(tx: &StablecoinTx, keys: &mut HashSet<u64>) {
    use crate::stablecoin::StablecoinTxType;
    match &tx.tx_type {
        StablecoinTxType::Transfer { receiver, .. } => {
            keys.insert(*receiver);
        }
        StablecoinTxType::Mint { to, .. } => {
            keys.insert(*to);
            keys.insert(u64::MAX); // totalSupply
        }
        StablecoinTxType::Burn { from, .. } => {
            keys.insert(*from);
            keys.insert(u64::MAX); // totalSupply
        }
        StablecoinTxType::InitBalance { account, .. } => {
            keys.insert(*account);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stablecoin::{StablecoinTx, StablecoinTxType};

    fn make_transfer(sender: u64, receiver: u64) -> StablecoinTx {
        StablecoinTx {
            tx_type: StablecoinTxType::Transfer {
                sender,
                receiver,
                amount: 10,
            },
            nonce: 0,
            tx_hash: sender * 1000 + receiver,
        }
    }

    #[test]
    fn test_segments_by_domain() {
        // Pre-sorted by CADO: domain 5, domain 5, domain 10, domain 10
        let txs = vec![
            make_transfer(1, 5),
            make_transfer(2, 5),
            make_transfer(3, 10),
            make_transfer(4, 10),
        ];
        let plan = build_domain_plan(&txs, 256);
        assert_eq!(plan.segments.len(), 2);
        assert_eq!(plan.segments[0].domain, 5);
        assert_eq!(plan.segments[1].domain, 10);
    }

    #[test]
    fn test_parallel_disjoint_segments() {
        // Domain 5: writes to {1,5} and {2,5}
        // Domain 10: writes to {3,10} and {4,10}
        // Disjoint write keys → par_bound[1] = true
        let txs = vec![
            make_transfer(1, 5),
            make_transfer(2, 5),
            make_transfer(3, 10),
            make_transfer(4, 10),
        ];
        let plan = build_domain_plan(&txs, 256);
        assert!(plan.par_bound[1]); // domains 5 and 10 are disjoint
    }

    #[test]
    fn test_parallel_cross_domain_sender_overlap() {
        // Domain 5: receiver=5, Domain 10: receiver=10.
        // Sender account 5 appears in both, but sender overlap is incidental
        // (handled by Block-STM OCC), so par_bound should be true.
        let txs = vec![
            make_transfer(1, 5),  // domain 5, write_keys={5}
            make_transfer(5, 10), // domain 10, write_keys={10}
        ];
        let plan = build_domain_plan(&txs, 256);
        assert!(plan.par_bound[1]); // disjoint conflict domains → parallel
    }

    #[test]
    fn test_non_parallel_shared_target() {
        // Two Mint transactions targeting different domains but sharing totalSupply.
        use crate::stablecoin::StablecoinTxType;
        let txs = vec![
            StablecoinTx {
                tx_type: StablecoinTxType::Mint { to: 5, amount: 100 },
                nonce: 0,
                tx_hash: 1,
            },
            StablecoinTx {
                tx_type: StablecoinTxType::Mint { to: 10, amount: 100 },
                nonce: 1,
                tx_hash: 2,
            },
        ];
        let plan = build_domain_plan(&txs, 256);
        // Both write to u64::MAX (totalSupply) → non-parallel
        assert!(!plan.par_bound[1]);
    }

    #[test]
    fn test_l_max_splitting() {
        let txs: Vec<_> = (0..10)
            .map(|i| make_transfer(100 + i, 5)) // all domain 5
            .collect();
        let plan = build_domain_plan(&txs, 4);
        assert_eq!(plan.segments.len(), 3); // 4+4+2 = 10
    }
}
