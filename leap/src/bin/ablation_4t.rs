use std::sync::Arc;
use std::time::Instant;
use leap::stablecoin::*;
use leap::cado::cado_with_mode;
use leap::domain_plan::build_domain_plan;
use leap::hot_delta::HotDeltaManager;
use leap::config::{CadoMode, LeapConfig};
use leap::executor::ParallelTransactionExecutor;

fn main() {
    let num_txns = 10000;
    let warmups = 2;
    let runs = 7;
    let accounts = 1000;

    let iters_per_us = calibrate_iters_per_us();
    let crypto_iters = (10.0 * iters_per_us).round() as u32;
    eprintln!("crypto_iters for 10us = {} ({:.1} iters/us)", crypto_iters, iters_per_us);

    let hotspot = HotspotConfig::Explicit { num_hotspots: 1, hotspot_ratio: 0.9 };
    let gen = StablecoinWorkloadGenerator::new(accounts, hotspot);

    for threads in &[2usize, 4] {
        // Baseline (no CADO, no opts)
        let config_base = LeapConfig { num_workers: *threads, ..LeapConfig::baseline() };
        let mut executor_base = ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config_base);
        let mut tps_base = Vec::new();
        for run in 0..(warmups + runs) {
            let txns = gen.generate(num_txns);
            let block_size = txns.len();
            let args = StablecoinExecArgs { crypto_work_iters: crypto_iters, hot_delta: None, funded_balance: 1_000_000 };
            let start = Instant::now();
            let _ = executor_base.execute_transactions_parallel(args, txns).unwrap();
            let elapsed = start.elapsed();
            if run >= warmups { tps_base.push(block_size as f64 / elapsed.as_secs_f64()); }
        }

        // Full LEAP (CADO + all opts)
        let config_full = LeapConfig { num_workers: *threads, ..LeapConfig::full() };
        let mut executor_full = ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config_full.clone());
        let mut tps_leap = Vec::new();
        for run in 0..(warmups + runs) {
            let mut txns = gen.generate(num_txns);
            let mut mgr = HotDeltaManager::new(config_full.theta_1, config_full.theta_2, config_full.p_max);
            mgr.detect_hotspots(&txns);
            let hot_delta = if mgr.is_skewed() { Some(Arc::new(mgr)) } else { None };
            if hot_delta.is_some() {
                cado_with_mode(&mut txns, &config_full.cado_mode);
                if config_full.enable_domain_aware && config_full.cado_mode == CadoMode::Concatenate {
                    let plan = build_domain_plan(&txns, config_full.l_max);
                    let n = txns.len();
                    executor_full.set_segment_bounds(plan.segment_bounds(), plan.txn_to_segment(n));
                }
            }
            let block_size = txns.len();
            let args = StablecoinExecArgs { crypto_work_iters: crypto_iters, hot_delta, funded_balance: 1_000_000 };
            let start = Instant::now();
            let _ = executor_full.execute_transactions_parallel(args, txns).unwrap();
            let elapsed = start.elapsed();
            if run >= warmups { tps_leap.push(block_size as f64 / elapsed.as_secs_f64()); }
        }

        tps_base.sort_by(|a, b| a.partial_cmp(b).unwrap());
        tps_leap.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med_base = tps_base[tps_base.len() / 2];
        let med_leap = tps_leap[tps_leap.len() / 2];
        println!("{}T H90% 10us:  Base={:.0}  LEAP={:.0}  delta={:+.1}%",
            threads, med_base, med_leap, (med_leap - med_base) / med_base * 100.0);
    }
}

fn calibrate_iters_per_us() -> f64 {
    let num_threads = num_cpus::get().min(32);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Failed to create calibration pool");
    let test_iters = 10_000u32;

    // Warmup all threads to stabilize CPU frequency.
    pool.scope(|s| {
        for t in 0..num_threads {
            s.spawn(move |_| {
                for i in 0..10u64 {
                    simulate_tx_crypto_work((t as u64) * 1000 + i, test_iters);
                }
            });
        }
    });

    // Measure under load: wall-clock ≈ per-thread time.
    let start = Instant::now();
    pool.scope(|s| {
        for t in 0..num_threads {
            s.spawn(move |_| {
                for i in 0..100u64 {
                    simulate_tx_crypto_work((t as u64) * 1000 + i, test_iters);
                }
            });
        }
    });
    let elapsed_us = start.elapsed().as_micros() as f64 / 100.0;
    test_iters as f64 / elapsed_us
}
