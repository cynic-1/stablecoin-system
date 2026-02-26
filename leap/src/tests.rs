use crate::{
    backpressure::BackpressureController,
    cado::cado_ordering,
    config::LeapConfig,
    domain_plan::build_domain_plan,
    executor::ParallelTransactionExecutor,
    hot_delta::HotDeltaManager,
    stablecoin::*,
    task::TransactionOutput,
};
use std::sync::Arc;

/// Helper: generate txns, set initial balances, and verify serial == parallel.
fn check_serial_equivalence(
    num_accounts: usize,
    num_txns: usize,
    hotspot: HotspotConfig,
    num_threads: usize,
) {
    let gen = StablecoinWorkloadGenerator::new(num_accounts, hotspot);
    let txns = gen.generate(num_txns);

    // Serial execution (0 iters = no crypto overhead for fast tests).
    let serial_state = serial_execute(&txns, 0);

    // Parallel execution.
    let parallel_state = parallel_execute_to_state(txns, num_threads, 0);

    // Compare all balance/nonce keys (ignore Delta keys from Hot-Delta).
    for (k, v) in &serial_state {
        match k {
            StateKey::Delta(_, _) => continue,
            _ => {
                let pv = parallel_state.get(k).unwrap_or(&0);
                assert_eq!(
                    v, pv,
                    "Mismatch on key {:?}: serial={}, parallel={}",
                    k, v, pv
                );
            }
        }
    }
}

#[test]
fn test_serial_equivalence_uniform_small() {
    check_serial_equivalence(100, 500, HotspotConfig::Uniform, 4);
}

#[test]
fn test_serial_equivalence_uniform_large() {
    check_serial_equivalence(1000, 2000, HotspotConfig::Uniform, 8);
}

#[test]
fn test_serial_equivalence_zipf() {
    check_serial_equivalence(100, 500, HotspotConfig::Zipf { alpha: 0.8 }, 4);
}

#[test]
fn test_serial_equivalence_hotspot() {
    check_serial_equivalence(
        100,
        500,
        HotspotConfig::Explicit {
            num_hotspots: 2,
            hotspot_ratio: 0.5,
        },
        4,
    );
}

#[test]
fn test_cado_then_execute() {
    let gen = StablecoinWorkloadGenerator::new(100, HotspotConfig::Uniform);
    let mut txns = gen.generate(500);

    // Apply CADO ordering.
    cado_ordering(&mut txns);

    // Serial execution on CADO-ordered sequence.
    let serial_state = serial_execute(&txns, 0);

    // Parallel execution on CADO-ordered sequence.
    let parallel_state = parallel_execute_to_state(txns, 4, 0);

    for (k, v) in &serial_state {
        match k {
            StateKey::Delta(_, _) => continue,
            _ => {
                let pv = parallel_state.get(k).unwrap_or(&0);
                assert_eq!(v, pv, "Mismatch after CADO on {:?}", k);
            }
        }
    }
}

#[test]
fn test_single_thread_correctness() {
    check_serial_equivalence(50, 200, HotspotConfig::Uniform, 1);
}

#[test]
fn test_multi_thread_scaling() {
    // Verify that increasing threads doesn't break correctness.
    for threads in [1, 2, 4, 8] {
        check_serial_equivalence(200, 1000, HotspotConfig::Uniform, threads);
    }
}

