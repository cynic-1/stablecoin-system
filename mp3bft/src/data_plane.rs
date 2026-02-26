use crate::{
    config::Committee,
    types::{AvailCert, Batch, DigestBytes, Signature, WorkerId},
};
use std::collections::HashMap;

/// Simulated data plane that mirrors Narwhal's worker layer.
///
/// In the real system, this would use network communication.
/// Here we simulate it for benchmarking purposes.
pub struct DataPlane {
    committee: Committee,
    /// Stored batches by digest.
    batches: HashMap<DigestBytes, Batch>,
    /// AvailCerts formed so far.
    avail_certs: Vec<AvailCert>,
}

impl DataPlane {
    pub fn new(committee: Committee) -> Self {
        DataPlane {
            committee,
            batches: HashMap::new(),
            avail_certs: Vec::new(),
        }
    }

    /// Submit a batch of transactions. In the real system, the worker broadcasts
    /// to peers and collects 2f+1 acknowledgements. Here we simulate instant
    /// availability certification.
    pub fn submit_batch(&mut self, worker_id: WorkerId, transactions: Vec<Vec<u8>>) -> AvailCert {
        let batch = Batch::new(worker_id, transactions);
        let digest = batch.id;
        self.batches.insert(digest, batch);

        // Simulate 2f+1 signatures from committee members.
        let signatures: Vec<Signature> = self
            .committee
            .node_ids()
            .iter()
            .take(self.committee.quorum_threshold())
            .map(|&node_id| Signature::sign(node_id, &digest.0))
            .collect();

        let ac = AvailCert {
            batch_digest: digest,
            worker_id,
            signatures,
        };
        self.avail_certs.push(ac.clone());
        ac
    }

    /// Retrieve a batch by its digest.
    pub fn get_batch(&self, digest: &DigestBytes) -> Option<&Batch> {
        self.batches.get(digest)
    }

    /// Get all available AvailCerts.
    pub fn available_certs(&self) -> &[AvailCert] {
        &self.avail_certs
    }

    /// Drain available certs (consume up to `limit`).
    pub fn take_certs(&mut self, limit: usize) -> Vec<AvailCert> {
        let n = std::cmp::min(limit, self.avail_certs.len());
        self.avail_certs.drain(..n).collect()
    }

    /// Retrieve transactions from a set of AvailCert digests.
    pub fn get_transactions(&self, cert_digests: &[DigestBytes]) -> Vec<Vec<u8>> {
        let mut txns = Vec::new();
        for digest in cert_digests {
            // Find the batch referenced by any cert with this digest.
            for cert in &self.avail_certs {
                if cert.digest() == *digest || cert.batch_digest == *digest {
                    if let Some(batch) = self.batches.get(&cert.batch_digest) {
                        txns.extend(batch.transactions.clone());
                    }
                }
            }
            // Also try direct batch lookup.
            if let Some(batch) = self.batches.get(digest) {
                txns.extend(batch.transactions.clone());
            }
        }
        txns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_and_retrieve() {
        let committee = Committee::new(vec![0, 1, 2, 3]);
        let mut dp = DataPlane::new(committee);

        let txns = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let cert = dp.submit_batch(0, txns.clone());

        assert!(cert.verify(3)); // 2f+1 = 3 for n=4
        let batch = dp.get_batch(&cert.batch_digest).unwrap();
        assert_eq!(batch.num_transactions(), 2);
    }

    #[test]
    fn test_take_certs() {
        let committee = Committee::new(vec![0, 1, 2, 3]);
        let mut dp = DataPlane::new(committee);

        for i in 0..5 {
            dp.submit_batch(0, vec![vec![i as u8]]);
        }

        let certs = dp.take_certs(3);
        assert_eq!(certs.len(), 3);
        assert_eq!(dp.available_certs().len(), 2);
    }
}
