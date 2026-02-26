use e2e::pipeline::*;
use leap::stablecoin::HotspotConfig;
use std::io::Write;

fn parse_arg(args: &[String], flag: &str) -> Option<String> {
    for i in 0..args.len() {
        if args[i] == flag && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        if let Some(val) = args[i].strip_prefix(&format!("{}=", flag)) {
            return Some(val.to_string());
        }
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let block_size: usize = parse_arg(&args, "--block-size")
        .unwrap_or_else(|| "1000".to_string())
        .parse()
        .expect("Invalid --block-size");

    let num_blocks: usize = parse_arg(&args, "--num-blocks")
        .unwrap_or_else(|| "20".to_string())
        .parse()
        .expect("Invalid --num-blocks");

    let pattern_str = parse_arg(&args, "--pattern").unwrap_or_else(|| "Uniform".to_string());

    let accounts: usize = parse_arg(&args, "--accounts")
        .unwrap_or_else(|| "1000".to_string())
        .parse()
        .expect("Invalid --accounts");

    let threads: usize = parse_arg(&args, "--threads")
        .unwrap_or_else(|| "16".to_string())
        .parse()
        .expect("Invalid --threads");

    let overhead_us: u64 = parse_arg(&args, "--overhead-us")
        .unwrap_or_else(|| "10".to_string())
        .parse()
        .expect("Invalid --overhead-us");

    let engine_str = parse_arg(&args, "--engine").unwrap_or_else(|| "Leap".to_string());

    let csv_path = parse_arg(&args, "--csv");

    // Calibrated: 16 SHA-256 iterations ≈ 1μs on this hardware
    let crypto_iters = (overhead_us * 16) as u32;

    let engine = EngineType::from_str(&engine_str).unwrap_or_else(|| {
        eprintln!("Unknown engine: {}. Use Leap, LeapBase, or Serial.", engine_str);
        std::process::exit(1);
    });

    let hotspot = match pattern_str.as_str() {
        "Uniform" => HotspotConfig::Uniform,
        s if s.starts_with("Zipf_") => {
            let alpha: f64 = s.strip_prefix("Zipf_").unwrap().parse().expect("Invalid Zipf alpha");
            HotspotConfig::Zipf { alpha }
        }
        s if s.starts_with("Hotspot_") && s.ends_with("pct") => {
            let pct_str = s.strip_prefix("Hotspot_").unwrap().strip_suffix("pct").unwrap();
            let pct: f64 = pct_str.parse::<f64>().expect("Invalid hotspot pct") / 100.0;
            HotspotConfig::Explicit {
                num_hotspots: 5,
                hotspot_ratio: pct,
            }
        }
        _ => {
            eprintln!("Unknown pattern: {}. Use Uniform, Zipf_X.X, or Hotspot_Npct.", pattern_str);
            std::process::exit(1);
        }
    };

    let warmup = 5;

    eprintln!(
        "E2E Block Execution: engine={} block_size={} blocks={} pattern={} accounts={} threads={} overhead={}us",
        engine.name(), block_size, num_blocks, pattern_str, accounts, threads, overhead_us
    );

    let results = run_block_execution(
        block_size,
        num_blocks,
        warmup,
        engine,
        threads,
        crypto_iters,
        accounts,
        hotspot,
    );

    // Build CSV output
    let mut csv_buf = String::from(
        "block_idx,engine,block_size,pattern,accounts,threads,overhead_us,cado_us,execution_us,total_us,tps\n"
    );

    for r in &results {
        csv_buf.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{:.0}\n",
            r.block_idx, r.engine, r.block_size, r.pattern, r.accounts,
            r.threads, r.overhead_us, r.cado_us, r.execution_us, r.total_us, r.tps
        ));
    }

    // Write CSV
    if let Some(path) = csv_path {
        let mut f = std::fs::File::create(&path).expect("Cannot create CSV file");
        f.write_all(csv_buf.as_bytes()).expect("Cannot write CSV");
        eprintln!("CSV written to {}", path);
    } else {
        print!("{}", csv_buf);
    }

    // Print summary stats
    if !results.is_empty() {
        let mut tps_values: Vec<f64> = results.iter().map(|r| r.tps).collect();
        tps_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median_tps = tps_values[tps_values.len() / 2];
        let avg_cado: f64 = results.iter().map(|r| r.cado_us as f64).sum::<f64>() / results.len() as f64;
        let avg_exec: f64 = results.iter().map(|r| r.execution_us as f64).sum::<f64>() / results.len() as f64;

        eprintln!(
            "  Median TPS: {:.0}, Avg CADO: {:.0}us, Avg Exec: {:.0}us",
            median_tps, avg_cado, avg_exec
        );
    }
}
