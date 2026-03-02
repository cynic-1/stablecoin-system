use std::env;
use std::fs;
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use leap::cado::cado_with_mode;
use leap::config::{CadoMode, LeapConfig};
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

fn scenarios() -> Vec<(&'static str, HotspotConfig)> {
    vec![
        ("Uniform", HotspotConfig::Uniform),
        ("Hotspot10", HotspotConfig::Explicit { num_hotspots: 1, hotspot_ratio: 0.10 }),
        ("Hotspot30", HotspotConfig::Explicit { num_hotspots: 1, hotspot_ratio: 0.30 }),
        ("Hotspot50", HotspotConfig::Explicit { num_hotspots: 1, hotspot_ratio: 0.50 }),
        ("Hotspot70", HotspotConfig::Explicit { num_hotspots: 1, hotspot_ratio: 0.70 }),
        ("Hotspot90", HotspotConfig::Explicit { num_hotspots: 1, hotspot_ratio: 0.90 }),
    ]
}

struct EngineSpec {
    name: &'static str,
    config: LeapConfig,
}

fn engine_specs() -> Vec<EngineSpec> {
    vec![
        EngineSpec {
            name: "LEAP-base",
            config: LeapConfig::baseline(),
        },
        EngineSpec {
            name: "LEAP-concat",
            config: LeapConfig::full_concat(),
        },
        EngineSpec {
            name: "LEAP-interleave",
            config: LeapConfig::full(),
        },
    ]
}

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
    let all_scenarios = scenarios();
    eprintln!("Scenarios: {:?}", all_scenarios.iter().map(|(n, _)| n).collect::<Vec<_>>());
    eprintln!("Engines: LEAP-base, LEAP-concat, LEAP-interleave");
    eprintln!();

    let engines = engine_specs();

    // Warmup: 1 dummy block per engine at (accounts=1000, threads=4)
    {
        eprintln!("Warming up rayon pool...");
        let gen = StablecoinWorkloadGenerator::new(1000, HotspotConfig::Uniform);

        for spec in &engines {
            let config = LeapConfig { num_workers: 4, ..spec.config.clone() };
            let executor = ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config);
            let txns = gen.generate(BLOCK_SIZE);
            let args = StablecoinExecArgs { crypto_work_iters: crypto_iters, hot_delta: None, funded_balance: FUNDED_BALANCE };
            let _ = executor.execute_transactions_parallel(args, txns);
        }
        eprintln!("Warmup done.\n");
    }

    let mut rows: Vec<String> = Vec::new();

    for &(scenario_name, ref hotspot_config) in &all_scenarios {
        eprintln!("--- Scenario: {} ---", scenario_name);

        for &accounts in ACCOUNTS {
            let gen = StablecoinWorkloadGenerator::new(accounts, hotspot_config.clone());

            // Serial baseline (once per scenario×accounts, threads=1).
            let mut serial_tps_runs = Vec::new();
            for run in 0..RUNS {
                let seed = (accounts as u64) * 1000 + run as u64;
                let txns = gen.generate_seeded(BLOCK_SIZE, seed);
                let start = Instant::now();
                let _ = serial_execute(&txns, crypto_iters);
                let elapsed = start.elapsed();
                let tps = BLOCK_SIZE as f64 / elapsed.as_secs_f64();
                rows.push(format!("Serial,{},{},1,{},{:.0}", scenario_name, accounts, run, tps));
                serial_tps_runs.push(tps);
            }
            let serial_avg = serial_tps_runs.iter().sum::<f64>() / RUNS as f64;
            eprintln!("{} accounts={:<5} threads=1  Serial={:>7.0}", scenario_name, accounts, serial_avg);

            for &threads in THREADS {
                if threads > cpus {
                    eprintln!("SKIP threads={} > cpus={}", threads, cpus);
                    continue;
                }

                let mut tps_by_engine: Vec<(&str, Vec<f64>)> = engines.iter().map(|e| (e.name, Vec::new())).collect();

                for run in 0..RUNS {
                    let seed = (accounts as u64) * 1000 + run as u64;
                    let txns_original = gen.generate_seeded(BLOCK_SIZE, seed);

                    for (eng_idx, spec) in engines.iter().enumerate() {
                        let mut txns = txns_original.clone();
                        let config = LeapConfig { num_workers: threads, ..spec.config.clone() };

                        let mut executor = ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config.clone());

                        // Detect skew on original ordering before any reordering.
                        let hot_delta = if config.enable_hot_delta {
                            let mut mgr = HotDeltaManager::new(config.theta_1, config.theta_2, config.p_max);
                            mgr.detect_hotspots(&txns);
                            if mgr.is_skewed() { Some(Arc::new(mgr)) } else { None }
                        } else {
                            None
                        };

                        // Apply CADO + Domain-Aware only when beneficial.
                        // When enable_hot_delta is true but workload isn't skewed,
                        // CADO reordering adds overhead without benefit.
                        let use_cado = !(config.enable_hot_delta && hot_delta.is_none());
                        if use_cado {
                            cado_with_mode(&mut txns, &config.cado_mode);
                            if config.enable_domain_aware && config.cado_mode == CadoMode::Concatenate {
                                let plan = build_domain_plan(&txns, config.l_max);
                                let n = txns.len();
                                executor.set_segment_bounds(plan.segment_bounds(), plan.txn_to_segment(n));
                            }
                        }

                        let args = StablecoinExecArgs { crypto_work_iters: crypto_iters, hot_delta, funded_balance: FUNDED_BALANCE };
                        let start = Instant::now();
                        let _ = executor.execute_transactions_parallel(args, txns).unwrap();
                        let elapsed = start.elapsed();
                        let tps = BLOCK_SIZE as f64 / elapsed.as_secs_f64();

                        rows.push(format!("{},{},{},{},{},{:.0}", spec.name, scenario_name, accounts, threads, run, tps));
                        tps_by_engine[eng_idx].1.push(tps);
                    }
                }

                // Print summary with speedup vs serial
                let avgs: Vec<f64> = tps_by_engine.iter().map(|(_, v)| v.iter().sum::<f64>() / RUNS as f64).collect();
                eprint!("{} accounts={:<5} threads={:<2} ", scenario_name, accounts, threads);
                for (i, (name, _)) in tps_by_engine.iter().enumerate() {
                    let vs_serial = (avgs[i] / serial_avg - 1.0) * 100.0;
                    if i == 0 {
                        eprint!(" {}={:>7.0}({:+.0}%vs serial)", name, avgs[i], vs_serial);
                    } else {
                        let vs_base = (avgs[i] - avgs[0]) / avgs[0] * 100.0;
                        eprint!("  {}={:>7.0}({:+.1}%vs base)", name, avgs[i], vs_base);
                    }
                }
                eprintln!();
            }
        }
        eprintln!();
    }

    // Write CSV
    let mut f = fs::File::create(&csv_path).expect("cannot create CSV");
    writeln!(f, "engine,scenario,accounts,threads,run,tps").unwrap();
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
