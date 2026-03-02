use leap::{
    cado::{cado_ordering, cado_with_mode},
    config::{CadoMode, LeapConfig},
    domain_plan::build_domain_plan,
    executor::ParallelTransactionExecutor,
    hot_delta::HotDeltaManager,
    stablecoin::*,
    task::TransactionOutput,
};
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Engine configurations
// ---------------------------------------------------------------------------

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
            name: "LEAP-base+CADO",
            config: LeapConfig {
                cado_mode: CadoMode::Interleave,
                ..LeapConfig::baseline()
            },
        },
        EngineSpec {
            name: "LEAP",
            config: LeapConfig::full(),
        },
        EngineSpec {
            name: "LEAP-concat",
            config: LeapConfig::full_concat(),
        },
        EngineSpec {
            name: "LEAP-noDomain",
            config: LeapConfig {
                enable_domain_aware: false,
                ..LeapConfig::full_concat()
            },
        },
        EngineSpec {
            name: "LEAP-noHotDelta",
            config: LeapConfig {
                enable_hot_delta: false,
                ..LeapConfig::full()
            },
        },
        EngineSpec {
            name: "LEAP-noBP",
            config: LeapConfig {
                enable_backpressure: false,
                ..LeapConfig::full()
            },
        },
    ]
}

// ---------------------------------------------------------------------------
// Execute with a specific engine and return final balances
// ---------------------------------------------------------------------------

/// Returns (balances, whether CADO was actually applied).
fn run_engine(
    spec: &EngineSpec,
    txns_original: &[StablecoinTx],
    num_threads: usize,
) -> (HashMap<u64, u64>, bool) {
    let mut txns = txns_original.to_vec();

    let config = LeapConfig {
        num_workers: num_threads,
        ..spec.config.clone()
    };

    // Detect skew on original ordering before any reordering.
    let hot_delta = if config.enable_hot_delta {
        let mut mgr = HotDeltaManager::new(config.theta_1, config.theta_2, config.p_max);
        mgr.detect_hotspots(&txns);
        if mgr.is_skewed() { Some(Arc::new(mgr)) } else { None }
    } else {
        None
    };

    // Skip CADO when HotDelta is enabled but workload isn't skewed.
    let use_cado = !(config.enable_hot_delta && hot_delta.is_none());
    let cado_applied = use_cado && config.use_cado();

    let mut executor =
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config.clone());
    if use_cado {
        cado_with_mode(&mut txns, &config.cado_mode);
        if config.enable_domain_aware && config.cado_mode == CadoMode::Concatenate {
            let plan = build_domain_plan(&txns, config.l_max);
            let num_txns_total = txns.len();
            executor.set_segment_bounds(plan.segment_bounds(), plan.txn_to_segment(num_txns_total));
        }
    }

    let args = StablecoinExecArgs {
        crypto_work_iters: 0,
        hot_delta: hot_delta.clone(),
        funded_balance: 0,
    };

    let outputs = executor
        .execute_transactions_parallel(args, txns)
        .expect("Parallel execution should succeed");

    let mut state: HashMap<StateKey, StateValue> = HashMap::new();
    for output in &outputs {
        for (k, v) in output.get_writes() {
            state.insert(k, v);
        }
    }

    // Aggregate delta shards into balances if hot-delta was used.
    if hot_delta.is_some() {
        let delta_entries: Vec<(u64, u64, u64)> = state
            .iter()
            .filter_map(|(k, &v)| {
                if let StateKey::Delta(account, shard) = k {
                    Some((*account, *shard, v))
                } else {
                    None
                }
            })
            .collect();

        for (account, _shard, delta_val) in &delta_entries {
            let bal = state.entry(StateKey::Balance(*account)).or_insert(0);
            *bal += delta_val;
        }
        for (account, shard, _) in &delta_entries {
            state.remove(&StateKey::Delta(*account, *shard));
        }
    }

    (extract_balances(&state), cado_applied)
}

fn run_serial(txns: &[StablecoinTx]) -> HashMap<u64, u64> {
    let state = serial_execute(txns, 0);
    extract_balances(&state)
}

