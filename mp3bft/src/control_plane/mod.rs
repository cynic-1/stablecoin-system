pub mod anti_duplication;
pub mod slot_layer;
pub mod macro_layer;
pub mod view_change;

use crate::{
    config::{Committee, MP3BFTConfig},
    types::*,
};

/// The full MP3-BFT++ consensus engine.
pub struct ConsensusEngine {
    pub config: MP3BFTConfig,
    pub committee: Committee,
    pub node_id: NodeId,

    // State
    pub current_view: View,
    pub current_height: Height,
    pub locked_qc: Option<MacroQC>,
    pub high_qc: Option<MacroQC>,

    // Committed blocks
    pub committed: Vec<CommittedBlock>,

    // Pending QCs for 3-chain rule
    pub macro_qc_chain: Vec<MacroQC>,
}

impl ConsensusEngine {
    pub fn new(
        config: MP3BFTConfig,
        committee: Committee,
        node_id: NodeId,
    ) -> Self {
        ConsensusEngine {
            config,
            committee,
            node_id,
            current_view: 0,
            current_height: 0,
            locked_qc: None,
            high_qc: None,
            committed: Vec::new(),
            macro_qc_chain: Vec::new(),
        }
    }

    /// Determine if this node is the macro leader for the current view.
    pub fn is_macro_leader(&self) -> bool {
        self.macro_leader(self.current_view) == self.node_id
    }

    /// Compute macro leader for a view (round-robin).
    pub fn macro_leader(&self, view: View) -> NodeId {
        let ids = self.committee.node_ids();
        ids[(view as usize) % ids.len()]
    }

    /// Compute slot proposer for (view, slot).
    pub fn slot_proposer(&self, view: View, slot: Slot) -> NodeId {
        let ids = self.committee.node_ids();
        let h = {
            let data = [view.to_le_bytes(), slot.to_le_bytes()].concat();
            let d = DigestBytes::hash(&data);
            u64::from_le_bytes(d.0[0..8].try_into().unwrap())
        };
        ids[(h as usize) % ids.len()]
    }

    /// Process a MacroQC and check the 3-chain commit rule.
    /// Returns newly committed blocks if the rule is satisfied.
    pub fn process_macro_qc(&mut self, qc: MacroQC, slot_qcs: Vec<SlotQC>) -> Vec<CommittedBlock> {
        // Update high_qc.
        let should_update = match &self.high_qc {
            None => true,
            Some(existing) => qc.height > existing.height,
        };
        if should_update {
            self.high_qc = Some(qc.clone());
        }

        // Add to chain.
        self.macro_qc_chain.push(qc.clone());

        // Check 3-chain commit rule:
        // If we have QC(h), QC(h-1), QC(h-2) where each extends the previous,
        // commit block at height h-2.
        let mut committed_blocks = Vec::new();

        if self.macro_qc_chain.len() >= 3 {
            let len = self.macro_qc_chain.len();
            let qc_h = &self.macro_qc_chain[len - 1];
            let qc_h1 = &self.macro_qc_chain[len - 2];
            let qc_h2 = &self.macro_qc_chain[len - 3];

            // Check consecutive heights.
            if qc_h.height == qc_h1.height + 1
                && qc_h1.height == qc_h2.height + 1
            {
                // Check parent links.
                let h_links_h1 = qc_h.header.parent_qc == Some(qc_h1.digest());
                let h1_links_h2 = qc_h1.header.parent_qc == Some(qc_h2.digest());

                if h_links_h1 && h1_links_h2 {
                    // 3-chain satisfied — commit qc_h2.
                    let block = CommittedBlock {
                        height: qc_h2.height,
                        view: qc_h2.view,
                        header: qc_h2.header.clone(),
                        qc: qc_h2.clone(),
                        slot_qcs: slot_qcs.clone(),
                    };
                    committed_blocks.push(block.clone());
                    self.committed.push(block);

                    // Update locked_qc to qc_h1.
                    self.locked_qc = Some(qc_h1.clone());
                    self.current_height = qc_h2.height + 1;
                }
            }
        }

        committed_blocks
    }
}
