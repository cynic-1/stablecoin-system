use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use leap::{
    cado::cado_ordering,
    config::LeapConfig,
    executor::ParallelTransactionExecutor,
    stablecoin::*,
};

fn bench_parallel_execution(c: &mut Criterion) {
    let num_txns = 10_000;
    let num_accounts = 1000;

    let mut group = c.benchmark_group("parallel_execution");
    group.sample_size(10);

    for threads in [1, 2, 4, 8] {
        let gen = StablecoinWorkloadGenerator::new(num_accounts, HotspotConfig::Uniform);

        group.bench_with_input(
            BenchmarkId::new("uniform", threads),
            &threads,
            |b, &threads| {
                b.iter(|| {
                    let txns = gen.generate(num_txns);
                    let config = LeapConfig {
                        num_workers: threads,
                        ..LeapConfig::baseline()
                    };
                    let executor =
                        ParallelTransactionExecutor::<StablecoinTx, StablecoinExecutor>::with_config(
                            config,
                        );
                    executor.execute_transactions_parallel(CRYPTO_WORK_ITERS, txns).unwrap();
                });
            },
        );
    }

    group.finish();
}

fn bench_cado_ordering(c: &mut Criterion) {
    let gen = StablecoinWorkloadGenerator::new(1000, HotspotConfig::Zipf { alpha: 0.8 });
    let txns = gen.generate(10_000);

    c.bench_function("cado_ordering_10k", |b| {
        b.iter(|| {
            let mut t = txns.clone();
            cado_ordering(&mut t);
        });
    });
}

criterion_group!(benches, bench_parallel_execution, bench_cado_ordering);
criterion_main!(benches);
