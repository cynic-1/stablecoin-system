# Copyright(C) Facebook, Inc. and its affiliates.
from fabric import task

from benchmark.local import LocalBench
from benchmark.logs import ParseError, LogParser
from benchmark.utils import Print
from benchmark.plot import Ploter, PlotError
from benchmark.instance import InstanceManager
from benchmark.remote import Bench, BenchError
from benchmark.static import StaticInstanceManager


@task
def local(ctx, debug=True):
    ''' Run benchmarks on localhost '''
    bench_params = {
        'faults': 0,
        'nodes': 4,
        'workers': 1,
        'rate': 50_000,
        'tx_size': 512,
        'duration': 20,
    }
    node_params = {
        'header_size': 1_000,  # bytes
        'max_header_delay': 200,  # ms
        'gc_depth': 50,  # rounds
        'sync_retry_delay': 10_000,  # ms
        'sync_retry_nodes': 3,  # number of nodes
        'batch_size': 500_000,  # bytes
        'max_batch_delay': 200  # ms
    }
    try:
        ret = LocalBench(bench_params, node_params).run(debug)
        print(ret.result())
    except BenchError as e:
        Print.error(e)


@task
def mp3bft(ctx, debug=True, k=4):
    ''' Run MP3-BFT++ benchmarks on localhost '''
    bench_params = {
        'faults': 0,
        'nodes': 4,
        'workers': 1,
        'rate': 50_000,
        'tx_size': 512,
        'duration': 20,
    }
    node_params = {
        'header_size': 1_000,  # bytes
        'max_header_delay': 200,  # ms
        'gc_depth': 50,  # rounds
        'sync_retry_delay': 10_000,  # ms
        'sync_retry_nodes': 3,  # number of nodes
        'batch_size': 500_000,  # bytes
        'max_batch_delay': 200  # ms
    }
    try:
        env_vars = {'MP3BFT_K_SLOTS': str(k)}
        ret = LocalBench(
            bench_params, node_params,
            extra_features='mp3bft', env_vars=env_vars
        ).run(debug)
        print(ret.result())
    except BenchError as e:
        Print.error(e)


@task
def create(ctx, nodes=2):
    ''' Create a testbed'''
    try:
        InstanceManager.make().create_instances(nodes)
    except BenchError as e:
        Print.error(e)


@task
def destroy(ctx):
    ''' Destroy the testbed '''
    try:
        InstanceManager.make().terminate_instances()
    except BenchError as e:
        Print.error(e)


@task
def start(ctx, max=2):
    ''' Start at most `max` machines per data center '''
    try:
        InstanceManager.make().start_instances(max)
    except BenchError as e:
        Print.error(e)


@task
def stop(ctx):
    ''' Stop all machines '''
    try:
        InstanceManager.make().stop_instances()
    except BenchError as e:
        Print.error(e)


@task
def info(ctx):
    ''' Display connect information about all the available machines '''
    try:
        InstanceManager.make().print_info()
    except BenchError as e:
        Print.error(e)


@task
def install(ctx):
    ''' Install the codebase on all machines '''
    try:
        Bench(ctx).install()
    except BenchError as e:
        Print.error(e)


@task
def remote(ctx, debug=False):
    ''' Run benchmarks on AWS '''
    bench_params = {
        'faults': 3,
        'nodes': [10],
        'workers': 1,
        'collocate': True,
        'rate': [10_000, 110_000],
        'tx_size': 512,
        'duration': 300,
        'runs': 2,
    }
    node_params = {
        'header_size': 1_000,  # bytes
        'max_header_delay': 200,  # ms
        'gc_depth': 50,  # rounds
        'sync_retry_delay': 10_000,  # ms
        'sync_retry_nodes': 3,  # number of nodes
        'batch_size': 500_000,  # bytes
        'max_batch_delay': 200  # ms
    }
    try:
        Bench(ctx).run(bench_params, node_params, debug)
    except BenchError as e:
        Print.error(e)


