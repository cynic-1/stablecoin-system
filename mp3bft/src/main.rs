use mp3bft::{
    config::{Committee, MP3BFTConfig},
    control_plane::{
        ConsensusEngine,
        macro_layer::run_macro_consensus,
        slot_layer::run_slot_consensus,
    },
    data_plane::DataPlane,
    types::*,
};
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let csv_path = args.get(1).map(|s| s.as_str());

    eprintln!("MP3-BFT++ Consensus Protocol — Benchmark Suite");
    eprintln!("===============================================\n");

    let node_counts = [4, 10, 20];
    let k_values = [1, 2, 4, 8, 16];
    let num_views = 30;
    let batches_per_view = 50;
    let txns_per_batch = 10_000;
    let network_latency = Duration::from_millis(200);

    let header = "nodes,quorum,k,committed_blocks,committed_txns,time_s,tps\n";
    let mut csv_buf = String::from(header);

    eprintln!("--- Experiment 2A/2C: TPS vs Parallel Slots (k) [200ms RTT] ---");

    for &n in &node_counts {
        let node_ids: Vec<u64> = (0..n as u64).collect();
        let committee = Committee::new(node_ids);
        let quorum = committee.quorum_threshold();

        eprintln!("  Nodes: {} (quorum: {})", n, quorum);

        for &k in &k_values {
            let config = MP3BFTConfig {
                k_slots: k,
                network_latency,
                ..MP3BFTConfig::default()
            };

            let mut dp = DataPlane::new(committee.clone());

            for i in 0..(batches_per_view * num_views) {
                let txns: Vec<Vec<u8>> = (0..txns_per_batch)
                    .map(|j| {
                        let idx = (i * txns_per_batch + j) as u64;
                        idx.to_le_bytes().to_vec()
                    })
                    .collect();
                dp.submit_batch(0, txns);
            }

            let cert_digests: Vec<DigestBytes> = dp
                .available_certs()
                .iter()
                .map(|c| c.batch_digest)
                .collect();

            let mut engine = ConsensusEngine::new(config.clone(), committee.clone(), 0);
            let mut prev_qc: Option<MacroQC> = None;
            let mut total_committed_txns = 0;

            let start = Instant::now();

            for view in 0..num_views as u64 {
                let view_start = (view as usize) * batches_per_view;
                let view_end = std::cmp::min(view_start + batches_per_view, cert_digests.len());
                let view_certs = if view_start < cert_digests.len() {
                    &cert_digests[view_start..view_end]
                } else {
                    &[]
                };

                let slot_qcs = run_slot_consensus(view, &config, &committee, view_certs);
                thread::sleep(config.network_latency); // Slot phase RTT
                let qc = run_macro_consensus(
                    view, view, &committee, &slot_qcs, prev_qc.as_ref(),
                );
                thread::sleep(config.network_latency); // Macro phase RTT

                if let Some(qc) = qc {
                    let committed = engine.process_macro_qc(qc.clone(), slot_qcs);
                    for block in &committed {
                        total_committed_txns += block.header.slot_entries.len()
                            * txns_per_batch;
                    }
                    prev_qc = Some(qc);
                }
            }

            let elapsed = start.elapsed();
            let time_s = elapsed.as_secs_f64();
            let tps = if time_s > 0.0 {
                total_committed_txns as f64 / time_s
            } else {
                0.0
            };

            eprintln!(
                "    k={:<3} committed_blocks={:<4} committed_txns={:<8} time={:.3}s TPS={:.0}",
                k, engine.committed.len(), total_committed_txns, time_s, tps,
            );

            csv_buf.push_str(&format!(
                "{},{},{},{},{},{:.6},{:.0}\n",
                n, quorum, k, engine.committed.len(), total_committed_txns, time_s, tps,
            ));
        }
        eprintln!();
    }

    // Protocol comparison
    eprintln!("--- Simulated Protocol Comparison (n=4) [200ms RTT] ---");
    let committee = Committee::new(vec![0, 1, 2, 3]);

    for (name, k) in [("Tusk-like", 1usize), ("MP3-BFT++", 8)] {
        let config = MP3BFTConfig {
            k_slots: k,
            network_latency,
            ..MP3BFTConfig::default()
        };

        let cert_digests: Vec<DigestBytes> = (0..5000)
            .map(|i| DigestBytes::hash(&(i as u64).to_le_bytes()))
            .collect();

        let mut engine = ConsensusEngine::new(config.clone(), committee.clone(), 0);
        let mut prev_qc: Option<MacroQC> = None;

        let start = Instant::now();
        for view in 0..30u64 {
            let view_certs = &cert_digests[..std::cmp::min(50, cert_digests.len())];
            let slot_qcs = run_slot_consensus(view, &config, &committee, view_certs);
            thread::sleep(config.network_latency); // Slot phase RTT
            if let Some(qc) = run_macro_consensus(view, view, &committee, &slot_qcs, prev_qc.as_ref()) {
                engine.process_macro_qc(qc.clone(), slot_qcs);
                prev_qc = Some(qc);
            }
            thread::sleep(config.network_latency); // Macro phase RTT
        }
        let elapsed = start.elapsed();

        eprintln!(
            "  {}: committed={} blocks in {:.3}s ({:.0} blocks/s)",
            name, engine.committed.len(), elapsed.as_secs_f64(),
            engine.committed.len() as f64 / elapsed.as_secs_f64(),
        );
    }

    // Write CSV
    if let Some(path) = csv_path {
        let mut f = std::fs::File::create(path).expect("Cannot create CSV file");
        f.write_all(csv_buf.as_bytes()).expect("Cannot write CSV");
        eprintln!("CSV written to {}", path);
    } else {
        print!("{}", csv_buf);
    }
}
