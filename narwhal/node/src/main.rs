// Copyright(C) Facebook, Inc. and its affiliates.
use anyhow::{Context, Result};
use clap::{crate_name, crate_version, App, AppSettings, ArgMatches, SubCommand};
use config::Export as _;
use config::Import as _;
use config::{Committee, KeyPair, Parameters, WorkerId};
use consensus::Consensus;
#[cfg(feature = "mp3bft")]
use consensus::MP3Consensus;
use env_logger::Env;
#[allow(unused_imports)]
use log::info;
use primary::{Certificate, Primary};
use store::Store;
use tokio::sync::mpsc::{channel, Receiver};
use worker::Worker;

#[cfg(feature = "e2e_exec")]
use std::time::Instant;

/// The default channel capacity.
pub const CHANNEL_CAPACITY: usize = 1_000;

fn main() -> Result<()> {
    // When e2e_exec is enabled, limit tokio worker threads to avoid CPU
    // contention with rayon. 4 tokio threads suffice for network I/O;
    // remaining cores are reserved for the parallel executor.
    // Without e2e_exec, use all CPUs for maximum consensus throughput.
    let default_tokio: usize = if cfg!(feature = "e2e_exec") { 4 } else { 16 };
    let tokio_threads: usize = std::env::var("TOKIO_WORKER_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_tokio);

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(tokio_threads)
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime")
        .block_on(async_main(tokio_threads))
}

async fn async_main(tokio_threads: usize) -> Result<()> {
    let matches = App::new(crate_name!())
        .version(crate_version!())
        .about("A research implementation of Narwhal and Tusk.")
        .args_from_usage("-v... 'Sets the level of verbosity'")
        .subcommand(
            SubCommand::with_name("generate_keys")
                .about("Print a fresh key pair to file")
                .args_from_usage("--filename=<FILE> 'The file where to print the new key pair'"),
        )
        .subcommand(
            SubCommand::with_name("run")
                .about("Run a node")
                .args_from_usage("--keys=<FILE> 'The file containing the node keys'")
                .args_from_usage("--committee=<FILE> 'The file containing committee information'")
                .args_from_usage("--parameters=[FILE] 'The file containing the node parameters'")
                .args_from_usage("--store=<PATH> 'The path where to create the data store'")
                .subcommand(SubCommand::with_name("primary").about("Run a single primary"))
                .subcommand(
                    SubCommand::with_name("worker")
                        .about("Run a single worker")
                        .args_from_usage("--id=<INT> 'The worker id'"),
                )
                .setting(AppSettings::SubcommandRequiredElseHelp),
        )
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .get_matches();

    let log_level = match matches.occurrences_of("v") {
        0 => "error",
        1 => "warn",
        2 => "info",
        3 => "debug",
        _ => "trace",
    };
    let mut logger = env_logger::Builder::from_env(Env::default().default_filter_or(log_level));
    #[cfg(feature = "benchmark")]
    logger.format_timestamp_millis();
    logger.init();

    match matches.subcommand() {
        ("generate_keys", Some(sub_matches)) => KeyPair::new()
            .export(sub_matches.value_of("filename").unwrap())
            .context("Failed to generate key pair")?,
        ("run", Some(sub_matches)) => {
            info!("Tokio worker threads: {}", tokio_threads);
            run(sub_matches).await?
        }
        _ => unreachable!(),
    }
    Ok(())
}

// Runs either a worker or a primary.
async fn run(matches: &ArgMatches<'_>) -> Result<()> {
    let key_file = matches.value_of("keys").unwrap();
    let committee_file = matches.value_of("committee").unwrap();
    let parameters_file = matches.value_of("parameters");
    let store_path = matches.value_of("store").unwrap();

    // Read the committee and node's keypair from file.
    let keypair = KeyPair::import(key_file).context("Failed to load the node's keypair")?;
    let committee =
        Committee::import(committee_file).context("Failed to load the committee information")?;

    // Load default parameters if none are specified.
    let parameters = match parameters_file {
        Some(filename) => {
            Parameters::import(filename).context("Failed to load the node's parameters")?
        }
        None => Parameters::default(),
    };

    // Capture batch_size before parameters may be moved.
    let batch_size = parameters.batch_size;

    // Make the data store.
    let store = Store::new(store_path).context("Failed to create a store")?;

    // Channels the sequence of certificates.
    let (tx_output, rx_output) = channel(CHANNEL_CAPACITY);

    // Check whether to run a primary, a worker, or an entire authority.
    match matches.subcommand() {
        // Spawn the primary and consensus core.
        ("primary", _) => {
            let (tx_new_certificates, rx_new_certificates) = channel(CHANNEL_CAPACITY);
            let (tx_feedback, rx_feedback) = channel(CHANNEL_CAPACITY);
            Primary::spawn(
                keypair,
                committee.clone(),
                parameters.clone(),
                store,
                /* tx_consensus */ tx_new_certificates,
                /* rx_consensus */ rx_feedback,
            );
            // Consensus selection: runtime via CONSENSUS_PROTOCOL env var when
            // compiled with mp3bft feature. This allows a single binary to run
            // either Tusk or MP3-BFT++ in distributed deployments.
            #[cfg(feature = "mp3bft")]
            {
                let use_tusk = std::env::var("CONSENSUS_PROTOCOL")
                    .map(|v| v.eq_ignore_ascii_case("tusk"))
                    .unwrap_or(false);

                if use_tusk {
                    info!("Using Tusk consensus (CONSENSUS_PROTOCOL=tusk)");
                    Consensus::spawn(
                        committee,
                        parameters.gc_depth,
                        /* rx_primary */ rx_new_certificates,
                        /* tx_primary */ tx_feedback,
                        tx_output,
                    );
                } else {
                    let k_slots: usize = std::env::var("MP3BFT_K_SLOTS")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(4);
                    info!("Using MP3-BFT++ consensus with k={} slots", k_slots);
                    MP3Consensus::spawn(
                        committee,
                        parameters.gc_depth,
                        k_slots,
                        /* rx_primary */ rx_new_certificates,
                        /* tx_primary */ tx_feedback,
                        tx_output,
                    );
                }
            }
            #[cfg(not(feature = "mp3bft"))]
            Consensus::spawn(
                committee,
                parameters.gc_depth,
                /* rx_primary */ rx_new_certificates,
                /* tx_primary */ tx_feedback,
                tx_output,
            );
        }

        // Spawn a single worker.
        ("worker", Some(sub_matches)) => {
            let id = sub_matches
                .value_of("id")
                .unwrap()
                .parse::<WorkerId>()
                .context("The worker id must be a positive integer")?;
            Worker::spawn(keypair.name, id, committee, parameters, store);
        }
        _ => unreachable!(),
    }

    // Analyze the consensus' output on a dedicated OS thread.
    // This avoids tokio spawn_blocking overhead per certificate.
    std::thread::Builder::new()
        .name("leap-executor".to_string())
        .spawn(move || analyze(rx_output, batch_size))
        .expect("Failed to spawn executor thread");

    // Keep the tokio runtime alive (process runs until killed externally).
    std::future::pending::<()>().await;

    // If this expression is reached, the program ends and all other tasks terminate.
    unreachable!();
}

/// Receives an ordered list of certificates and apply any application-specific logic.
#[cfg(not(feature = "e2e_exec"))]
fn analyze(mut rx_output: Receiver<Certificate>, _batch_size: usize) {
    while let Some(_certificate) = rx_output.blocking_recv() {
        // NOTE: Here goes the application logic.
    }
}

/// With e2e_exec: receives committed certificates and executes stablecoin txns via LEAP.
/// Runs on a dedicated OS thread (not tokio) to avoid spawn_blocking overhead per certificate.
#[cfg(feature = "e2e_exec")]
fn analyze(mut rx_output: Receiver<Certificate>, batch_size: usize) {
    use leap::cado::cado_ordering;
    use leap::config::LeapConfig;
    use leap::domain_plan::build_domain_plan;
    use leap::executor::ParallelTransactionExecutor;
    use leap::hot_delta::HotDeltaManager;
    use leap::stablecoin::{
        count_parallel_outcomes, serial_execute_counted, ExecCounts,
        HotspotConfig, StablecoinExecArgs, StablecoinExecutor,
        StablecoinTx, StablecoinWorkloadGenerator,
    };
    use std::sync::Arc;

    // Read configuration from environment variables.
    let engine = std::env::var("LEAP_ENGINE").unwrap_or_else(|_| "leap".into());
    let num_threads: usize = std::env::var("LEAP_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);

    // Sync rayon's global thread pool with LEAP_THREADS when not explicitly set.
    // Prevents rayon from defaulting to num_cpus on localhost multi-node setups.
    if std::env::var("RAYON_NUM_THREADS").is_err() {
        std::env::set_var("RAYON_NUM_THREADS", num_threads.to_string());
    }
    let crypto_us: u32 = std::env::var("LEAP_CRYPTO_US")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let num_accounts: usize = std::env::var("LEAP_ACCOUNTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let pattern_str = std::env::var("LEAP_PATTERN").unwrap_or_else(|_| "Uniform".into());
    let tx_size: usize = std::env::var("BENCH_TX_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(512);

    // Convert crypto_us to SHA-256 iterations via runtime calibration.
    // Avoids hardcoding 62ns/iter which varies across CPU models and clock speeds.
    let crypto_iters = {
        use leap::stablecoin::simulate_tx_crypto_work;
        // Warmup
        let _ = simulate_tx_crypto_work(42, 1000);
        // Measure
        let measure_iters = 10_000u32;
        let t0 = std::time::Instant::now();
        let _ = simulate_tx_crypto_work(42, measure_iters);
        let elapsed_us = t0.elapsed().as_secs_f64() * 1e6;
        let iters_per_us = measure_iters as f64 / elapsed_us;
        let result = if crypto_us == 0 { 0u32 } else {
            ((crypto_us as f64 * iters_per_us).round() as u32).max(1)
        };
        log::info!("SHA-256 calibration: {:.1} iters/μs ({:.0} ns/iter) → {}us = {} iters",
            iters_per_us, 1000.0 / iters_per_us, crypto_us, result);
        result
    };

    let hotspot = match pattern_str.as_str() {
        "Uniform" => HotspotConfig::Uniform,
        s if s.starts_with("Zipf_") => {
            let alpha: f64 = s[5..].parse().unwrap_or(0.8);
            HotspotConfig::Zipf { alpha }
        }
        s if s.starts_with("Hotspot_") => {
            let pct: f64 = s.trim_start_matches("Hotspot_")
                .trim_end_matches("pct")
                .parse()
                .unwrap_or(50.0);
            HotspotConfig::Explicit {
                num_hotspots: 1,
                hotspot_ratio: pct / 100.0,
            }
        }
        _ => HotspotConfig::Uniform,
    };

    let generator = StablecoinWorkloadGenerator::new(num_accounts, hotspot);

    // Fixed seed for deterministic transaction generation across runs.
    // Same seed + same certificate index = identical transactions,
    // ensuring fair A/B comparison between different execution engines.
    let base_seed: Option<u64> = std::env::var("LEAP_SEED")
        .ok()
        .and_then(|v| v.parse().ok());
    let mut cert_counter: u64 = 0;

    let rayon_threads = std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "?".into());
    info!(
        "E2E execution: engine={}, threads={}, rayon={}, crypto_us={}, accounts={}, pattern={}, tx_size={}, seed={:?}",
        engine, num_threads, rayon_threads, crypto_us, num_accounts, pattern_str, tx_size, base_seed
    );

    // Create config and executor ONCE before the loop so backpressure can adapt across certificates.
    let leap_config: Option<LeapConfig> = match engine.as_str() {
        "leap" => Some(LeapConfig {
            num_workers: num_threads,
            ..LeapConfig::full()
        }),
        "leap_base" => Some(LeapConfig {
            num_workers: num_threads,
            ..LeapConfig::baseline()
        }),
        _ => None,
    };

    // No Arc<Mutex<>> needed — executor lives on this dedicated thread only.
    let mut executor: Option<ParallelTransactionExecutor<StablecoinTx, StablecoinExecutor>> =
        leap_config.as_ref().map(|config| {
            ParallelTransactionExecutor::with_config(config.clone())
        });

    let mut recv_start = Instant::now();

    while let Some(certificate) = rx_output.blocking_recv() {
        let recv_ms = recv_start.elapsed().as_millis();

        let num_batches = certificate.header.payload.len();
        let num_txns = num_batches * batch_size / tx_size;
        if num_txns == 0 {
            recv_start = Instant::now();
            continue;
        }

        let round = certificate.header.round;
        let batch_digests: Vec<crypto::Digest> =
            certificate.header.payload.keys().cloned().collect();

        // Generate transfer-only stablecoin transactions.
        // Accounts are pre-funded via funded_balance (simulates persistent state).
        cert_counter += 1;
        let gen_start = Instant::now();
        let mut txns: Vec<StablecoinTx> = match base_seed {
            Some(seed) => generator.generate_seeded(num_txns, seed + cert_counter),
            None => generator.generate(num_txns),
        };
        let gen_ms = gen_start.elapsed().as_millis();

        let exec_start = Instant::now();

        // Execute directly on this thread (no spawn_blocking overhead).
        // Wrap in catch_unwind to prevent a rayon panic from killing the thread.
        let exec_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> (ExecCounts, u128, u128) {
            match engine.as_str() {
                "leap" => {
                    let prep_start = Instant::now();
                    let config = leap_config.as_ref().unwrap();
                    let exec = executor.as_mut().unwrap();

                    // Hot-Delta: detect hotspots BEFORE CADO ordering.
                    // If no hot accounts exist (e.g. Uniform pattern), skip
                    // CADO+HotDelta+DomainPlan entirely — their overhead
                    // creates artificial conflict bursts that hurt performance.
                    let mut mgr = HotDeltaManager::new(
                        config.theta_1, config.theta_2, config.p_max,
                    );
                    mgr.detect_hotspots(&txns);
                    let has_hotspots = !mgr.hot_accounts().is_empty();

                    let (hot_delta, bounds, txn_seg) = if has_hotspots {
                        // Full LEAP stack: CADO → HotDelta → DomainPlan.
                        cado_ordering(&mut txns);
                        let plan = build_domain_plan(&txns, config.l_max);
                        let b = plan.segment_bounds();
                        let s = plan.txn_to_segment(txns.len());
                        (Some(Arc::new(mgr)), b, s)
                    } else {
                        // No hotspots: run as plain Block-STM (no CADO overhead).
                        (None, vec![], vec![])
                    };

                    let args = StablecoinExecArgs {
                        crypto_work_iters: crypto_iters,
                        hot_delta,
                        funded_balance: 1_000_000,
                    };

                    exec.set_segment_bounds(bounds, txn_seg);
                    let prep_ms = prep_start.elapsed().as_millis();
                    let run_start = Instant::now();
                    let counts = match exec.execute_transactions_parallel(args, txns) {
                        Ok(outputs) => count_parallel_outcomes(&outputs),
                        Err(_) => ExecCounts::default(),
                    };
                    let run_ms = run_start.elapsed().as_millis();
                    (counts, prep_ms, run_ms)
                }
                "leap_base" => {
                    // No CADO ordering: baseline uses random transaction order,
                    // matching Exp-1 where LEAP-base has use_cado=false.
                    let exec = executor.as_mut().unwrap();
                    let args = StablecoinExecArgs {
                        crypto_work_iters: crypto_iters,
                        hot_delta: None,
                        funded_balance: 1_000_000,
                    };
                    let run_start = Instant::now();
                    let counts = match exec.execute_transactions_parallel(args, txns) {
                        Ok(outputs) => count_parallel_outcomes(&outputs),
                        Err(_) => ExecCounts::default(),
                    };
                    let run_ms = run_start.elapsed().as_millis();
                    (counts, 0, run_ms)
                }
                _ => {
                    // serial
                    let run_start = Instant::now();
                    let (_state, counts) = serial_execute_counted(&txns, crypto_iters, 1_000_000);
                    let run_ms = run_start.elapsed().as_millis();
                    (counts, 0, run_ms)
                }
            }
        }));

        let exec_ms = exec_start.elapsed().as_millis();

        match exec_result {
            Err(_) => {
                info!("Execution error B{}({}) : panic in executor", round, num_txns);
                recv_start = Instant::now();
                continue;
            }
            Ok((counts, prep_ms, run_ms)) => {
                // Log execution stats (one line per certificate, for TPS/success rate).
                // Phase timing: recv_ms=wait for channel, gen_ms=txn generation,
                // prep_ms=CADO+HotDelta+DomainPlan, run_ms=parallel execution.
                info!(
                    "ExecStats B{} total={} ok={} fail={} exec_ms={} recv_ms={} gen_ms={} prep_ms={} run_ms={}",
                    round, counts.total, counts.successful,
                    counts.total - counts.successful, exec_ms,
                    recv_ms, gen_ms, prep_ms, run_ms
                );
            }
        }

        // Log per-batch digest (same format as "Committed") for the parser.
        for digest in &batch_digests {
            info!(
                "Executed B{}({}) -> {:?} in {} ms",
                round, num_txns, digest, exec_ms
            );
        }

        recv_start = Instant::now();
    }
}
