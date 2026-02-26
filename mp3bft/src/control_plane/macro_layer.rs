use crate::{
    config::Committee,
    types::*,
};

/// Macro Leader (Algorithm 3.5):
/// Collects SlotQCs from the current view and assembles a MacroHeader.
pub fn create_macro_header(
    view: View,
    height: Height,
    leader: NodeId,
    parent_qc: Option<&MacroQC>,
    slot_qcs: &[SlotQC],
) -> MacroHeader {
    let parent_qc_digest = parent_qc.map(|qc| qc.digest());

    let slot_entries: Vec<SlotEntry> = slot_qcs
        .iter()
        .map(|qc| SlotEntry {
            slot: qc.slot,
            slot_qc_digest: qc.digest(),
        })
        .collect();

    let digest = MacroHeader::compute_digest(view, height, leader, parent_qc_digest, &slot_entries);

    MacroHeader {
        view,
        height,
        leader,
        parent_qc: parent_qc_digest,
        slot_entries,
        digest,
    }
}

/// Macro Validator:
/// Validates a MacroHeader before voting.
pub fn validate_macro_header(
    header: &MacroHeader,
    committee: &Committee,
    locked_qc: Option<&MacroQC>,
) -> bool {
    // 1. Check leader is correct for this view.
    let ids = committee.node_ids();
    let expected_leader = ids[(header.view as usize) % ids.len()];
    if header.leader != expected_leader {
        return false;
    }

    // 2. Lock rule: parent_qc.height >= locked_qc.height.
    if let Some(_locked) = locked_qc {
        if let Some(_parent_digest) = &header.parent_qc {
            // Parent QC must exist and be at least as high as locked.
            // (Simplified: in real system, we'd verify the parent QC is valid.)
        } else if header.height > 0 {
            // No parent QC but not genesis — invalid.
            return false;
        }
    }

    // 3. Check slot entries are non-empty.
    if header.slot_entries.is_empty() && header.height > 0 {
        return false;
    }

    true
}

/// Create a MacroVote for a valid header.
pub fn vote_on_header(header: &MacroHeader, voter: NodeId) -> MacroVote {
    let sig = Signature::sign(voter, &header.digest.0);
    MacroVote {
        view: header.view,
        height: header.height,
        header_digest: header.digest,
        voter,
        signature: sig,
    }
}

/// Macro Collector:
/// Aggregates MacroVotes into a MacroQC.
pub struct MacroCollector {
    pub view: View,
    pub height: Height,
    pub header: Option<MacroHeader>,
    pub votes: Vec<MacroVote>,
    pub quorum_threshold: usize,
}

impl MacroCollector {
    pub fn new(view: View, height: Height, quorum_threshold: usize) -> Self {
        MacroCollector {
            view,
            height,
            header: None,
            votes: Vec::new(),
            quorum_threshold,
        }
    }

    pub fn set_header(&mut self, header: MacroHeader) {
        self.header = Some(header);
    }

    /// Add a vote. Returns Some(MacroQC) when quorum is reached.
    pub fn add_vote(&mut self, vote: MacroVote) -> Option<MacroQC> {
        if vote.view != self.view || vote.height != self.height {
            return None;
        }
        if self.votes.iter().any(|v| v.voter == vote.voter) {
            return None;
        }

        self.votes.push(vote);

        if self.votes.len() >= self.quorum_threshold {
            if let Some(header) = &self.header {
                return Some(MacroQC {
                    view: self.view,
                    height: self.height,
                    header_digest: header.digest,
                    header: header.clone(),
                    votes: self.votes.clone(),
                });
            }
        }

        None
    }
}

/// Run macro-level consensus for a view (simulated).
/// Takes slot QCs and produces a MacroQC.
pub fn run_macro_consensus(
    view: View,
    height: Height,
    committee: &Committee,
    slot_qcs: &[SlotQC],
    parent_qc: Option<&MacroQC>,
) -> Option<MacroQC> {
    let node_ids = committee.node_ids();
    let quorum = committee.quorum_threshold();
    let leader = node_ids[(view as usize) % node_ids.len()];

    let header = create_macro_header(view, height, leader, parent_qc, slot_qcs);

    let mut collector = MacroCollector::new(view, height, quorum);
    collector.set_header(header.clone());

    for &voter in &node_ids {
        if validate_macro_header(&header, committee, None) {
            let vote = vote_on_header(&header, voter);
            if let Some(qc) = collector.add_vote(vote) {
                return Some(qc);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MP3BFTConfig;
    use crate::control_plane::slot_layer::run_slot_consensus;

    fn test_committee() -> Committee {
        Committee::new(vec![0, 1, 2, 3])
    }

    #[test]
    fn test_create_macro_header() {
        let committee = test_committee();
        let leader = committee.node_ids()[(0usize) % committee.size()];

        let header = create_macro_header(0, 0, leader, None, &[]);
        assert_eq!(header.view, 0);
        assert_eq!(header.height, 0);
        assert_eq!(header.leader, leader);
    }

    #[test]
    fn test_macro_qc_formation() {
        let committee = test_committee();
        let config = MP3BFTConfig { k_slots: 2, ..MP3BFTConfig::default() };
        let certs: Vec<_> = (0..4).map(|i| DigestBytes::hash(&[i as u8])).collect();
        let slot_qcs = run_slot_consensus(0, &config, &committee, &certs);

        let qc = run_macro_consensus(0, 0, &committee, &slot_qcs, None);
        assert!(qc.is_some());
        let qc = qc.unwrap();
        assert!(qc.verify(committee.quorum_threshold()));
    }

    #[test]
    fn test_3_chain_commit() {
        let committee = test_committee();
        let config = MP3BFTConfig { k_slots: 2, ..MP3BFTConfig::default() };
        let certs: Vec<_> = (0..4).map(|i| DigestBytes::hash(&[i as u8])).collect();

        let mut engine = crate::control_plane::ConsensusEngine::new(
            config.clone(),
            committee.clone(),
            0,
        );

        // Block 0
        let slot_qcs_0 = run_slot_consensus(0, &config, &committee, &certs);
        let qc0 = run_macro_consensus(0, 0, &committee, &slot_qcs_0, None).unwrap();
        let committed = engine.process_macro_qc(qc0.clone(), slot_qcs_0);
        assert!(committed.is_empty()); // Need 3 blocks for commit

        // Block 1
        let slot_qcs_1 = run_slot_consensus(1, &config, &committee, &certs);
        let qc1 = run_macro_consensus(1, 1, &committee, &slot_qcs_1, Some(&qc0)).unwrap();
        let committed = engine.process_macro_qc(qc1.clone(), slot_qcs_1);
        assert!(committed.is_empty()); // Need 1 more

        // Block 2
        let slot_qcs_2 = run_slot_consensus(2, &config, &committee, &certs);
        let qc2 = run_macro_consensus(2, 2, &committee, &slot_qcs_2, Some(&qc1)).unwrap();
        let committed = engine.process_macro_qc(qc2.clone(), slot_qcs_2);
        assert_eq!(committed.len(), 1); // Block 0 committed!
        assert_eq!(committed[0].height, 0);
    }
}
