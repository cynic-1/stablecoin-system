use crate::stablecoin::StablecoinTx;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Conflict-Aware Deterministic Ordering (CADO).
///
/// Orders transactions deterministically for optimal parallel execution:
/// 1. Deduplicate by (sender, nonce)
/// 2. Group by conflict domain
/// 3. Sort domains by H(domain_id); within each domain sort by (sender, nonce, tx_hash)
/// 4. Concatenate domains sequentially in sorted order
///
/// Sequential concatenation groups same-domain transactions together, enabling
/// DomainPlan to build large parallel segments. The scheduler's domain-aware
/// throttling then controls how far execution can run ahead of validation at
/// segment boundaries, preventing speculative waste.
pub fn cado_ordering(txs: &mut Vec<StablecoinTx>) {
    // Step 1: Deduplicate by (sender, nonce) — keep first occurrence.
    let mut seen = std::collections::HashSet::new();
    txs.retain(|tx| seen.insert((tx.sender(), tx.nonce)));

    // Step 2-3: Group by domain, sort domains and intra-domain order.
    let mut domain_groups: std::collections::BTreeMap<u64, Vec<StablecoinTx>> =
        std::collections::BTreeMap::new();
    for tx in txs.drain(..) {
        let dh = domain_hash(tx.conflict_domain());
        domain_groups.entry(dh).or_default().push(tx);
    }

    // Sort within each domain by (sender, nonce, tx_hash).
    for group in domain_groups.values_mut() {
        group.sort_by(|a, b| {
            a.sender()
                .cmp(&b.sender())
                .then_with(|| a.nonce.cmp(&b.nonce))
                .then_with(|| a.tx_hash.cmp(&b.tx_hash))
        });
    }

    // Step 4: Sequential concatenation — group same-domain txns together.
    // This enables DomainPlan to build large parallel segments.
    for (_domain_hash, group) in domain_groups {
        txs.extend(group);
    }
}

fn domain_hash(domain: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    domain.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stablecoin::{StablecoinTx, StablecoinTxType};

    fn make_transfer(sender: u64, receiver: u64, nonce: u64, tx_hash: u64) -> StablecoinTx {
        StablecoinTx {
            tx_type: StablecoinTxType::Transfer {
                sender,
                receiver,
                amount: 10,
            },
            nonce,
            tx_hash,
        }
    }

    #[test]
    fn test_determinism() {
        let mut txs1 = vec![
            make_transfer(1, 5, 0, 100),
            make_transfer(2, 3, 0, 200),
            make_transfer(3, 5, 0, 300),
            make_transfer(4, 3, 1, 400),
        ];
        let mut txs2 = txs1.clone();

        cado_ordering(&mut txs1);
        cado_ordering(&mut txs2);

        assert_eq!(txs1.len(), txs2.len());
        for (a, b) in txs1.iter().zip(txs2.iter()) {
            assert_eq!(a.tx_hash, b.tx_hash);
        }
    }

    #[test]
    fn test_deduplication() {
        let mut txs = vec![
            make_transfer(1, 2, 0, 100),
            make_transfer(1, 3, 0, 200), // duplicate (sender=1, nonce=0)
            make_transfer(2, 3, 0, 300),
        ];
        cado_ordering(&mut txs);
        assert_eq!(txs.len(), 2); // one duplicate removed
    }

    #[test]
    fn test_grouping_by_domain() {
        let mut txs = vec![
            make_transfer(1, 10, 0, 100), // domain=10
            make_transfer(2, 20, 0, 200), // domain=20
            make_transfer(3, 10, 1, 300), // domain=10
            make_transfer(4, 20, 1, 400), // domain=20
        ];
        cado_ordering(&mut txs);

        // After sequential concatenation, same-domain txns should be grouped together.
        let domains: Vec<u64> = txs.iter().map(|t| t.conflict_domain()).collect();
        assert_eq!(domains.len(), 4);
        // Same-domain txns are adjacent (concatenated).
        assert_eq!(domains[0], domains[1]);
        assert_eq!(domains[2], domains[3]);
        // Different domains are in separate groups.
        assert_ne!(domains[0], domains[2]);
    }
}
