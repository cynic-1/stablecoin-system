use crate::config::CadoMode;
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

/// Conflict-Aware Deterministic Ordering — Interleave mode.
///
/// Same deduplication and intra-domain sorting as `cado_ordering`, but instead
/// of concatenating domains sequentially, interleaves them round-robin.
/// This maximizes distance between same-receiver transactions, producing
/// OCC-friendly input that reduces speculative conflicts.
///
/// ```text
/// Concatenate: [A,A,A, B,B,B, C,C,C]
/// Interleave:  [A,B,C, A,B,C, A,B,C]
/// ```
pub fn cado_interleave(txs: &mut Vec<StablecoinTx>) {
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

    // Step 4: Round-robin interleave.
    // Sort domain groups by size descending for best distribution.
    let mut groups: Vec<Vec<StablecoinTx>> = domain_groups.into_values().collect();
    groups.sort_by(|a, b| b.len().cmp(&a.len()));

    // Convert to iterators for round-robin consumption.
    let mut iters: Vec<std::vec::IntoIter<StablecoinTx>> =
        groups.into_iter().map(|g| g.into_iter()).collect();

    loop {
        let mut any = false;
        for iter in &mut iters {
            if let Some(tx) = iter.next() {
                txs.push(tx);
                any = true;
            }
        }
        if !any {
            break;
        }
    }
}

/// Dispatcher: apply CADO ordering based on the configured mode.
pub fn cado_with_mode(txs: &mut Vec<StablecoinTx>, mode: &CadoMode) {
    match mode {
        CadoMode::Disabled => {}
        CadoMode::Concatenate => cado_ordering(txs),
        CadoMode::Interleave => cado_interleave(txs),
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

    #[test]
    fn test_interleave_spreads_domains() {
        let mut txs = vec![
            make_transfer(1, 10, 0, 100), // domain=10
            make_transfer(2, 10, 1, 200), // domain=10
            make_transfer(3, 10, 2, 300), // domain=10
            make_transfer(4, 20, 0, 400), // domain=20
            make_transfer(5, 20, 1, 500), // domain=20
            make_transfer(6, 20, 2, 600), // domain=20
            make_transfer(7, 30, 0, 700), // domain=30
            make_transfer(8, 30, 1, 800), // domain=30
            make_transfer(9, 30, 2, 900), // domain=30
        ];
        cado_interleave(&mut txs);

        let domains: Vec<u64> = txs.iter().map(|t| t.conflict_domain()).collect();
        assert_eq!(domains.len(), 9);

        // No two consecutive txns should share the same domain.
        for i in 0..domains.len() - 1 {
            assert_ne!(
                domains[i], domains[i + 1],
                "Adjacent txns at positions {} and {} share domain {}",
                i, i + 1, domains[i]
            );
        }
    }

    #[test]
    fn test_interleave_determinism() {
        let mut txs1 = vec![
            make_transfer(1, 5, 0, 100),
            make_transfer(2, 3, 0, 200),
            make_transfer(3, 5, 0, 300),
            make_transfer(4, 3, 1, 400),
        ];
        let mut txs2 = txs1.clone();

        cado_interleave(&mut txs1);
        cado_interleave(&mut txs2);

        assert_eq!(txs1.len(), txs2.len());
        for (a, b) in txs1.iter().zip(txs2.iter()) {
            assert_eq!(a.tx_hash, b.tx_hash);
        }
    }

    #[test]
    fn test_cado_with_mode_disabled() {
        use crate::config::CadoMode;
        let mut txs = vec![
            make_transfer(1, 10, 0, 100),
            make_transfer(2, 20, 0, 200),
        ];
        let original: Vec<u64> = txs.iter().map(|t| t.tx_hash).collect();
        cado_with_mode(&mut txs, &CadoMode::Disabled);
        let after: Vec<u64> = txs.iter().map(|t| t.tx_hash).collect();
        assert_eq!(original, after, "Disabled mode should not modify txns");
    }
}
