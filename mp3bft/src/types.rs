use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha256};


// ---------------------------------------------------------------------------
// Cryptographic primitives (simplified — no real Ed25519 for demo)
// ---------------------------------------------------------------------------

pub type NodeId = u64;
pub type Round = u64;
pub type Slot = u64;
pub type View = u64;
pub type Height = u64;
pub type BucketId = u64;
pub type WorkerId = u64;

/// 32-byte digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DigestBytes(pub [u8; 32]);

impl DigestBytes {
    pub fn zero() -> Self {
        DigestBytes([0u8; 32])
    }

    /// Compute SHA-256 of arbitrary bytes.
    pub fn hash(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        DigestBytes(bytes)
    }

    /// Compute deterministic hash from structured data.
    pub fn hash_fields(fields: &[&[u8]]) -> Self {
        let mut hasher = Sha256::new();
        for f in fields {
            hasher.update(f);
        }
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        DigestBytes(bytes)
    }
}

/// Simplified signature (just node_id + digest for simulation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub signer: NodeId,
    pub digest: DigestBytes,
}

impl Signature {
    pub fn sign(node_id: NodeId, data: &[u8]) -> Self {
        Signature {
            signer: node_id,
            digest: DigestBytes::hash(data),
        }
    }
}

// ---------------------------------------------------------------------------
// Data Plane types
// ---------------------------------------------------------------------------

/// A batch of raw transactions (opaque bytes for consensus layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    pub id: DigestBytes,
    pub worker_id: WorkerId,
    pub transactions: Vec<Vec<u8>>,
}

impl Batch {
    pub fn new(worker_id: WorkerId, transactions: Vec<Vec<u8>>) -> Self {
        let mut data = Vec::new();
        data.extend_from_slice(&worker_id.to_le_bytes());
        for tx in &transactions {
            data.extend_from_slice(tx);
        }
        let id = DigestBytes::hash(&data);
        Batch {
            id,
            worker_id,
            transactions,
        }
    }

    pub fn num_transactions(&self) -> usize {
        self.transactions.len()
    }
}

/// Availability certificate — 2f+1 nodes confirm they have the batch data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailCert {
    pub batch_digest: DigestBytes,
    pub worker_id: WorkerId,
    pub signatures: Vec<Signature>,
}

impl AvailCert {
    pub fn digest(&self) -> DigestBytes {
        DigestBytes::hash_fields(&[
            &self.batch_digest.0,
            &self.worker_id.to_le_bytes(),
        ])
    }

    pub fn verify(&self, quorum_threshold: usize) -> bool {
        self.signatures.len() >= quorum_threshold
    }
}

// ---------------------------------------------------------------------------
// Control Plane — Slot-Level types
// ---------------------------------------------------------------------------

/// Blocklet proposal from a slot proposer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockletProposal {
    pub view: View,
    pub slot: Slot,
    pub proposer: NodeId,
    /// AvailCert digests referenced by this proposal.
    pub avail_certs: Vec<DigestBytes>,
    /// Bucket range assigned to this proposer for anti-duplication.
    pub bucket_range: BucketRange,
    /// Digest of the proposal.
    pub digest: DigestBytes,
}

impl BlockletProposal {
    pub fn compute_digest(view: View, slot: Slot, proposer: NodeId, certs: &[DigestBytes]) -> DigestBytes {
        let mut data = Vec::new();
        data.extend_from_slice(&view.to_le_bytes());
        data.extend_from_slice(&slot.to_le_bytes());
        data.extend_from_slice(&proposer.to_le_bytes());
        for c in certs {
            data.extend_from_slice(&c.0);
        }
        DigestBytes::hash(&data)
    }
}

/// Bucket range for anti-duplication (Algorithm 3.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketRange {
    pub start: BucketId,
    pub end: BucketId, // exclusive
}

impl BucketRange {
    pub fn contains(&self, bucket: BucketId) -> bool {
        bucket >= self.start && bucket < self.end
    }
}

