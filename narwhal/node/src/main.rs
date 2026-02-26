// Copyright(C) Facebook, Inc. and its affiliates.
use anyhow::{Context, Result};
use clap::{crate_name, crate_version, App, AppSettings, ArgMatches, SubCommand};
use config::Export as _;
use config::Import as _;
use config::{Committee, KeyPair, Parameters, WorkerId};
#[cfg(not(feature = "mp3bft"))]
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

#[tokio::main]
async fn main() -> Result<()> {
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
        ("run", Some(sub_matches)) => run(sub_matches).await?,
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
            // MP3-BFT++ consensus: multi-slot ordering with 3-chain commit rule.
            // Tusk consensus: single-leader DAG ordering (default).
            #[cfg(feature = "mp3bft")]
            {
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

    // Analyze the consensus' output.
    analyze(rx_output, batch_size).await;

    // If this expression is reached, the program ends and all other tasks terminate.
    unreachable!();
}

/// Receives an ordered list of certificates and apply any application-specific logic.
#[cfg(not(feature = "e2e_exec"))]
async fn analyze(mut rx_output: Receiver<Certificate>, _batch_size: usize) {
    while let Some(_certificate) = rx_output.recv().await {
        // NOTE: Here goes the application logic.
    }
}

/// With e2e_exec: receives committed certificates and executes stablecoin txns via LEAP.
#[cfg(feature = "e2e_exec")]
async fn analyze(mut rx_output: Receiver<Certificate>, batch_size: usize) {
    use leap::cado::cado_ordering;
    use leap::config::LeapConfig;
    use leap::domain_plan::build_domain_plan;
    use leap::executor::ParallelTransactionExecutor;
    use leap::hot_delta::HotDeltaManager;
    use leap::stablecoin::{
        serial_execute, HotspotConfig, StablecoinExecArgs, StablecoinExecutor,
        StablecoinTx, StablecoinWorkloadGenerator,
    };
    use std::sync::Arc;

    // Read configuration from environment variables.
    let engine = std::env::var("LEAP_ENGINE").unwrap_or_else(|_| "leap".into());
    let num_threads: usize = std::env::var("LEAP_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
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

    // Convert crypto_us to SHA-256 iterations (~62ns per iter on this hardware).
    let crypto_iters = (crypto_us as f64 / 0.062) as u32;

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

    info!(
        "E2E execution: engine={}, threads={}, crypto_us={}, accounts={}, pattern={}, tx_size={}",
        engine, num_threads, crypto_us, num_accounts, pattern_str, tx_size
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

    let shared_executor: Option<
        Arc<std::sync::Mutex<ParallelTransactionExecutor<StablecoinTx, StablecoinExecutor>>>,
    > = leap_config.as_ref().map(|config| {
        Arc::new(std::sync::Mutex::new(
            ParallelTransactionExecutor::with_config(config.clone()),
        ))
    });

    while let Some(certificate) = rx_output.recv().await {
        let num_batches = certificate.header.payload.len();
        let num_txns = num_batches * batch_size / tx_size;
        if num_txns == 0 {
            continue;
        }

        let round = certificate.header.round;
        let batch_digests: Vec<crypto::Digest> =
            certificate.header.payload.keys().cloned().collect();

        // Generate funded stablecoin transactions matching the batch count.
        let mut txns: Vec<StablecoinTx> = generator.generate_with_funding(num_txns, 1_000_000);

        let engine_clone = engine.clone();
        let executor_clone = shared_executor.clone();
        let config_clone = leap_config.clone();
        let exec_start = Instant::now();

        // Run execution on a blocking thread (LEAP uses rayon internally).
        let exec_result = tokio::task::spawn_blocking(move || {
            match engine_clone.as_str() {
                "leap" => {
                    cado_ordering(&mut txns);
                    let config = config_clone.unwrap();
                    let executor_arc = executor_clone.unwrap();
                    let mut executor = executor_arc.lock().unwrap();

                    // Hot-Delta: detect hotspots.
                    let mut mgr = HotDeltaManager::new(
                        config.theta_1, config.theta_2, config.p_max,
                    );
                    mgr.detect_hotspots(&txns);
                    let hot_delta = Some(Arc::new(mgr));

                    // Domain-Aware: build segment plan.
                    let plan = build_domain_plan(&txns, config.l_max);
                    let bounds = plan.segment_bounds();
                    let num_txns_total = txns.len();

                    let args = StablecoinExecArgs {
                        crypto_work_iters: crypto_iters,
                        hot_delta,
                    };

                    executor.set_segment_bounds(bounds, plan.txn_to_segment(num_txns_total));
                    let _outputs = executor
                        .execute_transactions_parallel(args, txns);
                }
                "leap_base" => {
                    cado_ordering(&mut txns);
                    let executor_arc = executor_clone.unwrap();
                    let executor = executor_arc.lock().unwrap();
                    let args = StablecoinExecArgs {
                        crypto_work_iters: crypto_iters,
                        hot_delta: None,
                    };
                    let _outputs = executor
                        .execute_transactions_parallel(args, txns);
                }
                _ => {
                    // serial
                    let _state = serial_execute(&txns, crypto_iters);
                }
            }
        })
        .await;

        let exec_ms = exec_start.elapsed().as_millis();

        if let Err(e) = exec_result {
            info!("Execution error B{}({}) : {}", round, num_txns, e);
            continue;
        }

        // Log per-batch digest (same format as "Committed") for the parser.
        for digest in &batch_digests {
            info!(
                "Executed B{}({}) -> {:?} in {} ms",
                round, num_txns, digest, exec_ms
            );
        }
    }
}
