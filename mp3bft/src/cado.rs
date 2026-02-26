use crate::types::*;
use leap::cado::cado_ordering;
use leap::stablecoin::StablecoinTx;

/// Consensus-side CADO: After a macro-block is committed, extract all
/// transactions and apply CADO ordering to produce the deterministic
/// execution sequence π_h.
pub fn consensus_cado_order(
    _committed_block: &CommittedBlock,
    raw_transactions: Vec<StablecoinTx>,
) -> Vec<StablecoinTx> {
    let mut txns = raw_transactions;
    cado_ordering(&mut txns);
    txns
}

#[cfg(test)]
mod tests {
    use super::*;
    use leap::stablecoin::{StablecoinTx, StablecoinTxType};

    #[test]
    fn test_consensus_cado_deterministic() {
        let txns: Vec<StablecoinTx> = (0..10)
            .map(|i| StablecoinTx {
                tx_type: StablecoinTxType::Transfer {
                    sender: i % 5,
                    receiver: (i + 1) % 5,
                    amount: 10,
                },
                nonce: i / 5,
                tx_hash: i * 1000 + 42,
            })
            .collect();

        let block = CommittedBlock {
            height: 0,
            view: 0,
            header: MacroHeader {
                view: 0, height: 0, leader: 0,
                parent_qc: None, slot_entries: vec![],
                digest: DigestBytes::zero(),
            },
            qc: MacroQC {
                view: 0, height: 0, header_digest: DigestBytes::zero(),
                header: MacroHeader {
                    view: 0, height: 0, leader: 0,
                    parent_qc: None, slot_entries: vec![],
                    digest: DigestBytes::zero(),
                },
                votes: vec![],
            },
            slot_qcs: vec![],
        };

        let ordered1 = consensus_cado_order(&block, txns.clone());
        let ordered2 = consensus_cado_order(&block, txns.clone());

        // Must be deterministic.
        assert_eq!(ordered1.len(), ordered2.len());
        for (a, b) in ordered1.iter().zip(ordered2.iter()) {
            assert_eq!(a.tx_hash, b.tx_hash);
        }
    }
}
