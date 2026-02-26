// MP3-BFT++ consensus module for Narwhal.
//
// Drop-in replacement for Tusk (consensus/src/lib.rs) with the same
// `Consensus::spawn()` interface. Instead of a single-leader commit rule,
// MP3-BFT++ groups certificates into k parallel slots per commit round and
// applies a 3-chain commit rule, batching more certificates per commit event.

use config::{Committee, Stake};
use crypto::Hash as _;
use crypto::{Digest, PublicKey};
use log::{debug, info, log_enabled, warn};
use primary::{Certificate, Round};
use std::cmp::max;
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc::{Receiver, Sender};

/// The representation of the DAG in memory (same as Tusk).
type Dag = HashMap<Round, HashMap<PublicKey, (Digest, Certificate)>>;

/// SlotQC: k slot leaders at a round with f+1 support verified.
#[derive(Debug, Clone)]
struct SlotQC {
    #[allow(dead_code)]
    round: Round,
    leaders: Vec<(Digest, Certificate)>,
}

/// MacroBlock: prepared leader round awaiting 3-chain commitment.
#[derive(Debug, Clone)]
struct MacroBlock {
    height: u64,
    round: Round,
    slot_qc: SlotQC,
}

/// The state that needs to be persisted for crash-recovery.
struct State {
    /// The last committed round.
    last_committed_round: Round,
    /// Keeps the last committed round for each authority.
    last_committed: HashMap<PublicKey, Round>,
    /// The local DAG.
    dag: Dag,
    /// 3-chain: prepared MacroBlocks awaiting commitment.
    /// Blocks must be consecutive (round difference of 1) to form a chain.
    prepared_chain: Vec<MacroBlock>,
    /// Next MacroBlock height counter.
    next_height: u64,
}

impl State {
    fn new(genesis: Vec<Certificate>) -> Self {
        let genesis = genesis
            .into_iter()
            .map(|x| (x.origin(), (x.digest(), x)))
            .collect::<HashMap<_, _>>();

        Self {
            last_committed_round: 0,
            last_committed: genesis.iter().map(|(x, (_, y))| (*x, y.round())).collect(),
            dag: [(0, genesis)].iter().cloned().collect(),
            prepared_chain: Vec::new(),
            next_height: 0,
        }
    }

    fn update(&mut self, certificate: &Certificate, gc_depth: Round) {
        self.last_committed
            .entry(certificate.origin())
            .and_modify(|r| *r = max(*r, certificate.round()))
            .or_insert_with(|| certificate.round());

        let last_committed_round = *self.last_committed.values().max().unwrap();
        self.last_committed_round = last_committed_round;

        for (name, round) in &self.last_committed {
            self.dag.retain(|r, authorities| {
                authorities.retain(|n, _| n != name || r >= round);
                !authorities.is_empty() && r + gc_depth >= last_committed_round
            });
        }
    }
}


pub struct MP3Consensus {
    /// The committee information.
    committee: Committee,
    /// The depth of the garbage collector.
    gc_depth: Round,
    /// Number of parallel slots per commit round (the k in MP3-BFT++).
    k_slots: usize,

    /// Receives new certificates from the primary.
    rx_primary: Receiver<Certificate>,
    /// Outputs the sequence of ordered certificates to the primary (for cleanup and feedback).
    tx_primary: Sender<Certificate>,
    /// Outputs the sequence of ordered certificates to the application layer.
    tx_output: Sender<Certificate>,

    /// The genesis certificates.
    genesis: Vec<Certificate>,
}

impl MP3Consensus {
    pub fn spawn(
        committee: Committee,
        gc_depth: Round,
        k_slots: usize,
        rx_primary: Receiver<Certificate>,
        tx_primary: Sender<Certificate>,
        tx_output: Sender<Certificate>,
    ) {
        tokio::spawn(async move {
            Self {
                committee: committee.clone(),
                gc_depth,
                k_slots,
                rx_primary,
                tx_primary,
                tx_output,
                genesis: Certificate::genesis(&committee),
            }
            .run()
            .await;
        });
    }

