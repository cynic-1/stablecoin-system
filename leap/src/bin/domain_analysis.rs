/// Analyze conflict domain sizes under union-find on (sender, receiver).
/// Shows why "cross-domain parallel + intra-domain serial" degenerates
/// to serial execution for most workloads.
use leap::stablecoin::*;

fn main() {
    println!("=== Conflict Domain Analysis (Union-Find on sender+receiver) ===\n");
    println!("{:<10} {:<8} {:<12} {:<12} {:<15} {:<10}",
        "accounts", "txns", "domains", "largest", "serial_frac", "max_par");

    for &accounts in &[10, 100, 1000, 10000] {
        for &num_txns in &[1000, 10000] {
            let gen = StablecoinWorkloadGenerator::new(accounts, HotspotConfig::Uniform);
            let txns = gen.generate_seeded(num_txns, 42);

            // Union-Find on accounts
            let mut parent: Vec<usize> = (0..accounts).collect();

            fn find(parent: &mut Vec<usize>, x: usize) -> usize {
                if parent[x] != x {
                    parent[x] = find(parent, parent[x]);
                }
                parent[x]
            }

            fn union(parent: &mut Vec<usize>, a: usize, b: usize) {
                let ra = find(parent, a);
                let rb = find(parent, b);
                if ra != rb {
                    parent[ra] = rb;
                }
            }

            for tx in &txns {
                if let StablecoinTxType::Transfer { sender, receiver, .. } = &tx.tx_type {
                    let s = *sender as usize;
                    let r = *receiver as usize;
                    if s < accounts && r < accounts {
                        union(&mut parent, s, r);
                    }
                }
            }

            // Count components and sizes
            let mut comp_size: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            for i in 0..accounts {
                let root = find(&mut parent, i);
                *comp_size.entry(root).or_default() += 1;
            }

            // Assign txns to components
            let mut txn_per_comp: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            for tx in &txns {
                if let StablecoinTxType::Transfer { sender, .. } = &tx.tx_type {
                    let root = find(&mut parent, *sender as usize);
                    *txn_per_comp.entry(root).or_default() += 1;
                }
            }

            let num_domains = comp_size.len();
            let largest_domain_txns = txn_per_comp.values().max().copied().unwrap_or(0);
            let serial_frac = largest_domain_txns as f64 / num_txns as f64;
            // Max parallelism: number of domains that have transactions
            let domains_with_txns = txn_per_comp.len();

            println!("{:<10} {:<8} {:<12} {:<12} {:<15.1}% {:<10}",
                accounts, num_txns, num_domains, largest_domain_txns,
                serial_frac * 100.0, domains_with_txns);
        }
    }

    // Also test with Hotspot pattern
    println!("\n=== Hotspot_90% ===\n");
    println!("{:<10} {:<8} {:<12} {:<12} {:<15} {:<10}",
        "accounts", "txns", "domains", "largest", "serial_frac", "max_par");

    for &accounts in &[100, 1000, 10000] {
        let num_txns = 10000;
        let gen = StablecoinWorkloadGenerator::new(
            accounts,
            HotspotConfig::Explicit { num_hotspots: 1, hotspot_ratio: 0.9 },
        );
        let txns = gen.generate_seeded(num_txns, 42);

        let mut parent: Vec<usize> = (0..accounts).collect();

        fn find2(parent: &mut Vec<usize>, x: usize) -> usize {
            if parent[x] != x {
                parent[x] = find2(parent, parent[x]);
            }
            parent[x]
        }

        fn union2(parent: &mut Vec<usize>, a: usize, b: usize) {
            let ra = find2(parent, a);
            let rb = find2(parent, b);
            if ra != rb {
                parent[ra] = rb;
            }
        }

        for tx in &txns {
            if let StablecoinTxType::Transfer { sender, receiver, .. } = &tx.tx_type {
                let s = *sender as usize;
                let r = *receiver as usize;
                if s < accounts && r < accounts {
                    union2(&mut parent, s, r);
                }
            }
        }

        let mut txn_per_comp: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for tx in &txns {
            if let StablecoinTxType::Transfer { sender, .. } = &tx.tx_type {
                let root = find2(&mut parent, *sender as usize);
                *txn_per_comp.entry(root).or_default() += 1;
            }
        }

        let mut comp_size: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for i in 0..accounts {
            let root = find2(&mut parent, i);
            *comp_size.entry(root).or_default() += 1;
        }

        let num_domains = comp_size.len();
        let largest = txn_per_comp.values().max().copied().unwrap_or(0);
        let serial_frac = largest as f64 / num_txns as f64;
        let domains_with_txns = txn_per_comp.len();

        println!("{:<10} {:<8} {:<12} {:<12} {:<15.1}% {:<10}",
            accounts, num_txns, num_domains, largest,
            serial_frac * 100.0, domains_with_txns);
    }
}
