/// Reproduce LEAP execution panic and profile per-certificate overhead.
///
/// Mimics narwhal/node/src/main.rs lines 299-406: repeatedly generates
/// transactions, runs CADO ordering + HotDelta + DomainPlan + parallel
/// execution on a REUSED executor (same as E2E).
///
/// Usage:
///   cargo run --release --bin repro_e2e_panic [-- --iterations 200 --txns 976]
use leap::{
    cado::cado_ordering,
    config::LeapConfig,
    domain_plan::build_domain_plan,
    executor::ParallelTransactionExecutor,
    hot_delta::HotDeltaManager,
    stablecoin::*,
};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn main() {
    // Parse CLI args (simple).
    let args: Vec<String> = std::env::args().collect();
    let iterations: usize = arg_value(&args, "--iterations").unwrap_or(200);
    let num_txns: usize = arg_value(&args, "--txns").unwrap_or(976);
    let num_accounts: usize = arg_value(&args, "--accounts").unwrap_or(1000);
    let num_threads: usize = arg_value(&args, "--threads").unwrap_or(2);
    let crypto_us: u32 = arg_value(&args, "--crypto-us").unwrap_or(10);
    let hotspot_pct: f64 = arg_value(&args, "--hotspot").unwrap_or(90.0);
    let seed: u64 = arg_value(&args, "--seed").unwrap_or(42);

    // Calibrate crypto overhead under multi-threaded load.
    // On NUMA machines, single-threaded calibration at turbo boost overestimates
    // iters/us vs the all-core frequency used during actual execution.
    let cal_threads = num_threads.max(1);
    let cal_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(cal_threads)
        .build()
        .expect("Failed to create calibration pool");
    let measure_iters = 10_000u32;

    // Warmup all threads to stabilize CPU frequency.
    cal_pool.scope(|s| {
        for t in 0..cal_threads {
            s.spawn(move |_| {
                for i in 0..10u64 {
                    simulate_tx_crypto_work((t as u64) * 1000 + i, measure_iters);
                }
            });
        }
    });

    // Measure under load.
    let t0 = Instant::now();
    cal_pool.scope(|s| {
        for t in 0..cal_threads {
            s.spawn(move |_| {
                for i in 0..100u64 {
                    simulate_tx_crypto_work((t as u64) * 1000 + i, measure_iters);
                }
            });
        }
    });
    let elapsed_us = t0.elapsed().as_micros() as f64 / 100.0;
    let iters_per_us = measure_iters as f64 / elapsed_us;
    let crypto_iters = if crypto_us == 0 {
        0u32
    } else {
        ((crypto_us as f64 * iters_per_us).round() as u32).max(1)
    };

    eprintln!("=== LEAP E2E Panic Reproduction + Profiling ===");
    eprintln!("iterations={}, txns/iter={}, accounts={}, threads={}",
        iterations, num_txns, num_accounts, num_threads);
    eprintln!("crypto_us={}, crypto_iters={}, hotspot={}%, seed={}",
        crypto_us, crypto_iters, hotspot_pct, seed);
    eprintln!();

    let hotspot = HotspotConfig::Explicit {
        num_hotspots: 1,
        hotspot_ratio: hotspot_pct / 100.0,
    };
    let generator = StablecoinWorkloadGenerator::new(num_accounts, hotspot);
    let config = LeapConfig {
        num_workers: num_threads,
        ..LeapConfig::full()
    };

    // Create executor ONCE, reuse across iterations (same as E2E).
    let executor = Arc::new(Mutex::new(
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config.clone()),
    ));

    let mut panics = 0usize;
    let mut errors = 0usize;
    let mut successes = 0usize;
    let mut total_ok = 0usize;
    let mut total_fail = 0usize;

    // Timing accumulators (microseconds).
    let mut t_generate = 0u128;
    let mut t_cado = 0u128;
    let mut t_hotdelta = 0u128;
    let mut t_domainplan = 0u128;
    let mut t_set_bounds = 0u128;
    let mut t_execute = 0u128;
    let mut t_total = 0u128;

    for i in 0..iterations {
        let iter_start = Instant::now();

        // 1. Generate transactions (seeded, like E2E).
        let t = Instant::now();
        let mut txns = generator.generate_seeded(num_txns, seed + i as u64 + 1);
        t_generate += t.elapsed().as_micros();

        // 2. CADO ordering.
        let t = Instant::now();
        cado_ordering(&mut txns);
        t_cado += t.elapsed().as_micros();
        let num_after_cado = txns.len();

        // 3. HotDelta detection.
        let t = Instant::now();
        let mut mgr = HotDeltaManager::new(config.theta_1, config.theta_2, config.p_max);
        mgr.detect_hotspots(&txns);
        let hot_delta = Some(Arc::new(mgr));
        t_hotdelta += t.elapsed().as_micros();

        // 4. Domain-aware plan.
        let t = Instant::now();
        let plan = build_domain_plan(&txns, config.l_max);
        let bounds = plan.segment_bounds();
        let txn_to_seg = plan.txn_to_segment(txns.len());
        t_domainplan += t.elapsed().as_micros();

        // 5. Set segment bounds on executor (needs mut).
        let t = Instant::now();
        {
            let mut exec = executor.lock().unwrap_or_else(|e| e.into_inner());
            exec.set_segment_bounds(bounds, txn_to_seg);
        }
        t_set_bounds += t.elapsed().as_micros();

        // 6. Execute with catch_unwind.
        let t = Instant::now();
        let executor_clone = Arc::clone(&executor);
        let args = StablecoinExecArgs {
            crypto_work_iters: crypto_iters,
            hot_delta,
            funded_balance: 1_000_000,
        };

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let exec = executor_clone.lock().unwrap_or_else(|e| e.into_inner());
            exec.execute_transactions_parallel(args, txns)
        }));
        t_execute += t.elapsed().as_micros();

        let elapsed_ms = iter_start.elapsed().as_millis();
        t_total += iter_start.elapsed().as_micros();

        match result {
            Ok(Ok(outputs)) => {
                let counts = count_parallel_outcomes(&outputs);
                successes += 1;
                total_ok += counts.successful;
                total_fail += counts.total - counts.successful;
                if i % 50 == 0 || i < 3 {
                    eprintln!(
                        "[iter {:>3}] OK  txns={}->{} ok={} fail={} {:>4}ms",
                        i, num_txns, num_after_cado, counts.successful,
                        counts.total - counts.successful, elapsed_ms
                    );
                }
            }
            Ok(Err(e)) => {
                errors += 1;
                eprintln!(
                    "[iter {:>3}] ERR txns={}->{} error='{:?}' {:>4}ms",
                    i, num_txns, num_after_cado, e, elapsed_ms
                );
            }
            Err(panic_info) => {
                panics += 1;
                let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    format!("{:?}", panic_info)
                };
                eprintln!(
                    "[iter {:>3}] PANIC txns={}->{} panic='{}' {:>4}ms",
                    i, num_txns, num_after_cado, msg, elapsed_ms
                );
            }
        }
    }

    let n = iterations as f64;
    eprintln!();
    eprintln!("=== PER-CERTIFICATE OVERHEAD (avg over {} iterations) ===", iterations);
    eprintln!("  Generate:     {:>8.1} us", t_generate as f64 / n);
    eprintln!("  CADO order:   {:>8.1} us", t_cado as f64 / n);
    eprintln!("  HotDelta:     {:>8.1} us", t_hotdelta as f64 / n);
    eprintln!("  DomainPlan:   {:>8.1} us", t_domainplan as f64 / n);
    eprintln!("  SetBounds:    {:>8.1} us", t_set_bounds as f64 / n);
    eprintln!("  Execute:      {:>8.1} us", t_execute as f64 / n);
    eprintln!("  ---");
    eprintln!("  Total/cert:   {:>8.1} us ({:.1} ms)", t_total as f64 / n, t_total as f64 / n / 1000.0);
    eprintln!();
    let certs_per_sec = n / (t_total as f64 / 1e6);
    let effective_tps = certs_per_sec * num_txns as f64;
    eprintln!("  Certs/sec:    {:.1}", certs_per_sec);
    eprintln!("  Effective TPS: {:.0} tx/s", effective_tps);
    eprintln!("  (In 60s window: {:.0} certs, {:.0} txns)", certs_per_sec * 60.0, effective_tps * 60.0);
    eprintln!();

    eprintln!("=== RESULTS ===");
    eprintln!("Successes: {} / {}", successes, iterations);
    eprintln!("Errors: {}", errors);
    eprintln!("Panics: {}", panics);
    eprintln!("Total OK txns: {}", total_ok);
    eprintln!("Total FAIL txns: {}", total_fail);
    if total_ok + total_fail > 0 {
        eprintln!(
            "Success rate: {:.4}",
            total_ok as f64 / (total_ok + total_fail) as f64
        );
    }

    if panics > 0 {
        eprintln!();
        eprintln!("!!! {} PANICS DETECTED !!!", panics);
        std::process::exit(1);
    }
}

fn arg_value<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}