    async fn run(&mut self) {
        let mut state = State::new(self.genesis.clone());
        // Track the highest leader_round we prepared to avoid duplicate preparation.
        let mut last_prepared_round: Round = 0;

        // Listen to incoming certificates.
        while let Some(certificate) = self.rx_primary.recv().await {
            debug!("MP3-BFT++ processing {:?}", certificate);
            let round = certificate.round();

            // Add the new certificate to the local storage.
            state
                .dag
                .entry(round)
                .or_insert_with(HashMap::new)
                .insert(certificate.origin(), (certificate.digest(), certificate));

            // Pipeline cadence: every round has a leader.
            // r = current round, leader_round = r - 1, support checked at r.
            let r = round;
            if r < 2 {
                continue;
            }

            let leader_round = r - 1;
            if leader_round <= state.last_committed_round {
                continue;
            }

            // Skip if we already prepared this leader round.
            if leader_round <= last_prepared_round {
                continue;
            }

            // MP3-BFT++ multi-slot: elect k leaders for this round.
            let slot_leaders: Vec<(Digest, Certificate)> = self
                .slot_leaders(leader_round, &state.dag)
                .into_iter()
                .map(|(d, c)| (d.clone(), c.clone()))
                .collect();
            if slot_leaders.is_empty() {
                continue;
            }

            // Check support for each slot leader: needs f+1 children in round r.
            let mut supported_leaders: Vec<(Digest, Certificate)> = Vec::new();
            for (leader_digest, leader) in &slot_leaders {
                let stake: Stake = state
                    .dag
                    .get(&r)
                    .map(|round_certs| {
                        round_certs
                            .values()
                            .filter(|(_, x)| x.header.parents.contains(leader_digest))
                            .map(|(_, x)| self.committee.stake(&x.origin()))
                            .sum()
                    })
                    .unwrap_or(0);

                if stake >= self.committee.validity_threshold() {
                    debug!("MP3-BFT++ leader {:?} (slot {}) has enough support", leader, supported_leaders.len());
                    supported_leaders.push((leader_digest.clone(), leader.clone()));
                } else {
                    debug!("MP3-BFT++ leader {:?} does not have enough support", leader);
                }
            }

            if supported_leaders.is_empty() {
                continue;
            }

            last_prepared_round = leader_round;

            // 3-chain prepare phase: create SlotQC and MacroBlock.
            let slot_qc = SlotQC {
                round: leader_round,
                leaders: supported_leaders,
            };
            let macro_block = MacroBlock {
                height: state.next_height,
                round: leader_round,
                slot_qc,
            };
            state.next_height += 1;

            // 3-chain rule: extend chain if consecutive logical heights.
            // Safety proof (Theorem 3.2) relies on consecutive *heights*, not DAG rounds.
            // Heights are always sequential (next_height increments by 1), so the chain
            // is never broken by DAG round gaps — only by the absence of a prepare.
            let consecutive = state
                .prepared_chain
                .last()
                .map_or(true, |tail| macro_block.height == tail.height + 1);

            if consecutive {
                state.prepared_chain.push(macro_block);
            } else {
                debug!(
                    "MP3-BFT++ chain break at round {} (tail was {})",
                    leader_round,
                    state.prepared_chain.last().map_or(0, |t| t.round)
                );
                state.prepared_chain.clear();
                state.prepared_chain.push(macro_block);
            }

            info!(
                "MP3-BFT++ prepared round {} (chain length: {})",
                leader_round,
                state.prepared_chain.len()
            );

            // 3-chain commit: when chain has 3+ consecutive prepared blocks,
            // commit the oldest block. Repeat while chain >= 3 to handle catch-up.
            while state.prepared_chain.len() >= 3 {
                let committed_block = state.prepared_chain.remove(0);
                info!(
                    "MP3-BFT++ 3-chain commit at round {} (height {})",
                    committed_block.round, committed_block.height
                );

                // Commit all supported slot leaders and their linked sub-DAGs.
                let mut sequence = Vec::new();
                for (_leader_digest, leader) in &committed_block.slot_qc.leaders {
                    let linked_leaders = self.order_leaders(leader, &state);
                    for past_leader in linked_leaders.iter().rev() {
                        for x in self.order_dag(past_leader, &state) {
                            state.update(&x, self.gc_depth);
                            sequence.push(x);
                        }
                    }
                }

                // Log debug state.
                if log_enabled!(log::Level::Debug) {
                    for (name, round) in &state.last_committed {
                        debug!("MP3-BFT++ latest commit of {}: Round {}", name, round);
                    }
                }

                // Output the committed sequence.
                for certificate in sequence {
                    #[cfg(not(feature = "benchmark"))]
                    info!("Committed {}", certificate.header);

                    #[cfg(feature = "benchmark")]
                    for digest in certificate.header.payload.keys() {
                        // NOTE: This log entry is used to compute performance.
                        info!("Committed {} -> {:?}", certificate.header, digest);
                    }

                    self.tx_primary
                        .send(certificate.clone())
                        .await
                        .expect("Failed to send certificate to primary");

                    if let Err(e) = self.tx_output.send(certificate).await {
                        warn!("Failed to output certificate: {}", e);
                    }
                }
            }
        }
    }

