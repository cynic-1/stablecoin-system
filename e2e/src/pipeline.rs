use leap::{
    cado::cado_ordering,
    config::LeapConfig,
    executor::ParallelTransactionExecutor,
    stablecoin::{
        serial_execute, HotspotConfig, StablecoinExecutor, StablecoinTx,
        StablecoinWorkloadGenerator, StateKey, StateValue,
    },
    task::TransactionOutput,
};
use std::collections::HashMap;
use std::time::Instant;

/// Result of executing a single block.
#[derive(Debug, Clone)]
pub struct BlockExecResult {
    pub block_idx: usize,
    pub engine: String,
    pub block_size: usize,
    pub pattern: String,
    pub accounts: usize,
    pub threads: usize,
    pub overhead_us: u64,
    pub cado_us: u64,
    pub execution_us: u64,
    pub total_us: u64,
    pub tps: f64,
}

/// Engine type for block execution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EngineType {
    /// Full LEAP: CADO ordering + all optimizations enabled
    Leap,
    /// LEAP-base: no CADO, no optimizations (Block-STM equivalent)
    LeapBase,
    /// Serial: single-threaded sequential execution
    Serial,
}

impl EngineType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "leap" => Some(Self::Leap),
            "leap-base" | "leapbase" | "leap_base" => Some(Self::LeapBase),
            "serial" => Some(Self::Serial),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Leap => "Leap",
            Self::LeapBase => "LeapBase",
            Self::Serial => "Serial",
        }
    }
}

/// Execute a single block of transactions and return timing results.
pub fn execute_block(
    txns: Vec<StablecoinTx>,
    engine: EngineType,
    threads: usize,
    crypto_iters: u32,
) -> (u64, u64) {
    match engine {
        EngineType::Leap => {
            // CADO ordering
            let mut ordered = txns;
            let cado_start = Instant::now();
            cado_ordering(&mut ordered);
            let cado_us = cado_start.elapsed().as_micros() as u64;

            // LEAP parallel execution with all optimizations
            let config = LeapConfig {
                num_workers: threads,
                ..LeapConfig::full()
            };
            let executor =
                ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(
                    config,
                );
            let exec_start = Instant::now();
            let _outputs = executor
                .execute_transactions_parallel(crypto_iters.into(), ordered)
                .expect("LEAP execution must succeed");
            let exec_us = exec_start.elapsed().as_micros() as u64;

            (cado_us, exec_us)
        }
        EngineType::LeapBase => {
            // No CADO, LEAP with all optimizations disabled
            let config = LeapConfig {
                num_workers: threads,
                ..LeapConfig::baseline()
            };
            let executor =
                ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(
                    config,
                );
            let exec_start = Instant::now();
            let _outputs = executor
                .execute_transactions_parallel(crypto_iters.into(), txns)
                .expect("LeapBase execution must succeed");
            let exec_us = exec_start.elapsed().as_micros() as u64;

            (0, exec_us)
        }
        EngineType::Serial => {
            let exec_start = Instant::now();
            let _state = serial_execute(&txns, crypto_iters);
            let exec_us = exec_start.elapsed().as_micros() as u64;

            (0, exec_us)
        }
    }
}

/// Run block execution benchmark: warmup blocks + measured blocks.
pub fn run_block_execution(
    block_size: usize,
    num_blocks: usize,
    warmup: usize,
    engine: EngineType,
    threads: usize,
    crypto_iters: u32,
    accounts: usize,
    hotspot: HotspotConfig,
) -> Vec<BlockExecResult> {
    let pattern_name = match &hotspot {
        HotspotConfig::Uniform => "Uniform".to_string(),
        HotspotConfig::Zipf { alpha } => format!("Zipf_{}", alpha),
        HotspotConfig::Explicit { hotspot_ratio, .. } => {
            format!("Hotspot_{}pct", (hotspot_ratio * 100.0) as u32)
        }
    };

    let gen = StablecoinWorkloadGenerator::new(accounts, hotspot);
    let total_blocks = warmup + num_blocks;

    let mut results = Vec::with_capacity(num_blocks);

    for i in 0..total_blocks {
        let txns = gen.generate(block_size);
        let (cado_us, exec_us) = execute_block(txns, engine, threads, crypto_iters);
        let total_us = cado_us + exec_us;
        let tps = if total_us > 0 {
            block_size as f64 / (total_us as f64 / 1_000_000.0)
        } else {
            0.0
        };

        // Only record after warmup
        if i >= warmup {
            results.push(BlockExecResult {
                block_idx: i - warmup,
                engine: engine.name().to_string(),
                block_size,
                pattern: pattern_name.clone(),
                accounts,
                threads,
                overhead_us: (crypto_iters as u64) / 16, // reverse calibration
                cado_us,
                execution_us: exec_us,
                total_us,
                tps,
            });
        }
    }

    results
}

/// Execute a block with full LEAP and return the final state (for correctness testing).
pub fn execute_block_with_state(
    txns: Vec<StablecoinTx>,
    engine: EngineType,
    threads: usize,
    crypto_iters: u32,
) -> HashMap<StateKey, StateValue> {
    match engine {
        EngineType::Leap => {
            let mut ordered = txns;
            cado_ordering(&mut ordered);

            let config = LeapConfig {
                num_workers: threads,
                ..LeapConfig::full()
            };
            let executor =
                ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(
                    config,
                );
            let outputs = executor
                .execute_transactions_parallel(crypto_iters.into(), ordered)
                .expect("LEAP execution must succeed");

            let mut state = HashMap::new();
            for output in &outputs {
                for (k, v) in output.get_writes() {
                    state.insert(k, v);
                }
            }
            state
        }
        EngineType::LeapBase => {
            let config = LeapConfig {
                num_workers: threads,
                ..LeapConfig::baseline()
            };
            let executor =
                ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(
                    config,
                );
            let outputs = executor
                .execute_transactions_parallel(crypto_iters.into(), txns)
                .expect("LeapBase execution must succeed");

            let mut state = HashMap::new();
            for output in &outputs {
                for (k, v) in output.get_writes() {
                    state.insert(k, v);
                }
            }
            state
        }
        EngineType::Serial => {
            serial_execute(&txns, crypto_iters)
        }
    }
}
