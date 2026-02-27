use leap::{
    cado::cado_ordering,
    config::LeapConfig,
    domain_plan::build_domain_plan,
    executor::ParallelTransactionExecutor,
    hot_delta::HotDeltaManager,
    stablecoin::*,
};
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

/// Target overhead levels in microseconds.
const OVERHEAD_TARGETS_US: &[u32] = &[0, 1, 3, 10, 50, 100];

/// Measure how many SHA-256 iterations correspond to 1 microsecond on this CPU.
/// Runs for ~50ms to get a stable estimate.
fn calibrate_iters_per_us() -> f64 {
    use leap::stablecoin::simulate_tx_crypto_work;
    let warmup_iters = 1000u32;
    let _ = simulate_tx_crypto_work(42, warmup_iters);

    let measure_iters = 10_000u32;
    let start = std::time::Instant::now();
    let _ = simulate_tx_crypto_work(42, measure_iters);
    let elapsed_us = start.elapsed().as_secs_f64() * 1e6;
    measure_iters as f64 / elapsed_us
}

/// Build overhead levels calibrated for the current CPU.
/// Returns Vec of (label, sha256_iters, target_us).
/// Labels are leaked to `&'static str` for compatibility with the rest of the code.
fn calibrated_overhead_levels(iters_per_us: f64) -> Vec<(&'static str, u32, u32)> {
    OVERHEAD_TARGETS_US
        .iter()
        .map(|&us| {
            let iters = if us == 0 {
                0u32
            } else {
                ((us as f64 * iters_per_us).round() as u32).max(1)
            };
            let label: &'static str = Box::leak(format!("{}us", us).into_boxed_str());
            (label, iters, us)
        })
        .collect()
}

/// Overhead levels where parallel execution is expected to beat serial.
/// Below this threshold, serial execution dominates due to zero synchronization
/// overhead, making parallel engine comparisons meaningless.
/// This is determined at runtime by the viability check (Part 0).
const PARALLEL_VIABLE_REFERENCE_THREADS: usize = 4;

fn scenarios() -> Vec<(&'static str, HotspotConfig)> {
    vec![
        ("Uniform", HotspotConfig::Uniform),
        ("Zipf_0.8", HotspotConfig::Zipf { alpha: 0.8 }),
        ("Zipf_1.2", HotspotConfig::Zipf { alpha: 1.2 }),
        (
            "Hotspot_50pct",
            HotspotConfig::Explicit {
                num_hotspots: 1,
                hotspot_ratio: 0.5,
            },
        ),
        (
            "Hotspot_90pct",
            HotspotConfig::Explicit {
                num_hotspots: 1,
                hotspot_ratio: 0.9,
            },
        ),
    ]
}

fn engine_configs() -> Vec<(&'static str, LeapConfig, bool)> {
    vec![
        ("LEAP-base", LeapConfig::baseline(), false),
        ("LEAP", LeapConfig::full(), true),
        (
            "LEAP-noDomain",
            LeapConfig {
                enable_domain_aware: false,
                ..LeapConfig::full()
            },
            true,
        ),
        (
            "LEAP-noHotDelta",
            LeapConfig {
                enable_hot_delta: false,
                ..LeapConfig::full()
            },
            true,
        ),
        (
            "LEAP-noBP",
            LeapConfig {
                enable_backpressure: false,
                ..LeapConfig::full()
            },
            true,
        ),
    ]
}

/// Run a single benchmark point and return TPS values for each run.
fn bench_parallel(
    _engine_name: &str,
    base_config: &LeapConfig,
    use_cado: bool,
    _scenario_name: &str,
    hotspot: &HotspotConfig,
    num_accounts: usize,
    threads: usize,
    crypto_iters: u32,
    num_txns: usize,
    num_warmups: usize,
    num_runs: usize,
) -> Vec<f64> {
    let gen = StablecoinWorkloadGenerator::new(num_accounts, hotspot.clone());
    let mut tps_values = Vec::new();

    // Create executor ONCE — backpressure adapts across blocks.
    let config = LeapConfig {
        num_workers: threads,
        ..base_config.clone()
    };
    let mut executor =
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config.clone());

    for run in 0..(num_warmups + num_runs) {
        let mut txns = gen.generate_with_funding(num_txns, 1_000_000);
        if use_cado {
            cado_ordering(&mut txns);
        }

        // Setup Hot-Delta if enabled (per block — hotspot detection is block-specific).
        let hot_delta = if config.enable_hot_delta {
            let mut mgr = HotDeltaManager::new(config.theta_1, config.theta_2, config.p_max);
            mgr.detect_hotspots(&txns);
            Some(Arc::new(mgr))
        } else {
            None
        };

        // Setup Domain-Aware scheduling if enabled.
        if config.enable_domain_aware && use_cado {
            let plan = build_domain_plan(&txns, config.l_max);
            let num_txns_total = txns.len();
            executor.set_segment_bounds(plan.segment_bounds(), plan.txn_to_segment(num_txns_total));
        }

        let block_size = txns.len();
        let args = StablecoinExecArgs {
            crypto_work_iters: crypto_iters,
            hot_delta,
        };

        let start = Instant::now();
        let _result = executor
            .execute_transactions_parallel(args, txns)
            .expect("Execution failed");
        let elapsed = start.elapsed();
        let tps = block_size as f64 / elapsed.as_secs_f64();

        if run >= num_warmups {
            tps_values.push(tps);
        }
    }

    tps_values
}

