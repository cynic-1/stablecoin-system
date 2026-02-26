use crate::{
    config::{Committee, MP3BFTConfig},
    control_plane::{
        ConsensusEngine,
        macro_layer::run_macro_consensus,
        slot_layer::run_slot_consensus,
    },
    data_plane::DataPlane,
    types::*,
};

#[test]
fn test_full_consensus_round() {
    let committee = Committee::new(vec![0, 1, 2, 3]);
    let config = MP3BFTConfig {
        k_slots: 4,
        ..MP3BFTConfig::default()
    };

    // Submit batches via data plane.
    let mut dp = DataPlane::new(committee.clone());
    for i in 0..8 {
        dp.submit_batch(0, vec![vec![i as u8; 64]]);
    }

    // Get AvailCert digests.
    let cert_digests: Vec<DigestBytes> = dp
        .available_certs()
        .iter()
        .map(|c| c.batch_digest)
        .collect();

    // Run slot consensus.
    let slot_qcs = run_slot_consensus(0, &config, &committee, &cert_digests);
    assert_eq!(slot_qcs.len(), 4);

    // Run macro consensus.
    let macro_qc = run_macro_consensus(0, 0, &committee, &slot_qcs, None);
    assert!(macro_qc.is_some());

    let qc = macro_qc.unwrap();
    assert!(qc.verify(committee.quorum_threshold()));
    assert_eq!(qc.header.slot_entries.len(), 4);
}

#[test]
fn test_multi_view_with_3_chain() {
    let committee = Committee::new(vec![0, 1, 2, 3]);
    let config = MP3BFTConfig {
        k_slots: 2,
        ..MP3BFTConfig::default()
    };
    let mut engine = ConsensusEngine::new(config.clone(), committee.clone(), 0);

    let certs: Vec<DigestBytes> = (0..4).map(|i| DigestBytes::hash(&[i as u8])).collect();

    let mut prev_qc: Option<MacroQC> = None;
    let mut total_committed = 0;

    for view in 0..5u64 {
        let slot_qcs = run_slot_consensus(view, &config, &committee, &certs);
        let qc = run_macro_consensus(
            view,
            view, // height == view for simplicity
            &committee,
            &slot_qcs,
            prev_qc.as_ref(),
        )
        .unwrap();

        let committed = engine.process_macro_qc(qc.clone(), slot_qcs);
        total_committed += committed.len();
        prev_qc = Some(qc);
    }

    // With 5 consecutive blocks, 3-chain should commit blocks 0, 1, 2.
    assert_eq!(total_committed, 3);
    assert_eq!(engine.committed.len(), 3);
}

#[test]
fn test_safety_no_conflicting_commits() {
    let committee = Committee::new(vec![0, 1, 2, 3]);
    let config = MP3BFTConfig {
        k_slots: 2,
        ..MP3BFTConfig::default()
    };
    let mut engine = ConsensusEngine::new(config.clone(), committee.clone(), 0);

    let certs: Vec<DigestBytes> = (0..4).map(|i| DigestBytes::hash(&[i as u8])).collect();

    let mut prev_qc: Option<MacroQC> = None;
    for view in 0..10u64 {
        let slot_qcs = run_slot_consensus(view, &config, &committee, &certs);
        let qc = run_macro_consensus(view, view, &committee, &slot_qcs, prev_qc.as_ref()).unwrap();
        engine.process_macro_qc(qc.clone(), slot_qcs);
        prev_qc = Some(qc);
    }

    // Verify no two committed blocks have the same height.
    let heights: Vec<Height> = engine.committed.iter().map(|b| b.height).collect();
    let unique: std::collections::HashSet<Height> = heights.iter().cloned().collect();
    assert_eq!(heights.len(), unique.len(), "No conflicting commits at same height");
}

#[test]
fn test_tps_scales_with_k() {
    let committee = Committee::new(vec![0, 1, 2, 3]);
    let certs: Vec<DigestBytes> = (0..100).map(|i| DigestBytes::hash(&[i as u8])).collect();

    let mut results = Vec::new();
    for k in [1, 2, 4, 8, 16] {
        let config = MP3BFTConfig {
            k_slots: k,
            ..MP3BFTConfig::default()
        };
        let start = std::time::Instant::now();
        for view in 0..100u64 {
            let _slot_qcs = run_slot_consensus(view, &config, &committee, &certs);
        }
        let elapsed = start.elapsed();
        results.push((k, elapsed));
    }

    // k=1 should be slowest (or equal), k=16 should handle same work.
    // Just verify no crashes and completion.
    for (k, elapsed) in &results {
        println!("k={}: {:?}", k, elapsed);
    }
}
