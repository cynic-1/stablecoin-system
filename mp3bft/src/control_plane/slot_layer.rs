use crate::{
    config::{Committee, MP3BFTConfig},
    control_plane::anti_duplication::{assign_buckets, check_bucket_compliance, check_intra_slot_unique},
    types::*,
};


/// Slot Proposer (Algorithm 3.2):
/// Creates a BlockletProposal for the assigned slot.
pub fn create_proposal(
    view: View,
    slot: Slot,
    proposer: NodeId,
    avail_certs: Vec<DigestBytes>,
    config: &MP3BFTConfig,
) -> BlockletProposal {
    let ranges = assign_buckets(view, config.n_buckets, config.k_slots);
    let bucket_range = ranges[slot as usize % ranges.len()].clone();

    // Limit to m_max certs.
    let certs: Vec<DigestBytes> = avail_certs.into_iter().take(config.m_max).collect();

    let digest = BlockletProposal::compute_digest(view, slot, proposer, &certs);

    BlockletProposal {
        view,
        slot,
        proposer,
        avail_certs: certs,
        bucket_range,
        digest,
    }
}

/// Slot Validator (Algorithm 3.3):
/// Validates a proposal before voting.
pub fn validate_proposal(
    proposal: &BlockletProposal,
    config: &MP3BFTConfig,
    committee: &Committee,
) -> bool {
    // 1. Check proposer is the correct slot proposer.
    let expected_proposer = {
        let ids = committee.node_ids();
        let data = [proposal.view.to_le_bytes(), proposal.slot.to_le_bytes()].concat();
        let d = DigestBytes::hash(&data);
        let h = u64::from_le_bytes(d.0[0..8].try_into().unwrap());
        ids[(h as usize) % ids.len()]
    };
    if proposal.proposer != expected_proposer {
        return false;
    }

    // 2. Check bucket compliance.
    let ranges = assign_buckets(proposal.view, config.n_buckets, config.k_slots);
    let assigned = &ranges[proposal.slot as usize % ranges.len()];
    if !check_bucket_compliance(proposal, assigned, config.n_buckets) {
        return false;
    }

    // 3. Check intra-slot uniqueness.
    if !check_intra_slot_unique(proposal) {
        return false;
    }

    // 4. Check cert count limit.
    if proposal.avail_certs.len() > config.m_max {
        return false;
    }

    true
}

/// Create a SlotVote for a valid proposal.
pub fn vote_on_proposal(proposal: &BlockletProposal, voter: NodeId) -> SlotVote {
    let sig = Signature::sign(voter, &proposal.digest.0);
    SlotVote {
        view: proposal.view,
        slot: proposal.slot,
        proposal_digest: proposal.digest,
        voter,
        signature: sig,
    }
}

/// Slot Collector (Algorithm 3.4):
/// Aggregates votes into a SlotQC when quorum is reached.
pub struct SlotCollector {
    pub view: View,
    pub slot: Slot,
    pub proposal: Option<BlockletProposal>,
    pub votes: Vec<SlotVote>,
    pub quorum_threshold: usize,
}

impl SlotCollector {
    pub fn new(view: View, slot: Slot, quorum_threshold: usize) -> Self {
        SlotCollector {
            view,
            slot,
            proposal: None,
            votes: Vec::new(),
            quorum_threshold,
        }
    }

    pub fn set_proposal(&mut self, proposal: BlockletProposal) {
        self.proposal = Some(proposal);
    }

    /// Add a vote. Returns Some(SlotQC) when quorum is reached.
    pub fn add_vote(&mut self, vote: SlotVote) -> Option<SlotQC> {
        if vote.view != self.view || vote.slot != self.slot {
            return None;
        }

        // Check this voter hasn't already voted.
        if self.votes.iter().any(|v| v.voter == vote.voter) {
            return None;
        }

        self.votes.push(vote);

        if self.votes.len() >= self.quorum_threshold {
            if let Some(proposal) = &self.proposal {
                return Some(SlotQC {
                    view: self.view,
                    slot: self.slot,
                    proposal_digest: proposal.digest,
                    proposal: proposal.clone(),
                    votes: self.votes.clone(),
                });
            }
        }

        None
    }
}