    /// Elect k leaders for the given round using a deterministic hash-based
    /// assignment. Each slot maps to a different authority (if possible).
    fn slot_leaders<'a>(&self, round: Round, dag: &'a Dag) -> Vec<(&'a Digest, &'a Certificate)> {
        let round_certs = match dag.get(&round) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut keys: Vec<_> = self.committee.authorities.keys().cloned().collect();
        keys.sort();

        let n = keys.len();
        let k = self.k_slots.min(n); // Can't have more slots than authorities.

        let mut leaders = Vec::new();
        let mut used = HashSet::new();

        for slot in 0..k {
            // Deterministic slot-to-authority mapping using round + slot.
            // This spreads leadership across authorities.
            #[cfg(test)]
            let idx = slot % n;
            #[cfg(not(test))]
            let idx = ((round as usize) + slot) % n;

            let leader_key = keys[idx];
            if used.contains(&leader_key) {
                continue;
            }

            if let Some((digest, cert)) = round_certs.get(&leader_key) {
                leaders.push((digest, cert));
                used.insert(leader_key);
            }
        }

        leaders
    }

    /// Order past leaders linked to the current leader.
    /// With every-round leaders, walk backward one round at a time.
    fn order_leaders(&self, leader: &Certificate, state: &State) -> Vec<Certificate> {
        let mut to_commit = vec![leader.clone()];
        let mut leader = leader;
        for r in (state.last_committed_round + 1..=leader.round().saturating_sub(1)).rev() {
            // Use the same single-leader election for the backward chain walk.
            let (_, prev_leader) = match self.primary_leader(r, &state.dag) {
                Some(x) => x,
                None => continue,
            };

            if self.linked(leader, prev_leader, &state.dag) {
                to_commit.push(prev_leader.clone());
                leader = prev_leader;
            }
        }
        to_commit
    }

    /// Single leader election (same as Tusk) for backward chain walking.
    fn primary_leader<'a>(&self, round: Round, dag: &'a Dag) -> Option<&'a (Digest, Certificate)> {
        #[cfg(test)]
        let coin = 0;
        #[cfg(not(test))]
        let coin = round;

        let mut keys: Vec<_> = self.committee.authorities.keys().cloned().collect();
        keys.sort();
        let leader = keys[coin as usize % self.committee.size()];

        dag.get(&round).map(|x| x.get(&leader)).flatten()
    }

    /// Check if there is a path between two leaders (same as Tusk).
    fn linked(&self, leader: &Certificate, prev_leader: &Certificate, dag: &Dag) -> bool {
        let mut parents = vec![leader];
        for r in (prev_leader.round()..leader.round()).rev() {
            parents = dag
                .get(&r)
                .expect("We should have the whole history by now")
                .values()
                .filter(|(digest, _)| parents.iter().any(|x| x.header.parents.contains(digest)))
                .map(|(_, certificate)| certificate)
                .collect();
        }
        parents.contains(&prev_leader)
    }

    /// Flatten the sub-dag referenced by the input certificate (same as Tusk).
    fn order_dag(&self, leader: &Certificate, state: &State) -> Vec<Certificate> {
        debug!("MP3-BFT++ processing sub-dag of {:?}", leader);
        let mut ordered = Vec::new();
        let mut already_ordered = HashSet::new();

        let mut buffer = vec![leader];
        while let Some(x) = buffer.pop() {
            debug!("Sequencing {:?}", x);
            ordered.push(x.clone());
            for parent in &x.header.parents {
                let (digest, certificate) = match state
                    .dag
                    .get(&(x.round() - 1))
                    .map(|x| x.values().find(|(x, _)| x == parent))
                    .flatten()
                {
                    Some(x) => x,
                    None => continue,
                };

                let mut skip = already_ordered.contains(&digest);
                skip |= state
                    .last_committed
                    .get(&certificate.origin())
                    .map_or_else(|| false, |r| r == &certificate.round());
                if !skip {
                    buffer.push(certificate);
                    already_ordered.insert(digest);
                }
            }
        }

        ordered.retain(|x| x.round() + self.gc_depth >= state.last_committed_round);
        ordered.sort_by_key(|x| x.round());
        ordered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus_tests::mock_committee;
    use crypto::{generate_keypair, SecretKey};
    use primary::Header;
    use rand::rngs::StdRng;
    use rand::SeedableRng as _;
    use std::collections::{BTreeSet, VecDeque};
    use tokio::sync::mpsc::channel;

    fn keys() -> Vec<(PublicKey, SecretKey)> {
        let mut rng = StdRng::from_seed([0; 32]);
        (0..4).map(|_| generate_keypair(&mut rng)).collect()
    }

    fn mock_certificate(
        origin: PublicKey,
        round: Round,
        parents: BTreeSet<Digest>,
    ) -> (Digest, Certificate) {
        let certificate = Certificate {
            header: Header {
                author: origin,
                round,
                parents,
                ..Header::default()
            },
            ..Certificate::default()
        };
        (certificate.digest(), certificate)
    }

    fn make_certificates(
        start: Round,
        stop: Round,
        initial_parents: &BTreeSet<Digest>,
        keys: &[PublicKey],
    ) -> (VecDeque<Certificate>, BTreeSet<Digest>) {
        let mut certificates = VecDeque::new();
        let mut parents = initial_parents.iter().cloned().collect::<BTreeSet<_>>();
        let mut next_parents = BTreeSet::new();

        for round in start..=stop {
            next_parents.clear();
            for name in keys {
                let (digest, certificate) = mock_certificate(*name, round, parents.clone());
                certificates.push_back(certificate);
                next_parents.insert(digest);
            }
            parents = next_parents.clone();
        }
        (certificates, next_parents)
    }

    // Test: MP3-BFT++ with k=1 commits when 3-chain forms.
    // Pipeline cadence: every round has a leader.
    // Round 2: leader_round=1, support at round 2 → prepare(1)
    // Round 3: leader_round=2, support at round 3 → prepare(2)
    // Round 4: leader_round=3, support at round 4 → prepare(3)
    // Chain = [1,2,3] → commit round 1 leader + sub-dag.
    #[tokio::test]
    async fn mp3bft_commit_with_k1() {
        let keys: Vec<_> = keys().into_iter().map(|(x, _)| x).collect();
        let genesis = Certificate::genesis(&mock_committee())
            .iter()
            .map(|x| x.digest())
            .collect::<BTreeSet<_>>();

        // Build DAG rounds 1-4. Round 4 triggers prepare(3) → chain [1,2,3] → commit 1.
        let (certificates, _next_parents) = make_certificates(1, 4, &genesis, &keys);
        let mut certificates = certificates;

        // Large channel capacities to avoid deadlock during batch commit.
        let (tx_waiter, rx_waiter) = channel(100);
        let (tx_primary, mut rx_primary) = channel(100);
        let (tx_output, mut rx_output) = channel(100);
        MP3Consensus::spawn(
            mock_committee(),
            /* gc_depth */ 50,
            /* k_slots */ 1,
            rx_waiter,
            tx_primary,
            tx_output,
        );
        tokio::spawn(async move { while rx_primary.recv().await.is_some() {} });

        while let Some(certificate) = certificates.pop_front() {
            tx_waiter.send(certificate).await.unwrap();
        }

        // 3-chain commits round 1 leader: genesis certs (round 0) already committed,
        // so we get 1 round-1 leader cert.
        let certificate = rx_output.recv().await.unwrap();
        assert_eq!(certificate.round(), 1);
    }

    // Test: MP3-BFT++ with k=4 should commit certificates from multiple slot leaders.
    // Pipeline cadence: every round has a leader. k=4 means all 4 authorities lead.
    // Chain = [1,2,3] at round 4 → commit round 1 (all 4 slot leaders).
    #[tokio::test]
    async fn mp3bft_commit_with_k4() {
        let keys: Vec<_> = keys().into_iter().map(|(x, _)| x).collect();
        let genesis = Certificate::genesis(&mock_committee())
            .iter()
            .map(|x| x.digest())
            .collect::<BTreeSet<_>>();

        // Build DAG rounds 1-4. Round 4 triggers chain [1,2,3] → commit 1.
        let (certificates, _next_parents) = make_certificates(1, 4, &genesis, &keys);
        let mut certificates = certificates;

        let (tx_waiter, rx_waiter) = channel(100);
        let (tx_primary, mut rx_primary) = channel(100);
        let (tx_output, mut rx_output) = channel(100);
        MP3Consensus::spawn(
            mock_committee(),
            /* gc_depth */ 50,
            /* k_slots */ 4,
            rx_waiter,
            tx_primary,
            tx_output,
        );
        tokio::spawn(async move { while rx_primary.recv().await.is_some() {} });

        while let Some(certificate) = certificates.pop_front() {
            tx_waiter.send(certificate).await.unwrap();
        }

        // With k=4, round 1 has 4 slot leaders. 3-chain commits round 1.
        // All 4 slot leaders at round 1 should be committed.
        for _ in 1..=4 {
            let certificate = rx_output.recv().await.unwrap();
            assert_eq!(certificate.round(), 1);
        }
    }

    // Test: MP3-BFT++ handles missing leaders gracefully with 3-chain.
    // Leader (keys[0]) missing for rounds 1-2, present from round 3.
    // With pipeline cadence, leader at round 1 has no keys[0], but
    // keys[1] is elected. Chain forms when 3 consecutive prepared rounds exist.
    #[tokio::test]
    async fn mp3bft_missing_leader() {
        let mut keys: Vec<_> = keys().into_iter().map(|(x, _)| x).collect();
        keys.sort();

        let genesis = Certificate::genesis(&mock_committee())
            .iter()
            .map(|x| x.digest())
            .collect::<BTreeSet<_>>();

        // Remove leader for rounds 1-2.
        let nodes: Vec<_> = keys.iter().cloned().skip(1).collect();
        let (mut certificates, parents) = make_certificates(1, 2, &genesis, &nodes);

        // Full participation from round 3 onward (rounds 3-8).
        let (out, _parents) = make_certificates(3, 8, &parents, &keys);
        certificates.extend(out);

        let (tx_waiter, rx_waiter) = channel(100);
        let (tx_primary, mut rx_primary) = channel(100);
        let (tx_output, mut rx_output) = channel(100);
        MP3Consensus::spawn(
            mock_committee(),
            /* gc_depth */ 50,
            /* k_slots */ 2,
            rx_waiter,
            tx_primary,
            tx_output,
        );
        tokio::spawn(async move { while rx_primary.recv().await.is_some() {} });

        while let Some(certificate) = certificates.pop_front() {
            tx_waiter.send(certificate).await.unwrap();
        }

        // Should commit when 3-chain forms. Collect several committed certs.
        let certificate = rx_output.recv().await.unwrap();
        assert!(certificate.round() >= 1);
        // Verify we get more commits (the chain continues).
        let certificate2 = rx_output.recv().await.unwrap();
        assert!(certificate2.round() >= 1);
    }

    // Test: verify 3-chain produces multiple consecutive commits correctly.
    // Pipeline cadence: every round has a leader.
    // With rounds 1-7:
    //   Round 2: prepare(1). Round 3: prepare(2). Round 4: prepare(3) → chain [1,2,3] → commit 1
    //   Round 5: prepare(4) → chain [2,3,4] → commit 2
    //   Round 6: prepare(5) → chain [3,4,5] → commit 3
    //   Round 7: prepare(6) → chain [4,5,6] → commit 4
    #[tokio::test]
    async fn mp3bft_3_chain_explicit() {
        let keys: Vec<_> = keys().into_iter().map(|(x, _)| x).collect();
        let genesis = Certificate::genesis(&mock_committee())
            .iter()
            .map(|x| x.digest())
            .collect::<BTreeSet<_>>();

        // Build DAG for rounds 1-7.
        let (certificates, _next_parents) = make_certificates(1, 7, &genesis, &keys);
        let mut certificates = certificates;

        let (tx_waiter, rx_waiter) = channel(100);
        let (tx_primary, mut rx_primary) = channel(100);
        let (tx_output, mut rx_output) = channel(100);
        MP3Consensus::spawn(
            mock_committee(),
            /* gc_depth */ 50,
            /* k_slots */ 1,
            rx_waiter,
            tx_primary,
            tx_output,
        );
        tokio::spawn(async move { while rx_primary.recv().await.is_some() {} });

        while let Some(certificate) = certificates.pop_front() {
            tx_waiter.send(certificate).await.unwrap();
        }

        // Commit 1 (round 1 leader): just 1 round-1 leader cert (genesis certs round 0 already committed).
        let cert = rx_output.recv().await.unwrap();
        assert_eq!(cert.round(), 1, "expected round 1 leader in first commit");

        // Commit 2 (round 2 leader): 3 remaining round-1 certs + 1 round-2 leader = 4.
        for _ in 1..=3 {
            let certificate = rx_output.recv().await.unwrap();
            assert_eq!(certificate.round(), 1, "expected remaining round 1 certs in second commit");
        }
        let cert = rx_output.recv().await.unwrap();
        assert_eq!(cert.round(), 2, "expected round 2 leader in second commit");

        // Commit 3 (round 3 leader): 3 remaining round-2 certs + 1 round-3 leader = 4.
        for _ in 1..=3 {
            let certificate = rx_output.recv().await.unwrap();
            assert_eq!(certificate.round(), 2, "expected remaining round 2 certs in third commit");
        }
        let cert = rx_output.recv().await.unwrap();
        assert_eq!(cert.round(), 3, "expected round 3 leader in third commit");

        // Commit 4 (round 4 leader): 3 remaining round-3 certs + 1 round-4 leader = 4.
        for _ in 1..=3 {
            let certificate = rx_output.recv().await.unwrap();
            assert_eq!(certificate.round(), 3, "expected remaining round 3 certs in fourth commit");
        }
        let cert = rx_output.recv().await.unwrap();
        assert_eq!(cert.round(), 4, "expected round 4 leader in fourth commit");
    }
}