#[test]
fn test_empty_block() {
    let executor =
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::new();
    let result = executor.execute_transactions_parallel(0u32.into(), vec![]);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_mint_and_transfer() {
    let txns = vec![
        StablecoinTx {
            tx_type: StablecoinTxType::Mint {
                to: 1,
                amount: 1000,
            },
            nonce: 0,
            tx_hash: 1,
        },
        StablecoinTx {
            tx_type: StablecoinTxType::Transfer {
                sender: 1,
                receiver: 2,
                amount: 500,
            },
            nonce: 1,
            tx_hash: 2,
        },
    ];

    let serial_state = serial_execute(&txns, 0);
    assert_eq!(serial_state[&StateKey::Balance(1)], 500);
    assert_eq!(serial_state[&StateKey::Balance(2)], 500);
    assert_eq!(serial_state[&StateKey::TotalSupply], 1000);
}

// ---------------------------------------------------------------------------
// Integration tests: Hot-Delta sharding
// ---------------------------------------------------------------------------

#[test]
fn test_hot_delta_correctness_hotspot() {
    // With hotspot workload, Hot-Delta should produce same final balances as serial.
    let gen = StablecoinWorkloadGenerator::new(
        100,
        HotspotConfig::Explicit {
            num_hotspots: 2,
            hotspot_ratio: 0.9,
        },
    );
    let txns = gen.generate(500);

    let serial_state = serial_execute(&txns, 0);

    let mut mgr = HotDeltaManager::new(5, 30, 8);
    mgr.detect_hotspots(&txns);

    let parallel_state =
        parallel_execute_to_state_with_hot_delta(txns, 4, 0, Arc::new(mgr));

    // Compare balances and nonces (deltas aggregated in parallel_execute_to_state_with_hot_delta).
    for (k, v) in &serial_state {
        match k {
            StateKey::Delta(_, _) => continue,
            _ => {
                let pv = parallel_state.get(k).unwrap_or(&0);
                assert_eq!(
                    v, pv,
                    "Hot-Delta mismatch on key {:?}: serial={}, parallel={}",
                    k, v, pv
                );
            }
        }
    }
}

#[test]
fn test_hot_delta_correctness_uniform() {
    // Uniform workload: no accounts should be hot, behavior should match serial.
    let gen = StablecoinWorkloadGenerator::new(1000, HotspotConfig::Uniform);
    let txns = gen.generate(500);

    let serial_state = serial_execute(&txns, 0);

    let mut mgr = HotDeltaManager::new(10, 50, 8);
    mgr.detect_hotspots(&txns);
    // With 1000 accounts and 500 txns uniform, unlikely any account has >= 10 hits.

    let parallel_state =
        parallel_execute_to_state_with_hot_delta(txns, 4, 0, Arc::new(mgr));

    for (k, v) in &serial_state {
        match k {
            StateKey::Delta(_, _) => continue,
            _ => {
                let pv = parallel_state.get(k).unwrap_or(&0);
                assert_eq!(
                    v, pv,
                    "Hot-Delta uniform mismatch on key {:?}: serial={}, parallel={}",
                    k, v, pv
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Integration tests: Domain-Aware scheduling
// ---------------------------------------------------------------------------

#[test]
fn test_domain_aware_correctness() {
    let gen = StablecoinWorkloadGenerator::new(
        100,
        HotspotConfig::Explicit {
            num_hotspots: 3,
            hotspot_ratio: 0.8,
        },
    );
    let mut txns = gen.generate(500);
    cado_ordering(&mut txns);

    let serial_state = serial_execute(&txns, 0);

    // Build domain plan and execute with domain-aware scheduling.
    let plan = build_domain_plan(&txns, 64);
    let num_txns_total = txns.len();
    let bounds = plan.segment_bounds();
    let txn_to_seg = plan.txn_to_segment(num_txns_total);

    let config = LeapConfig {
        num_workers: 4,
        enable_domain_aware: true,
        enable_backpressure: true,
        enable_hot_delta: false,
        ..LeapConfig::default()
    };

    let args = StablecoinExecArgs {
        crypto_work_iters: 0,
        hot_delta: None,
    };

    let mut executor =
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config);
    executor.set_segment_bounds(bounds, txn_to_seg);
    let outputs = executor
        .execute_transactions_parallel(args, txns)
        .expect("Domain-aware execution should succeed");

    let mut parallel_state = std::collections::HashMap::new();
    for output in &outputs {
        for (k, v) in output.get_writes() {
            parallel_state.insert(k, v);
        }
    }

    for (k, v) in &serial_state {
        let pv = parallel_state.get(k).unwrap_or(&0);
        assert_eq!(
            v, pv,
            "Domain-aware mismatch on key {:?}: serial={}, parallel={}",
            k, v, pv
        );
    }
}

// ---------------------------------------------------------------------------
// Integration tests: All three optimizations combined
// ---------------------------------------------------------------------------

#[test]
fn test_all_optimizations_combined() {
    let gen = StablecoinWorkloadGenerator::new(
        100,
        HotspotConfig::Explicit {
            num_hotspots: 2,
            hotspot_ratio: 0.9,
        },
    );
    let mut txns = gen.generate(500);
    cado_ordering(&mut txns);

    let serial_state = serial_execute(&txns, 0);

    // Setup Hot-Delta.
    let mut mgr = HotDeltaManager::new(5, 30, 8);
    mgr.detect_hotspots(&txns);
    let mgr = Arc::new(mgr);

    // Setup Domain Plan.
    let plan = build_domain_plan(&txns, 64);
    let num_txns_total = txns.len();
    let bounds = plan.segment_bounds();
    let txn_to_seg = plan.txn_to_segment(num_txns_total);

    let config = LeapConfig {
        num_workers: 4,
        enable_domain_aware: true,
        enable_hot_delta: true,
        enable_backpressure: true,
        ..LeapConfig::default()
    };

    let args = StablecoinExecArgs {
        crypto_work_iters: 0,
        hot_delta: Some(mgr.clone()),
    };

    let mut executor =
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config);
    executor.set_segment_bounds(bounds, txn_to_seg);
    let outputs = executor
        .execute_transactions_parallel(args, txns)
        .expect("Combined execution should succeed");

    // Aggregate state including delta folding.
    let mut state = std::collections::HashMap::new();
    for output in &outputs {
        for (k, v) in output.get_writes() {
            state.insert(k, v);
        }
    }

    // Fold deltas into balances.
    let delta_entries: Vec<(StateKey, u64)> = state
        .iter()
        .filter_map(|(k, &v)| {
            if let StateKey::Delta(_, _) = k {
                Some((k.clone(), v))
            } else {
                None
            }
        })
        .collect();

    for (k, delta_val) in &delta_entries {
        if let StateKey::Delta(account, _) = k {
            let bal = state.entry(StateKey::Balance(*account)).or_insert(0);
            *bal += delta_val;
        }
    }
    for (k, _) in &delta_entries {
        state.remove(k);
    }

    for (k, v) in &serial_state {
        match k {
            StateKey::Delta(_, _) => continue,
            _ => {
                let pv = state.get(k).unwrap_or(&0);
                assert_eq!(
                    v, pv,
                    "Combined mismatch on key {:?}: serial={}, parallel={}",
                    k, v, pv
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Integration test: Adaptive backpressure adjusts window
// ---------------------------------------------------------------------------

#[test]
fn test_adaptive_backpressure_adjusts() {
    use crate::backpressure::BlockExecStats;

    let mut ctrl = BackpressureController::new(32, 4, 64);
    assert_eq!(ctrl.window(), 32);

    // Simulate high contention: window should shrink.
    ctrl.adjust(&BlockExecStats {
        total_executions: 100,
        total_aborts: 25,
        total_waits: 0,
    });
    assert!(ctrl.window() < 32, "Window should shrink on high abort rate");

    let shrunk = ctrl.window();

    // Simulate low contention: window should expand.
    ctrl.adjust(&BlockExecStats {
        total_executions: 100,
        total_aborts: 1,
        total_waits: 1,
    });
    assert!(
        ctrl.window() > shrunk,
        "Window should expand on low contention"
    );
}

// ---------------------------------------------------------------------------
// Funded serial-equivalence tests (transactions with real writes)
// ---------------------------------------------------------------------------

/// Helper: generate funded txns and verify serial == parallel produces non-empty state.
fn check_funded_serial_equivalence(config: LeapConfig, use_cado: bool, use_hot_delta: bool) {
    let gen = StablecoinWorkloadGenerator::new(50, HotspotConfig::Uniform);
    let mut txns = gen.generate_with_funding(200, 1_000_000);

    if use_cado {
        cado_ordering(&mut txns);
    }

    // Serial execution.
    let serial_state = serial_execute(&txns, 0);
    assert!(
        !serial_state.is_empty(),
        "Serial state must be non-empty with funded accounts"
    );
    // Verify some balances are non-zero.
    let non_zero = serial_state
        .iter()
        .filter(|(k, &v)| matches!(k, StateKey::Balance(_)) && v > 0)
        .count();
    assert!(non_zero > 0, "Must have non-zero balances after funded execution");

    // Parallel execution.
    let hot_delta = if use_hot_delta {
        let mut mgr = HotDeltaManager::new(5, 30, 8);
        mgr.detect_hotspots(&txns);
        Some(Arc::new(mgr))
    } else {
        None
    };

    let args = StablecoinExecArgs {
        crypto_work_iters: 0,
        hot_delta: hot_delta.clone(),
    };

    let mut executor =
        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(config.clone());

    if config.enable_domain_aware && use_cado {
        let plan = build_domain_plan(&txns, config.l_max);
        let num_txns_total = txns.len();
        executor.set_segment_bounds(plan.segment_bounds(), plan.txn_to_segment(num_txns_total));
    }

    let outputs = executor
        .execute_transactions_parallel(args, txns)
        .expect("Funded parallel execution should succeed");

    // Reconstruct parallel state.
    let mut parallel_state = std::collections::HashMap::new();
    for output in &outputs {
        for (k, v) in output.get_writes() {
            parallel_state.insert(k, v);
        }
    }

    // Fold deltas into balances if hot-delta was used.
    if use_hot_delta {
        let delta_entries: Vec<(StateKey, u64)> = parallel_state
            .iter()
            .filter_map(|(k, &v)| {
                if let StateKey::Delta(_, _) = k {
                    Some((k.clone(), v))
                } else {
                    None
                }
            })
            .collect();
        for (k, delta_val) in &delta_entries {
            if let StateKey::Delta(account, _) = k {
                let bal = parallel_state.entry(StateKey::Balance(*account)).or_insert(0);
                *bal += delta_val;
            }
        }
        for (k, _) in &delta_entries {
            parallel_state.remove(k);
        }
    }

    // Compare.
    let mut mismatches = Vec::new();
    for (k, v) in &serial_state {
        match k {
            StateKey::Delta(_, _) => continue,
            _ => {
                let pv = parallel_state.get(k).unwrap_or(&0);
                if v != pv {
                    mismatches.push(format!("{:?}: serial={}, parallel={}, diff={}", k, v, pv, *pv as i64 - *v as i64));
                }
            }
        }
    }
    if !mismatches.is_empty() {
        eprintln!("=== State mismatches ({}) ===", mismatches.len());
        for m in &mismatches {
            eprintln!("  {}", m);
        }
        panic!("Funded mismatch: {} keys differ", mismatches.len());
    }
}

#[test]
fn test_funded_serial_equivalence_baseline() {
    let config = LeapConfig {
        num_workers: 4,
        enable_backpressure: false,
        enable_domain_aware: false,
        enable_hot_delta: false,
        ..LeapConfig::default()
    };
    check_funded_serial_equivalence(config, false, false);
}

#[test]
fn test_funded_serial_equivalence_with_cado() {
    let config = LeapConfig {
        num_workers: 4,
        enable_backpressure: false,
        enable_domain_aware: false,
        enable_hot_delta: false,
        ..LeapConfig::default()
    };
    check_funded_serial_equivalence(config, true, false);
}

#[test]
fn test_funded_serial_equivalence_domain_aware() {
    let config = LeapConfig {
        num_workers: 4,
        enable_backpressure: true,
        enable_domain_aware: true,
        enable_hot_delta: false,
        ..LeapConfig::default()
    };
    check_funded_serial_equivalence(config, true, false);
}

#[test]
fn test_funded_serial_equivalence_hot_delta() {
    let config = LeapConfig {
        num_workers: 4,
        enable_backpressure: false,
        enable_domain_aware: false,
        enable_hot_delta: true,
        ..LeapConfig::default()
    };
    check_funded_serial_equivalence(config, false, true);
}

#[test]
fn test_funded_serial_equivalence_cado_hot_delta() {
    // CADO + hot-delta (no domain-aware) to isolate interaction.
    let config = LeapConfig {
        num_workers: 4,
        enable_backpressure: false,
        enable_domain_aware: false,
        enable_hot_delta: true,
        ..LeapConfig::default()
    };
    check_funded_serial_equivalence(config, true, true);
}

#[test]
fn test_funded_serial_equivalence_all_opts() {
    let config = LeapConfig {
        num_workers: 4,
        enable_backpressure: true,
        enable_domain_aware: true,
        enable_hot_delta: true,
        ..LeapConfig::default()
    };
    check_funded_serial_equivalence(config, true, true);
}

#[test]
fn test_funded_burn_with_hot_delta() {
    // Mint → Transfer → Burn sequence with hot-delta, verify balances correct.
    let txns = vec![
        StablecoinTx {
            tx_type: StablecoinTxType::Mint { to: 5, amount: 1000 },
            nonce: 0,
            tx_hash: 100,
        },
        StablecoinTx {
            tx_type: StablecoinTxType::Mint { to: 1, amount: 500 },
            nonce: 1,
            tx_hash: 101,
        },
        StablecoinTx {
            tx_type: StablecoinTxType::Transfer { sender: 5, receiver: 1, amount: 200 },
            nonce: 2,
            tx_hash: 102,
        },
        StablecoinTx {
            tx_type: StablecoinTxType::Transfer { sender: 5, receiver: 1, amount: 100 },
            nonce: 3,
            tx_hash: 103,
        },
        StablecoinTx {
            tx_type: StablecoinTxType::Burn { from: 1, amount: 400 },
            nonce: 4,
            tx_hash: 104,
        },
    ];

    // Serial execution gives ground truth.
    let serial_state = serial_execute(&txns, 0);
    // Account 5: 1000 - 200 - 100 = 700
    // Account 1: 500 + 200 + 100 - 400 = 400
    // Supply: 1500 - 400 = 1100
    assert_eq!(serial_state[&StateKey::Balance(5)], 700);
    assert_eq!(serial_state[&StateKey::Balance(1)], 400);
    assert_eq!(serial_state[&StateKey::TotalSupply], 1100);

    // Parallel with hot-delta.
    let mut mgr = HotDeltaManager::new(2, 10, 4);
    mgr.detect_hotspots(&txns);
    let parallel_state =
        parallel_execute_to_state_with_hot_delta(txns, 4, 0, Arc::new(mgr));

    for (k, v) in &serial_state {
        match k {
            StateKey::Delta(_, _) => continue,
            _ => {
                let pv = parallel_state.get(k).unwrap_or(&0);
                assert_eq!(v, pv, "Burn+HotDelta mismatch on {:?}: serial={}, parallel={}", k, v, pv);
            }
        }
    }
}

#[test]
fn test_cado_concatenation_enables_domain_plan() {
    // After CADO sequential concatenation, DomainPlan should produce meaningful segments.
    let gen = StablecoinWorkloadGenerator::new(
        100,
        HotspotConfig::Explicit {
            num_hotspots: 3,
            hotspot_ratio: 0.8,
        },
    );
    let mut txns = gen.generate(1000);
    cado_ordering(&mut txns);

    let plan = build_domain_plan(&txns, 64);
    let bounds = plan.segment_bounds();

    let seg_sizes: Vec<usize> = bounds.iter().map(|(s, e, _)| e - s).collect();
    let avg_seg = seg_sizes.iter().sum::<usize>() as f64 / seg_sizes.len() as f64;

    // With sequential concatenation, avg segment size should be >> 1.
    assert!(
        avg_seg > 2.0,
        "After CADO concatenation, avg segment size should be > 2, got {:.1}",
        avg_seg
    );

    // Single-txn segments should be a minority.
    let single_txn = seg_sizes.iter().filter(|&&s| s == 1).count();
    let single_pct = single_txn as f64 / seg_sizes.len() as f64 * 100.0;
    assert!(
        single_pct < 50.0,
        "Single-txn segments should be < 50%, got {:.1}%",
        single_pct
    );
}
