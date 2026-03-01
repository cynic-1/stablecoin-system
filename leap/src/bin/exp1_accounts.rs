use std::env;
use std::fs;
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use leap::cado::cado_ordering;
use leap::config::LeapConfig;
use leap::domain_plan::build_domain_plan;
use leap::executor::ParallelTransactionExecutor;
use leap::hot_delta::HotDeltaManager;
use leap::stablecoin::*;

const BLOCK_SIZE: usize = 10_000;
const OVERHEAD_US: u32 = 100;
const ACCOUNTS: &[usize] = &[2, 10, 100, 1000, 10000];
const THREADS: &[usize] = &[4, 8, 16, 32];
const RUNS: usize = 3;
const FUNDED_BALANCE: u64 = 1_000_000;

fn main() {
    let csv_path = env::args().nth(1).unwrap_or_else(|| "exp1_accounts.csv".to_string());
    let cpus = num_cpus::get();

    let max_threads = THREADS.iter().copied().filter(|&t| t <= cpus).max().unwrap_or(cpus);

    // Calibrate crypto overhead under multi-threaded load to capture real
    // all-core frequency (turbo drops on multi-socket NUMA machines).
    let iters_per_us = calibrate_iters_per_us(max_threads);
    let crypto_iters = (OVERHEAD_US as f64 * iters_per_us).round() as u32;

    eprintln!("=== Exp-1 Account Sweep ===");
    eprintln!("CPUs: {}", cpus);
    eprintln!("Calibration ({} threads): {:.1} iters/us -> {} iters for {}us", max_threads, iters_per_us, crypto_iters, OVERHEAD_US);
    eprintln!("TIP: on NUMA machines, pin to one socket: numactl --cpunodebind=0 --membind=0 ./exp1_accounts");
    eprintln!("Block size: {}, Runs: {}", BLOCK_SIZE, RUNS);
    eprintln!("Accounts: {:?}", ACCOUNTS);
    eprintln!("Threads: {:?}", THREADS);
    eprintln!();

    // Warmup: 1 dummy block per engine at (accounts=1000, threads=4)
    {
        eprintln!("Warming up rayon pool...");
        let gen = StablecoinWorkloadGenerator::new(1000, HotspotConfig::Uniform);
        let args = StablecoinExecArgs { crypto_work_iters: crypto_iters, hot_delta: None, funded_balance: FUNDED_BALANCE };

        let config_base = LeapConfig { num_workers: 4, ..LeapConfig::baseline() };
        let executor_base = ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config_base);
        let txns = gen.generate(BLOCK_SIZE);
        let _ = executor_base.execute_transactions_parallel(args.clone(), txns);

        let config_full = LeapConfig { num_workers: 4, ..LeapConfig::full() };
        let executor_full = ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config_full);
        let txns = gen.generate(BLOCK_SIZE);
        let args = StablecoinExecArgs { crypto_work_iters: crypto_iters, hot_delta: None, funded_balance: FUNDED_BALANCE };
        let _ = executor_full.execute_transactions_parallel(args, txns);
        eprintln!("Warmup done.\n");
    }

    let mut rows: Vec<String> = Vec::new();

    for &accounts in ACCOUNTS {
        let gen = StablecoinWorkloadGenerator::new(accounts, HotspotConfig::Uniform);

        for &threads in THREADS {
            if threads > cpus {
                eprintln!("SKIP threads={} > cpus={}", threads, cpus);
                continue;
            }

            let mut tps_base_runs = Vec::new();
            let mut tps_leap_runs = Vec::new();

            for run in 0..RUNS {
                let seed = (accounts as u64) * 1000 + run as u64;
                let txns = gen.generate_seeded(BLOCK_SIZE, seed);

                // --- LEAP-base (BlockSTM equivalent): no CADO, no optimizations ---
                let config_base = LeapConfig { num_workers: threads, ..LeapConfig::baseline() };
                let executor_base = ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config_base);
                let args_base = StablecoinExecArgs { crypto_work_iters: crypto_iters, hot_delta: None, funded_balance: FUNDED_BALANCE };
                let start = Instant::now();
                let _ = executor_base.execute_transactions_parallel(args_base, txns.clone()).unwrap();
                let elapsed_base = start.elapsed();
                let tps_base = BLOCK_SIZE as f64 / elapsed_base.as_secs_f64();

                // --- LEAP (CADO + all optimizations) ---
                let mut txns_leap = txns;
                cado_ordering(&mut txns_leap);

                let config_full = LeapConfig { num_workers: threads, ..LeapConfig::full() };
                let mut executor_full = ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config_full.clone());

                let mut mgr = HotDeltaManager::new(config_full.theta_1, config_full.theta_2, config_full.p_max);
                mgr.detect_hotspots(&txns_leap);
                let hot_delta = Some(Arc::new(mgr));

                if config_full.enable_domain_aware {
                    let plan = build_domain_plan(&txns_leap, config_full.l_max);
                    let n = txns_leap.len();
                    executor_full.set_segment_bounds(plan.segment_bounds(), plan.txn_to_segment(n));
                }

                let args_leap = StablecoinExecArgs { crypto_work_iters: crypto_iters, hot_delta, funded_balance: FUNDED_BALANCE };
                let start = Instant::now();
                let _ = executor_full.execute_transactions_parallel(args_leap, txns_leap).unwrap();
                let elapsed_leap = start.elapsed();
                let tps_leap = BLOCK_SIZE as f64 / elapsed_leap.as_secs_f64();

                rows.push(format!("LEAP-base,{},{},{},{:.0}", accounts, threads, run, tps_base));
                rows.push(format!("LEAP,{},{},{},{:.0}", accounts, threads, run, tps_leap));

                tps_base_runs.push(tps_base);
                tps_leap_runs.push(tps_leap);
            }

            let avg_base: f64 = tps_base_runs.iter().sum::<f64>() / RUNS as f64;
            let avg_leap: f64 = tps_leap_runs.iter().sum::<f64>() / RUNS as f64;
            let delta = (avg_leap - avg_base) / avg_base * 100.0;
            eprintln!(
                "accounts={:<5} threads={:<2}  LEAP-base={:>7.0}  LEAP={:>7.0}  delta={:+.1}%",
                accounts, threads, avg_base, avg_leap, delta
            );
        }
    }

    // Write CSV
    let mut f = fs::File::create(&csv_path).expect("cannot create CSV");
    writeln!(f, "engine,accounts,threads,run,tps").unwrap();
    for row in &rows {
        writeln!(f, "{}", row).unwrap();
    }
    eprintln!("\nWrote {} rows to {}", rows.len(), csv_path);
}

/// Calibrate SHA-256 iterations per microsecond under multi-threaded load.
/// On multi-socket machines, single-threaded calibration runs at turbo boost
/// (~3.9GHz) but the benchmark runs at all-core base (~2.3GHz), causing
/// crypto overhead to be 50-70% higher than intended.
fn calibrate_iters_per_us(num_threads: usize) -> f64 {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Failed to create calibration thread pool");
    let test_iters = 10_000u32;

    // Warmup: all threads run crypto work to stabilize CPU frequency.
    pool.scope(|s| {
        for t in 0..num_threads {
            s.spawn(move |_| {
                for i in 0..10u64 {
                    simulate_tx_crypto_work((t as u64) * 1000 + i, test_iters);
                }
            });
        }
    });

    // Measure: all threads busy, wall-clock ≈ per-thread time.
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