@task
def remote_mp3bft(ctx, debug=False, k=4):
    ''' Run MP3-BFT++ benchmarks on AWS '''
    bench_params = {
        'faults': 0,
        'nodes': [4],
        'workers': 1,
        'collocate': True,
        'rate': [50_000, 100_000, 200_000],
        'tx_size': 512,
        'duration': 60,
        'runs': 2,
    }
    node_params = {
        'header_size': 1_000,
        'max_header_delay': 200,
        'gc_depth': 50,
        'sync_retry_delay': 10_000,
        'sync_retry_nodes': 3,
        'batch_size': 500_000,
        'max_batch_delay': 200,
    }
    try:
        env_vars = {'MP3BFT_K_SLOTS': str(k)}
        Bench(ctx, extra_features='mp3bft', env_vars=env_vars).run(
            bench_params, node_params, debug
        )
    except BenchError as e:
        Print.error(e)


@task
def remote_e2e(ctx, engine='leap', k=4, debug=False):
    ''' Run E2E (consensus + LEAP execution) benchmarks on AWS '''
    bench_params = {
        'faults': 0,
        'nodes': [4],
        'workers': 1,
        'collocate': True,
        'rate': [50_000, 100_000, 150_000, 200_000],
        'tx_size': 512,
        'duration': 60,
        'runs': 2,
    }
    node_params = {
        'header_size': 1_000,
        'max_header_delay': 200,
        'gc_depth': 50,
        'sync_retry_delay': 10_000,
        'sync_retry_nodes': 3,
        'batch_size': 500_000,
        'max_batch_delay': 200,
    }
    try:
        features = 'e2e_exec,mp3bft'
        # On remote machines each node runs on a dedicated server, so use all cores.
        # Adjust LEAP_THREADS to match the instance's vCPU count.
        env_vars = {
            'MP3BFT_K_SLOTS': str(k),
            'LEAP_ENGINE': str(engine),
            'LEAP_THREADS': '16',   # override to match remote instance type
            'RAYON_NUM_THREADS': '16',
            'LEAP_CRYPTO_US': '10',
            'LEAP_ACCOUNTS': '1000',
            'BENCH_TX_SIZE': str(bench_params['tx_size']),
        }
        Bench(ctx, extra_features=features, env_vars=env_vars).run(
            bench_params, node_params, debug
        )
    except BenchError as e:
        Print.error(e)


@task
def plot(ctx):
    ''' Plot performance using the logs generated by "fab remote" '''
    plot_params = {
        'faults': [0],
        'nodes': [10, 20, 50],
        'workers': [1],
        'collocate': True,
        'tx_size': 512,
        'max_latency': [3_500, 4_500]
    }
    try:
        Ploter.plot(plot_params)
    except PlotError as e:
        Print.error(BenchError('Failed to plot performance', e))