fn bench_serial(
    hotspot: &HotspotConfig,
    num_accounts: usize,
    crypto_iters: u32,
    num_txns: usize,
    num_warmups: usize,
    num_runs: usize,
) -> Vec<f64> {
    let gen = StablecoinWorkloadGenerator::new(num_accounts, hotspot.clone());
    let mut tps_values = Vec::new();

    for run in 0..(num_warmups + num_runs) {
        let txns = gen.generate_with_funding(num_txns, 1_000_000);
        let block_size = txns.len();
        let start = Instant::now();
        let _ = serial_execute(&txns, crypto_iters);
        let elapsed = start.elapsed();
        let tps = block_size as f64 / elapsed.as_secs_f64();

        if run >= num_warmups {
            tps_values.push(tps);
        }
    }

    tps_values
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let csv_path = args.get(1).map(|s| s.as_str());

    eprintln!("LEAP Parallel Execution Engine — Multi-Dimension Benchmark Suite");
    eprintln!("================================================================");
    eprintln!("CPUs available: {}", num_cpus::get());
    if let Some(p) = csv_path {
        eprintln!("CSV output: {}", p);
    }

    // Calibrate SHA-256 throughput on this CPU so overhead labels are accurate.
    let iters_per_us = calibrate_iters_per_us();
    eprintln!("SHA-256 calibration: {:.1} iters/μs  ({:.0} ns/iter)",
        iters_per_us, 1000.0 / iters_per_us);
    let overhead_levels = calibrated_overhead_levels(iters_per_us);
    eprintln!("Overhead levels: {}",
        overhead_levels.iter()
            .map(|(label, iters, _)| format!("{}={}iters", label, iters))
            .collect::<Vec<_>>().join(", "));
    eprintln!();

    let num_txns = 10_000;
    let num_warmups = 2;
    let num_runs = 7;
    let max_threads = num_cpus::get();

    // CSV buffer: engine,scenario,accounts,overhead_us,threads,run,tps
    let header = "engine,scenario,accounts,overhead_us,threads,run,tps\n";
    let mut csv_buf = String::from(header);

    let all_scenarios = scenarios();
    let all_engines = engine_configs();

    // Helper to append CSV rows
    let mut emit = |engine: &str,
                    scenario: &str,
                    accounts: usize,
                    overhead_us: u32,
                    threads: usize,
                    tps_values: &[f64]| {
        for (run_idx, &tps) in tps_values.iter().enumerate() {
            csv_buf.push_str(&format!(
                "{},{},{},{},{},{},{:.0}\n",
                engine, scenario, accounts, overhead_us, threads, run_idx, tps
            ));
        }
    };

    // =========================================================================
    // Part 0: Parallel Viability Check
    // Determine at which overhead levels parallel execution beats serial.
    // This is a prerequisite: comparing LEAP vs LEAP-base is only meaningful
    // when parallel execution outperforms serial execution.
    // =========================================================================
    eprintln!("=== Part 0: Parallel Viability Check (parallel > serial?) ===");
    eprintln!("  Reference: {} threads vs serial", PARALLEL_VIABLE_REFERENCE_THREADS);

    let viability_scenario = &all_scenarios
        .iter()
        .find(|(n, _)| *n == "Uniform")
        .unwrap()
        .1;
    let viability_engine = &all_engines
        .iter()
        .find(|(n, _, _)| *n == "LEAP-base")
        .unwrap();

    let mut viable_overheads: Vec<(&str, u32, u32)> = Vec::new();
    let mut all_overhead_viability: Vec<(&str, u32, f64, f64, bool)> = Vec::new();

    for &(oh_label, oh_iters, oh_us) in &overhead_levels {
        let serial_tps = bench_serial(
            viability_scenario, 1000, oh_iters, num_txns, num_warmups, num_runs,
        );
        let parallel_tps = bench_parallel(
            viability_engine.0,
            &viability_engine.1,
            viability_engine.2,
            "Uniform",
            viability_scenario,
            1000,
            PARALLEL_VIABLE_REFERENCE_THREADS,
            oh_iters,
            num_txns,
            num_warmups,
            num_runs,
        );
        let s = median(&serial_tps);
        let p = median(&parallel_tps);
        let viable = p > s;
        all_overhead_viability.push((oh_label, oh_us, s, p, viable));

        if viable {
            viable_overheads.push((oh_label, oh_iters, oh_us));
        }

        eprintln!(
            "  {:>5}: Serial={:>12.0}  Parallel({}t)={:>12.0}  {}",
            oh_label,
            s,
            PARALLEL_VIABLE_REFERENCE_THREADS,
            p,
            if viable { "VIABLE" } else { "SKIP (serial wins)" }
        );

        // Record viability check data in CSV
        emit("Serial", "Uniform", 1000, oh_us, 1, &serial_tps);
        emit(
            "LEAP-base",
            "Uniform",
            1000,
            oh_us,
            PARALLEL_VIABLE_REFERENCE_THREADS,
            &parallel_tps,
        );
    }

    let viable_threshold = all_overhead_viability
        .iter()
        .find(|(_, _, _, _, v)| *v)
        .map(|(label, us, _, _, _)| (*label, *us));

    if let Some((label, us)) = viable_threshold {
        eprintln!("\n  Parallel viability threshold: {} ({}μs)", label, us);
        eprintln!("  Engine comparisons will focus on overhead >= {}μs", us);
    } else {
        eprintln!("\n  WARNING: Parallel never beats serial at any overhead level!");
        eprintln!("  All engine comparison data should be interpreted with caution.");
    }
    eprintln!();

    // =========================================================================
    // Part 1: Main comparison (LEAP-base vs LEAP) — only viable overheads
    // =========================================================================
    eprintln!("=== Part 1: Main Comparison (viable overhead × scenario × threads) ===");
    if viable_overheads.is_empty() {
        eprintln!("  SKIPPED — no viable overhead levels found.");
        eprintln!("  Falling back to 10μs+ for reference data.\n");
        // Fall back to 10μs+ for at least some data
        for &(oh_label, oh_iters, oh_us) in &overhead_levels {
            if oh_us >= 10 {
                viable_overheads.push((oh_label, oh_iters, oh_us));
            }
        }
    }

    let part1_scenarios = ["Uniform", "Hotspot_90pct"];
    let part1_threads = [1, 2, 4, 8, 16];

    for &(oh_label, oh_iters, oh_us) in &viable_overheads {
        eprintln!("  Overhead: {} ({}μs, {} iters)", oh_label, oh_us, oh_iters);

        for &scenario_name in &part1_scenarios {
            let hotspot = &all_scenarios
                .iter()
                .find(|(n, _)| *n == scenario_name)
                .unwrap()
                .1;

            // Serial baseline
            let tps = bench_serial(hotspot, 1000, oh_iters, num_txns, num_warmups, num_runs);
            eprintln!(
                "    Serial/{}: median={:.0} TPS",
                scenario_name,
                median(&tps)
            );
            emit("Serial", scenario_name, 1000, oh_us, 1, &tps);

            // LEAP-base and LEAP
            for (engine_name, base_config, use_cado) in &all_engines {
                if *engine_name != "LEAP-base" && *engine_name != "LEAP" {
                    continue;
                }
                for &threads in &part1_threads {
                    if threads > max_threads {
                        continue;
                    }
                    let tps = bench_parallel(
                        engine_name,
                        base_config,
                        *use_cado,
                        scenario_name,
                        hotspot,
                        1000,
                        threads,
                        oh_iters,
                        num_txns,
                        num_warmups,
                        num_runs,
                    );
                    eprintln!(
                        "    {}/{} {}t: median={:.0} TPS",
                        engine_name,
                        scenario_name,
                        threads,
                        median(&tps)
                    );
                    emit(engine_name, scenario_name, 1000, oh_us, threads, &tps);
                }
            }
        }
        eprintln!();
    }

    // =========================================================================
    // Part 2: Contention intensity (vary accounts)
    // =========================================================================
    eprintln!("=== Part 2: Contention Intensity (vary accounts) ===");

    let part2_accounts = [50, 200, 1000];
    let part2_threads = [1, 4, 8, 16];
    let part2_oh_iters = 160u32; // 10μs — moderate, both compute and contention matter
    let part2_oh_us = 10u32;
    let part2_scenario = "Hotspot_90pct";
    let part2_hotspot = &all_scenarios
        .iter()
        .find(|(n, _)| *n == part2_scenario)
        .unwrap()
        .1;

    for &accounts in &part2_accounts {
        eprintln!("  Accounts: {}", accounts);
        for (engine_name, base_config, use_cado) in &all_engines {
            if *engine_name != "LEAP-base" && *engine_name != "LEAP" {
                continue;
            }
            for &threads in &part2_threads {
                if threads > max_threads {
                    continue;
                }
                let tps = bench_parallel(
                    engine_name,
                    base_config,
                    *use_cado,
                    part2_scenario,
                    part2_hotspot,
                    accounts,
                    threads,
                    part2_oh_iters,
                    num_txns,
                    num_warmups,
                    num_runs,
                );
                eprintln!(
                    "    {}/{}accts {}t: median={:.0} TPS",
                    engine_name,
                    accounts,
                    threads,
                    median(&tps)
                );
                emit(
                    engine_name,
                    part2_scenario,
                    accounts,
                    part2_oh_us,
                    threads,
                    &tps,
                );
            }
        }
    }
    eprintln!();

    // =========================================================================
    // Part 3: Ablation study (10μs overhead, all 5 engine configs)
    // Use viable overhead so ablation differences are meaningful.
    // =========================================================================
    let part3_oh_iters = 160u32;  // 10μs — above parallel viability threshold
    let part3_oh_us = 10u32;

    eprintln!("=== Part 3: Ablation Study ({}μs overhead) ===", part3_oh_us);
    let part3_scenarios = ["Zipf_0.8", "Hotspot_90pct"];
    let part3_threads = [1, 4, 8, 16];

    for &scenario_name in &part3_scenarios {
        eprintln!("  Scenario: {}", scenario_name);
        let hotspot = &all_scenarios
            .iter()
            .find(|(n, _)| *n == scenario_name)
            .unwrap()
            .1;

        for (engine_name, base_config, use_cado) in &all_engines {
            for &threads in &part3_threads {
                if threads > max_threads {
                    continue;
                }
                let tps = bench_parallel(
                    engine_name,
                    base_config,
                    *use_cado,
                    scenario_name,
                    hotspot,
                    1000,
                    threads,
                    part3_oh_iters,
                    num_txns,
                    num_warmups,
                    num_runs,
                );
                eprintln!(
                    "    {}/{} {}t: median={:.0} TPS",
                    engine_name,
                    scenario_name,
                    threads,
                    median(&tps)
                );
                emit(engine_name, scenario_name, 1000, part3_oh_us, threads, &tps);
            }
        }
    }
    eprintln!();

    // =========================================================================
    // Part 4: Full scenario sweep at realistic overhead (100μs)
    // =========================================================================
    eprintln!("=== Part 4: Realistic Scaling (100μs overhead, all scenarios) ===");

    let part4_oh_iters = 1600u32;
    let part4_oh_us = 100u32;
    let part4_threads = [1, 2, 4, 8, 16];

    for (scenario_name, hotspot) in &all_scenarios {
        eprintln!("  Scenario: {}", scenario_name);

        // Serial
        let tps = bench_serial(hotspot, 1000, part4_oh_iters, num_txns, num_warmups, num_runs);
        eprintln!("    Serial: median={:.0} TPS", median(&tps));
        emit("Serial", scenario_name, 1000, part4_oh_us, 1, &tps);

        // LEAP-base and LEAP
        for (engine_name, base_config, use_cado) in &all_engines {
            if *engine_name != "LEAP-base" && *engine_name != "LEAP" {
                continue;
            }
            for &threads in &part4_threads {
                if threads > max_threads {
                    continue;
                }
                let tps = bench_parallel(
                    engine_name,
                    base_config,
                    *use_cado,
                    scenario_name,
                    hotspot,
                    1000,
                    threads,
                    part4_oh_iters,
                    num_txns,
                    num_warmups,
                    num_runs,
                );
                eprintln!(
                    "    {}/{} {}t: median={:.0} TPS",
                    engine_name,
                    scenario_name,
                    threads,
                    median(&tps)
                );
                emit(engine_name, scenario_name, 1000, part4_oh_us, threads, &tps);
            }
        }
    }
    eprintln!();

    // =========================================================================
    // Write CSV output
    // =========================================================================
    if let Some(path) = csv_path {
        let mut f = std::fs::File::create(path).expect("Cannot create CSV file");
        f.write_all(csv_buf.as_bytes()).expect("Cannot write CSV");
        eprintln!("CSV written to {}", path);
    } else {
        print!("{}", csv_buf);
    }

    eprintln!("Benchmark complete.");
}
