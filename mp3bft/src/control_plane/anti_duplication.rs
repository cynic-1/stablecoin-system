use crate::types::{BlockletProposal, BucketRange, DigestBytes, View};
use std::collections::HashSet;

/// Assign bucket ranges to k slots for a given view (Algorithm 3.1).
///
/// Evenly divides n_buckets across k_slots, rotating by view to distribute load.
pub fn assign_buckets(view: View, n_buckets: usize, k_slots: usize) -> Vec<BucketRange> {
    let buckets_per_slot = n_buckets / k_slots;
    let offset = ((view as usize) * buckets_per_slot) % n_buckets;

    (0..k_slots)
        .map(|s| {
            let start = (offset + s * buckets_per_slot) % n_buckets;
            let end = start + buckets_per_slot;
            BucketRange {
                start: start as u64,
                end: end as u64,
            }
        })
        .collect()
}

/// Check bucket compliance: all AvailCerts in the proposal must reference
/// transactions whose bucket falls within the assigned range.
///
/// In the real system, we'd hash each transaction to a bucket.
/// Here we check that the proposal's bucket_range matches the assigned range.
pub fn check_bucket_compliance(
    proposal: &BlockletProposal,
    assigned_range: &BucketRange,
    _n_buckets: usize,
) -> bool {
    proposal.bucket_range == *assigned_range
}

/// Check intra-slot uniqueness: no duplicate (sender, nonce) within a proposal.
///
/// This is a simplified check — in the real system, we'd inspect actual
/// transaction contents. Here we check that all AvailCert digests are unique.
pub fn check_intra_slot_unique(proposal: &BlockletProposal) -> bool {
    let mut seen = HashSet::new();
    for cert_digest in &proposal.avail_certs {
        if !seen.insert(cert_digest) {
            return false;
        }
    }
    true
}

/// Compute the bucket for a transaction hash.
pub fn tx_bucket(tx_hash: &[u8], n_buckets: usize) -> u64 {
    let d = DigestBytes::hash(tx_hash);
    let val = u64::from_le_bytes(d.0[0..8].try_into().unwrap());
    val % (n_buckets as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bucket_assignment() {
        let ranges = assign_buckets(0, 1024, 4);
        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[0].end, 256);
        assert_eq!(ranges[1].start, 256);
        assert_eq!(ranges[1].end, 512);
    }

    #[test]
    fn test_bucket_rotation_across_views() {
        let r0 = assign_buckets(0, 1024, 4);
        let r1 = assign_buckets(1, 1024, 4);
        // Different views should produce different assignments.
        assert_ne!(r0[0].start, r1[0].start);
    }

    #[test]
    fn test_bucket_compliance() {
        let ranges = assign_buckets(0, 1024, 4);
        let proposal = BlockletProposal {
            view: 0,
            slot: 0,
            proposer: 0,
            avail_certs: vec![],
            bucket_range: ranges[0].clone(),
            digest: DigestBytes::zero(),
        };
        assert!(check_bucket_compliance(&proposal, &ranges[0], 1024));
        assert!(!check_bucket_compliance(&proposal, &ranges[1], 1024));
    }

    #[test]
    fn test_intra_slot_unique() {
        let d1 = DigestBytes::hash(b"cert1");
        let d2 = DigestBytes::hash(b"cert2");
        let d3 = d1; // duplicate

        let proposal_ok = BlockletProposal {
            view: 0, slot: 0, proposer: 0,
            avail_certs: vec![d1, d2],
            bucket_range: BucketRange { start: 0, end: 256 },
            digest: DigestBytes::zero(),
        };
        assert!(check_intra_slot_unique(&proposal_ok));

        let proposal_dup = BlockletProposal {
            view: 0, slot: 0, proposer: 0,
            avail_certs: vec![d1, d3],
            bucket_range: BucketRange { start: 0, end: 256 },
            digest: DigestBytes::zero(),
        };
        assert!(!check_intra_slot_unique(&proposal_dup));
    }
}