@task
def e2e(ctx, engine='leap', k=4, debug=True):
    ''' Run E2E benchmarks (consensus + LEAP execution) on localhost '''
    bench_params = {
        'faults': 0,
        'nodes': 4,
        'workers': 1,
        'rate': 50_000,
        'tx_size': 512,
        'duration': 30,
    }
    node_params = {
        'header_size': 1_000,
        'max_header_delay': 200,
        'gc_depth': 50,
        'sync_retry_delay': 10_000,
        'sync_retry_nodes': 3,
        'batch_size': 500_000,
        'max_batch_delay': 200,
    }
    try:
        features = 'e2e_exec'
        import os
        nodes = bench_params['nodes']
        total_cores = os.cpu_count() or 8
        leap_threads = max(1, total_cores // nodes)
        env_vars = {
            'LEAP_ENGINE': str(engine),
            'LEAP_THREADS': str(leap_threads),
            'RAYON_NUM_THREADS': str(leap_threads),
            'LEAP_CRYPTO_US': '10',
            'LEAP_ACCOUNTS': '1000',
            'BENCH_TX_SIZE': str(bench_params['tx_size']),
        }
        if engine == 'leap' or engine == 'leap_base':
            features += ',mp3bft'
            env_vars['MP3BFT_K_SLOTS'] = str(k)
        ret = LocalBench(
            bench_params, node_params,
            extra_features=features, env_vars=env_vars
        ).run(debug)
        print(ret.result())
    except BenchError as e:
        Print.error(e)


@task
def static_install(ctx):
    ''' Install dependencies on servers listed in hosts.json '''
    try:
        manager = StaticInstanceManager('hosts.json')
        Bench(ctx, manager=manager).install()
    except BenchError as e:
        Print.error(e)


@task
def static_info(ctx):
    ''' Show SSH connection info for servers in hosts.json '''
    try:
        StaticInstanceManager('hosts.json').print_info()
    except BenchError as e:
        Print.error(e)


@task
def static_tusk(ctx, debug=False):
    ''' Run Narwhal-Tusk benchmark on servers in hosts.json '''
    bench_params = {
        'faults': 0,
        'nodes': [4],
        'workers': 1,
        'collocate': True,
        'rate': [50_000, 100_000, 200_000],
        'tx_size': 512,
        'duration': 60,
        'runs': 2,
    }
    node_params = {
        'header_size': 1_000,
        'max_header_delay': 200,
        'gc_depth': 50,
        'sync_retry_delay': 10_000,
        'sync_retry_nodes': 3,
        'batch_size': 500_000,
        'max_batch_delay': 200,
    }
    try:
        manager = StaticInstanceManager('hosts.json')
        Bench(ctx, manager=manager).run(bench_params, node_params, debug)
    except BenchError as e:
        Print.error(e)


@task
def static_mp3bft(ctx, debug=False, k=4):
    ''' Run MP3-BFT++ benchmark on servers in hosts.json '''
    bench_params = {
        'faults': 0,
        'nodes': [4],
        'workers': 1,
        'collocate': True,
        'rate': [50_000, 100_000, 200_000],
        'tx_size': 512,
        'duration': 60,
        'runs': 2,
    }
    node_params = {
        'header_size': 1_000,
        'max_header_delay': 200,
        'gc_depth': 50,
        'sync_retry_delay': 10_000,
        'sync_retry_nodes': 3,
        'batch_size': 500_000,
        'max_batch_delay': 200,
    }
    try:
        manager = StaticInstanceManager('hosts.json')
        env_vars = {'MP3BFT_K_SLOTS': str(k)}
        Bench(ctx, extra_features='mp3bft', env_vars=env_vars, manager=manager).run(
            bench_params, node_params, debug
        )
    except BenchError as e:
        Print.error(e)


@task
def static_e2e(ctx, engine='leap', k=4, debug=False, threads=16):
    ''' Run E2E (consensus + LEAP) benchmark on servers in hosts.json '''
    bench_params = {
        'faults': 0,
        'nodes': [4],
        'workers': 1,
        'collocate': True,
        'rate': [50_000, 100_000, 150_000, 200_000],
        'tx_size': 512,
        'duration': 60,
        'runs': 2,
    }
    node_params = {
        'header_size': 1_000,
        'max_header_delay': 200,
        'gc_depth': 50,
        'sync_retry_delay': 10_000,
        'sync_retry_nodes': 3,
        'batch_size': 500_000,
        'max_batch_delay': 200,
    }
    try:
        manager = StaticInstanceManager('hosts.json')
        features = 'e2e_exec,mp3bft'
        env_vars = {
            'MP3BFT_K_SLOTS': str(k),
            'LEAP_ENGINE': str(engine),
            'LEAP_THREADS': str(threads),
            'RAYON_NUM_THREADS': str(threads),
            'LEAP_CRYPTO_US': '10',
            'LEAP_ACCOUNTS': '1000',
            'BENCH_TX_SIZE': str(bench_params['tx_size']),
        }
        Bench(ctx, extra_features=features, env_vars=env_vars, manager=manager).run(
            bench_params, node_params, debug
        )
    except BenchError as e:
        Print.error(e)


@task
def kill(ctx):
    ''' Stop execution on all machines '''
    try:
        Bench(ctx).kill()
    except BenchError as e:
        Print.error(e)


@task
def logs(ctx):
    ''' Print a summary of the logs '''
    try:
        print(LogParser.process('./logs', faults='?').result())
    except ParseError as e:
        Print.error(BenchError('Failed to parse logs', e))
