use crate::pipeline::*;
use leap::stablecoin::HotspotConfig;

#[test]
fn test_cp7_block_execution_correctness() {
    // CP-7: LEAP executes blocks correctly without panicking.
    // Run block execution and verify it produces timing results.
    let results = run_block_execution(
        200,
        3,
        1,
        EngineType::Leap,
        4,
        0,
        50,
        HotspotConfig::Uniform,
    );

    assert_eq!(results.len(), 3, "Must produce 3 measured blocks");
    for r in &results {
        assert_eq!(r.block_size, 200);
        assert!(r.total_us > 0, "Execution must take nonzero time");
    }

    println!(
        "CP-7 block execution: {} blocks, median TPS = {:.0}",
        results.len(),
        median_tps(&results),
    );
}

#[test]
fn test_cp7_serial_baseline() {
    // Serial execution works correctly, no CADO applied.
    let results = run_block_execution(
        200,
        3,
        1,
        EngineType::Serial,
        1,
        0,
        50,
        HotspotConfig::Uniform,
    );

    assert_eq!(results.len(), 3, "Must produce 3 measured blocks");
    for r in &results {
        assert_eq!(r.engine, "Serial");
        assert_eq!(r.cado_us, 0, "Serial must not run CADO");
    }

    println!(
        "Serial baseline: {} blocks, median TPS = {:.0}",
        results.len(),
        median_tps(&results),
    );
}

#[test]
fn test_leap_beats_serial_on_block() {
    // With crypto overhead and multiple threads, LEAP should beat serial.
    let block_size = 1000;
    let crypto_iters = 10 * 16; // 10μs overhead

    let leap_results = run_block_execution(
        block_size,
        5,
        2,
        EngineType::Leap,
        8,
        crypto_iters,
        200,
        HotspotConfig::Uniform,
    );

    let serial_results = run_block_execution(
        block_size,
        5,
        2,
        EngineType::Serial,
        1,
        crypto_iters,
        200,
        HotspotConfig::Uniform,
    );

    let leap_median = median_tps(&leap_results);
    let serial_median = median_tps(&serial_results);

    println!(
        "LEAP 8t: {:.0} TPS, Serial: {:.0} TPS, speedup: {:.1}x",
        leap_median,
        serial_median,
        leap_median / serial_median,
    );

    assert!(
        leap_median > serial_median,
        "LEAP 8t ({:.0}) must beat serial ({:.0})",
        leap_median,
        serial_median,
    );
}

#[test]
fn test_high_conflict_block() {
    // Zipf 1.2 doesn't crash and produces valid results.
    let results = run_block_execution(
        500,
        3,
        1,
        EngineType::Leap,
        4,
        0,
        50,
        HotspotConfig::Zipf { alpha: 1.2 },
    );

    assert!(!results.is_empty(), "Must produce results");
    for r in &results {
        assert!(r.tps > 0.0, "TPS must be positive");
    }

    println!(
        "High-conflict (Zipf 1.2): median TPS = {:.0}",
        median_tps(&results),
    );
}

fn median_tps(results: &[BlockExecResult]) -> f64 {
    let mut vals: Vec<f64> = results.iter().map(|r| r.tps).collect();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    vals[vals.len() / 2]
}