fn extract_balances(state: &HashMap<StateKey, StateValue>) -> HashMap<u64, u64> {
    state
        .iter()
        .filter_map(|(k, &v)| {
            if let StateKey::Balance(account) = k {
                Some((*account, v))
            } else {
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let num_accounts = 10;
    let num_transfers = 50;
    let initial_balance = 10_000u64;
    let num_threads = 4;

    println!("Correctness Check: Per-Account Balances Across All Engines");
    println!("==========================================================");
    println!(
        "  Accounts: {}, Transfers: {}, Init balance: {}, Threads: {}",
        num_accounts, num_transfers, initial_balance, num_threads
    );
    println!();

    let all_engines = engine_specs();

    // Two scenarios
    let scenarios: Vec<(&str, HotspotConfig)> = vec![
        ("Uniform", HotspotConfig::Uniform),
        (
            "Hotspot_90pct",
            HotspotConfig::Explicit {
                num_hotspots: 1,
                hotspot_ratio: 0.9,
            },
        ),
    ];

    // CSV output buffer
    let mut csv_buf = String::from("scenario,engine,account,balance\n");

    for (scenario_name, hotspot_config) in &scenarios {
        let gen = StablecoinWorkloadGenerator::new(num_accounts, hotspot_config.clone());
        let txns = gen.generate_with_funding(num_transfers, initial_balance);

        // Serial references
        let serial_orig = run_serial(&txns);
        let mut txns_cado = txns.clone();
        cado_ordering(&mut txns_cado);
        let serial_cado = run_serial(&txns_cado);

        // Collect all engine results: (name, balances, serial_ref_name)
        let mut results: Vec<(&str, HashMap<u64, u64>, &str)> = Vec::new();
        results.push(("Serial", serial_orig.clone(), "—"));
        results.push(("Serial+CADO", serial_cado.clone(), "—"));
        for spec in &all_engines {
            let (bals, cado_applied) = run_engine(spec, &txns, num_threads);
            let ref_name = if cado_applied { "Serial+CADO" } else { "Serial" };
            results.push((spec.name, bals, ref_name));
        }

        // Collect all account ids
        let all_accounts: Vec<u64> = {
            let mut set = BTreeSet::new();
            for (_, bals, _) in &results {
                for &a in bals.keys() {
                    set.insert(a);
                }
            }
            set.into_iter().collect()
        };

        // ---- Print table ----
        println!("╔══════════════════════════════════════════════════════════════════╗");
        println!("║  Scenario: {:<54}║", scenario_name);
        println!("║  Txns: {} funding + {} transfers = {} total{:>20}║",
                 txns.len() - num_transfers, num_transfers, txns.len(), "");
        println!("╚══════════════════════════════════════════════════════════════════╝");
        println!();

        // Header
        let col_w = 16;
        print!("{:>8}", "Acct");
        for (name, _, _) in &results {
            print!(" {:>width$}", name, width = col_w);
        }
        println!();

        // Separator
        print!("{:>8}", "--------");
        for _ in &results {
            print!(" {:>width$}", "----------------", width = col_w);
        }
        println!();

        // Data rows
        for &acct in &all_accounts {
            print!("{:>8}", acct);
            for (_, bals, _) in &results {
                let bal = bals.get(&acct).copied().unwrap_or(0);
                print!(" {:>width$}", bal, width = col_w);
            }
            println!();
        }
        println!();

        // ---- Diff check ----
        println!("Correctness (parallel vs serial reference):");
        for (name, bals, ref_name) in &results {
            if *ref_name == "—" {
                continue; // serial references, skip
            }
            let serial_ref = if *ref_name == "Serial+CADO" { &serial_cado } else { &serial_orig };
            let mut mismatches = 0;
            for &acct in &all_accounts {
                let s = serial_ref.get(&acct).copied().unwrap_or(0);
                let e = bals.get(&acct).copied().unwrap_or(0);
                if s != e {
                    mismatches += 1;
                }
            }
            let status = if mismatches == 0 { "PASS" } else { "FAIL" };
            println!("  {:<18} vs {:<14} => {} (mismatches: {})",
                     name, ref_name, status, mismatches);
        }
        println!();
        println!();

        // CSV
        for (name, bals, _) in &results {
            for &acct in &all_accounts {
                let bal = bals.get(&acct).copied().unwrap_or(0);
                csv_buf.push_str(&format!("{},{},{},{}\n", scenario_name, name, acct, bal));
            }
        }
    }

    // Write CSV
    let out_dir = "correctness_results";
    std::fs::create_dir_all(out_dir).unwrap();
    let csv_path = format!("{}/correctness_detail.csv", out_dir);
    let mut f = std::fs::File::create(&csv_path).unwrap();
    f.write_all(csv_buf.as_bytes()).unwrap();
    println!("CSV written to {}", csv_path);
}
