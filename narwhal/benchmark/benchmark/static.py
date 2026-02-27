"""
Static instance manager and StaticBench for manually provisioned servers
(no AWS required). Drop-in replacements for InstanceManager and Bench.

Config file format (hosts.json):
{
    "key_path": "/path/to/your/key.pem",
    "username": "ubuntu",
    "base_port": 5000,
    "repo": {
        "name": "claude_stablecoin",
        "url": "https://github.com/YOUR/REPO",
        "branch": "main"
    },
    "hosts": ["192.168.1.1", "192.168.1.2", "192.168.1.3", "192.168.1.4"]
}

Each entry in "hosts" is one server. For collocated mode (primary + worker on
same machine), one IP = one node. You need at least max(bench_params.nodes) IPs.
"""

from json import load, JSONDecodeError
from benchmark.utils import BenchError


class StaticSettingsError(Exception):
    pass


class StaticSettings:
    """Mimics the Settings interface used by Bench."""

    def __init__(self, key_path, username, base_port, repo_name, repo_url, branch):
        self.key_path = key_path
        self.username = username
        self.base_port = base_port
        self.repo_name = repo_name
        self.repo_url = repo_url
        self.branch = branch

    @classmethod
    def load(cls, filename='hosts.json'):
        try:
            with open(filename) as f:
                data = load(f)
            return cls(
                key_path=data['key_path'],
                username=data.get('username', 'ubuntu'),
                base_port=data['base_port'],
                repo_name=data['repo']['name'],
                repo_url=data['repo']['url'],
                branch=data['repo']['branch'],
            )
        except (OSError, JSONDecodeError) as e:
            raise StaticSettingsError(str(e))
        except KeyError as e:
            raise StaticSettingsError(f'Malformed hosts.json: missing key {e}')


class StaticInstanceManager:
    """
    Manages a fixed list of servers. Implements the same interface as
    InstanceManager so it can be used as a drop-in inside Bench.

    hosts() returns {'static': [ip1, ip2, ...]} — a single fake "region"
    containing all IPs. This satisfies Bench._select_hosts() for collocate=True.
    """

    def __init__(self, settings_file='hosts.json'):
        try:
            with open(settings_file) as f:
                data = load(f)
            self._hosts = data['hosts']
            if not self._hosts:
                raise StaticSettingsError('hosts list is empty')
        except (OSError, JSONDecodeError) as e:
            raise BenchError('Failed to load hosts.json', StaticSettingsError(str(e)))
        except KeyError as e:
            raise BenchError('Malformed hosts.json', StaticSettingsError(f'missing key {e}'))

        try:
            self.settings = StaticSettings.load(settings_file)
        except StaticSettingsError as e:
            raise BenchError('Failed to load static settings', e)

    def hosts(self, flat=False):
        """
        Return host list in the format Bench expects.
        flat=True  → ['ip1', 'ip2', ...]       (used by install/kill)
        flat=False → {'static': ['ip1', ...]}  (used by _select_hosts)
        """
        if flat:
            return list(self._hosts)
        return {'static': list(self._hosts)}

    # Stubs for lifecycle methods that don't apply to static servers.
    def create_instances(self, *args, **kwargs):
        raise NotImplementedError('Static servers: create instances manually')

    def terminate_instances(self, *args, **kwargs):
        raise NotImplementedError('Static servers: terminate instances manually')

    def start_instances(self, *args, **kwargs):
        raise NotImplementedError('Static servers: start instances manually')

    def stop_instances(self, *args, **kwargs):
        raise NotImplementedError('Static servers: stop instances manually')

    def print_info(self):
        key = self.settings.key_path
        user = self.settings.username
        print('\n' + '-' * 60)
        print(f' Static testbed ({len(self._hosts)} servers)')
        print('-' * 60)
        for i, ip in enumerate(self._hosts):
            print(f'  {i}\tssh -i {key} {user}@{ip}')
        print('-' * 60 + '\n')


class StaticBench:
    """
    A standalone wrapper around Bench that works without a Fabric @task context.
    Use this in regular Python scripts (run_distributed_*.py) instead of LocalBench.

    Usage:
        bench = StaticBench(extra_features='mp3bft', env_vars={'MP3BFT_K_SLOTS': '4'})
        result = bench.run(bench_params, node_params)
        print(result.result())
    """

    def __init__(self, extra_features=None, env_vars=None, hosts_file='hosts.json'):
        from benchmark.remote import Bench
        from paramiko import RSAKey
        from paramiko.ssh_exception import PasswordRequiredException, SSHException
        from benchmark.utils import BenchError

        manager = StaticInstanceManager(hosts_file)

        # Build a plain-dict connect_kwargs that Fabric Connection/Group accepts.
        # Fabric's connect_kwargs can be a regular dict with paramiko kwargs.
        try:
            pkey = RSAKey.from_private_key_file(manager.settings.key_path)
        except (IOError, PasswordRequiredException, SSHException) as e:
            raise BenchError('Failed to load SSH key', e)

        # Monkey-patch a minimal ctx-like object: Bench only reads
        # ctx.connect_kwargs.pkey and then stores ctx.connect_kwargs as self.connect.
        # A plain namespace with pkey attribute works for both uses.
        class _ConnKwargs(dict):
            """Dict subclass that also supports attribute-style access."""
            def __getattr__(self, name):
                try:
                    return self[name]
                except KeyError:
                    raise AttributeError(name)
            def __setattr__(self, name, value):
                self[name] = value

        class _FakeCtx:
            connect_kwargs = _ConnKwargs()

        ctx = _FakeCtx()
        ctx.connect_kwargs.pkey = pkey

        self._bench = Bench(ctx, extra_features=extra_features,
                            env_vars=env_vars, manager=manager)

    def run(self, bench_params_dict, node_params_dict, debug=False):
        return self._bench.run(bench_params_dict, node_params_dict, debug=debug)

    def install(self):
        return self._bench.install()

    @property
    def manager(self):
        return self._bench.manager