/// Run slot-level consensus for all k slots in parallel (simulated).
/// Returns a vector of SlotQCs, one per slot.
pub fn run_slot_consensus(
    view: View,
    config: &MP3BFTConfig,
    committee: &Committee,
    available_certs: &[DigestBytes],
) -> Vec<SlotQC> {
    let node_ids = committee.node_ids();
    let quorum = committee.quorum_threshold();
    let mut slot_qcs = Vec::new();

    // Distribute certs evenly across slots.
    let certs_per_slot = if config.k_slots > 0 {
        (available_certs.len() + config.k_slots - 1) / config.k_slots
    } else {
        available_certs.len()
    };

    for slot in 0..config.k_slots {
        let start = slot * certs_per_slot;
        let end = std::cmp::min(start + certs_per_slot, available_certs.len());
        let slot_certs: Vec<DigestBytes> = if start < available_certs.len() {
            available_certs[start..end].to_vec()
        } else {
            vec![]
        };

        let proposer_id = {
            let data = [view.to_le_bytes(), (slot as u64).to_le_bytes()].concat();
            let d = DigestBytes::hash(&data);
            let h = u64::from_le_bytes(d.0[0..8].try_into().unwrap());
            node_ids[(h as usize) % node_ids.len()]
        };

        // Create proposal.
        let proposal = create_proposal(view, slot as Slot, proposer_id, slot_certs, config);

        // Validate and collect votes.
        let mut collector = SlotCollector::new(view, slot as Slot, quorum);
        collector.set_proposal(proposal.clone());

        for &voter_id in &node_ids {
            if validate_proposal(&proposal, config, committee) {
                let vote = vote_on_proposal(&proposal, voter_id);
                if let Some(qc) = collector.add_vote(vote) {
                    slot_qcs.push(qc);
                    break;
                }
            }
        }
    }

    slot_qcs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_committee() -> Committee {
        Committee::new(vec![0, 1, 2, 3])
    }

    #[test]
    fn test_create_and_validate_proposal() {
        let config = MP3BFTConfig::default();
        let committee = test_committee();
        let certs = vec![DigestBytes::hash(b"cert1"), DigestBytes::hash(b"cert2")];

        let proposer = {
            let ids = committee.node_ids();
            let data = [0u64.to_le_bytes(), 0u64.to_le_bytes()].concat();
            let d = DigestBytes::hash(&data);
            let h = u64::from_le_bytes(d.0[0..8].try_into().unwrap());
            ids[(h as usize) % ids.len()]
        };

        let proposal = create_proposal(0, 0, proposer, certs, &config);
        assert!(validate_proposal(&proposal, &config, &committee));
    }

    #[test]
    fn test_slot_collector_quorum() {
        let committee = test_committee();
        let quorum = committee.quorum_threshold(); // 3
        let config = MP3BFTConfig::default();

        let proposer = 0;
        let proposal = create_proposal(0, 0, proposer, vec![], &config);

        let mut collector = SlotCollector::new(0, 0, quorum);
        collector.set_proposal(proposal.clone());

        // Add votes from nodes 0, 1, 2 (quorum = 3).
        assert!(collector.add_vote(vote_on_proposal(&proposal, 0)).is_none());
        assert!(collector.add_vote(vote_on_proposal(&proposal, 1)).is_none());
        let qc = collector.add_vote(vote_on_proposal(&proposal, 2));
        assert!(qc.is_some());

        let qc = qc.unwrap();
        assert!(qc.verify(quorum));
    }

    #[test]
    fn test_run_slot_consensus() {
        let config = MP3BFTConfig { k_slots: 4, ..MP3BFTConfig::default() };
        let committee = test_committee();
        let certs: Vec<_> = (0..8).map(|i| DigestBytes::hash(&[i as u8])).collect();

        let qcs = run_slot_consensus(0, &config, &committee, &certs);
        assert_eq!(qcs.len(), 4); // One QC per slot
    }

    #[test]
    fn test_duplicate_vote_rejected() {
        let committee = test_committee();
        let config = MP3BFTConfig::default();
        let proposal = create_proposal(0, 0, 0, vec![], &config);

        let mut collector = SlotCollector::new(0, 0, committee.quorum_threshold());
        collector.set_proposal(proposal.clone());

        let vote = vote_on_proposal(&proposal, 0);
        assert!(collector.add_vote(vote.clone()).is_none());
        // Duplicate vote should be rejected.
        assert!(collector.add_vote(vote).is_none());
        assert_eq!(collector.votes.len(), 1);
    }
}