/// Vote on a blocklet proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotVote {
    pub view: View,
    pub slot: Slot,
    pub proposal_digest: DigestBytes,
    pub voter: NodeId,
    pub signature: Signature,
}

/// Slot-level quorum certificate — aggregation of 2f+1 SlotVotes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotQC {
    pub view: View,
    pub slot: Slot,
    pub proposal_digest: DigestBytes,
    pub proposal: BlockletProposal,
    pub votes: Vec<SlotVote>,
}

impl SlotQC {
    pub fn verify(&self, quorum_threshold: usize) -> bool {
        if self.votes.len() < quorum_threshold {
            return false;
        }
        // All votes must reference the same proposal.
        self.votes
            .iter()
            .all(|v| v.proposal_digest == self.proposal_digest && v.view == self.view && v.slot == self.slot)
    }

    pub fn digest(&self) -> DigestBytes {
        DigestBytes::hash_fields(&[
            &self.view.to_le_bytes(),
            &self.slot.to_le_bytes(),
            &self.proposal_digest.0,
        ])
    }
}

// ---------------------------------------------------------------------------
// Control Plane — Macro-Block types
// ---------------------------------------------------------------------------

/// Entry in a macro-block header referencing a slot's QC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotEntry {
    pub slot: Slot,
    pub slot_qc_digest: DigestBytes,
}

/// Macro-block header — proposed by the macro leader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroHeader {
    pub view: View,
    pub height: Height,
    pub leader: NodeId,
    pub parent_qc: Option<DigestBytes>,
    pub slot_entries: Vec<SlotEntry>,
    pub digest: DigestBytes,
}

impl MacroHeader {
    pub fn compute_digest(
        view: View,
        height: Height,
        leader: NodeId,
        parent_qc: Option<DigestBytes>,
        entries: &[SlotEntry],
    ) -> DigestBytes {
        let mut data = Vec::new();
        data.extend_from_slice(&view.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(&leader.to_le_bytes());
        if let Some(pqc) = parent_qc {
            data.extend_from_slice(&pqc.0);
        }
        for e in entries {
            data.extend_from_slice(&e.slot.to_le_bytes());
            data.extend_from_slice(&e.slot_qc_digest.0);
        }
        DigestBytes::hash(&data)
    }
}

/// Vote on a macro-block header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroVote {
    pub view: View,
    pub height: Height,
    pub header_digest: DigestBytes,
    pub voter: NodeId,
    pub signature: Signature,
}

/// Macro-block quorum certificate — aggregation of 2f+1 MacroVotes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroQC {
    pub view: View,
    pub height: Height,
    pub header_digest: DigestBytes,
    pub header: MacroHeader,
    pub votes: Vec<MacroVote>,
}

impl MacroQC {
    pub fn verify(&self, quorum_threshold: usize) -> bool {
        if self.votes.len() < quorum_threshold {
            return false;
        }
        self.votes
            .iter()
            .all(|v| v.header_digest == self.header_digest && v.view == self.view && v.height == self.height)
    }

    pub fn digest(&self) -> DigestBytes {
        DigestBytes::hash_fields(&[
            &self.view.to_le_bytes(),
            &self.height.to_le_bytes(),
            &self.header_digest.0,
        ])
    }
}

// ---------------------------------------------------------------------------
// View Change types
// ---------------------------------------------------------------------------

/// NEW_VIEW message sent on timeout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewViewMessage {
    pub new_view: View,
    pub sender: NodeId,
    pub high_qc: Option<MacroQC>,
    pub signature: Signature,
}

// ---------------------------------------------------------------------------
// Committed block
// ---------------------------------------------------------------------------

/// A committed macro-block with its 3-chain proof.
#[derive(Debug, Clone)]
pub struct CommittedBlock {
    pub height: Height,
    pub view: View,
    pub header: MacroHeader,
    pub qc: MacroQC,
    /// All slot QCs included in this block.
    pub slot_qcs: Vec<SlotQC>,
}
