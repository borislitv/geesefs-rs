#!/usr/bin/env python3
"""Local disposable L2 qualification, not production or Go-relative acceptance.

Default is a fail-fast preflight; --execute requires frozen build receipts and an
already delegated cgroup. No SSH, global cache drops, ambient AWS keys or builds.
Uses integration's owned localhost S3 fixture/mount lifecycle and compare's
workloads, CPU/RSS/cgroup sampling and paired statistics, not a second framework.
"""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import pwd
import re
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error

import compare
from accounting import cpu_efficiency
from paired import assess

integration = compare.integration
ROOT = integration.ROOT
MIB = 1024 ** 2
POPULATION_ROUNDS = 8
POPULATION_SECONDS = 180
POPULATION_READ_AHEAD_KIB = 128
EVICTION_PIPELINE_BYTES = 5 * MIB
STATES = ['both_cold', 'l1_hot', 'l2_hot_pagecache_hot', 'l2_hot_pagecache_cold_requested']
WORKLOADS = ['metadata', 'sequential', 'random', 'concurrent', 'eviction', 'mixed', 'writer_fsync']
READS = ['sequential', 'random', 'concurrent', 'mixed']
METRICS = ['throughput', 'p50_ms', 'p95_ms', 'p99_ms', 'rss_peak_bytes', 'rss_steady_bytes',
           'daemon_cpu_seconds', 'client_cpu_seconds', 'cgroup_cpu_seconds',
           'cgroup_peak_bytes', 'cgroup_steady_bytes', 'file_cache_steady_bytes', 'kernel_steady_bytes',
           'allocator_peak_bytes', 'allocator_steady_bytes']


def example_manifest():
    return dict(schema=1, memory_limit_mb=64, metadata_limit='16 MiB', entry_limit=100000,
                dataset_bytes=128 * MIB, l1_hot_bytes=MIB, eviction_bytes=512 * MIB,
                disk_capacity_bytes=256 * MIB, metadata_entries=2048, random_operations=256,
                random_block_bytes=128 * 1024, concurrency=4, writer_bytes=8 * MIB,
                cpu_quota_us=200000, cpu_period_us=100000, cgroup_memory_max_mb=2048,
                minimum_active_seconds=1,
                states=STATES[:], workloads=WORKLOADS[:],
                disk_metrics=dict(l1_hit_bytes='geesefs_read_cache_hit_bytes_total',
                                   l1_resident_bytes='geesefs_read_cache_bytes',
                                   l1_loaned_bytes='geesefs_read_cache_loaned_bytes',
                                  remote_bytes='geesefs_read_remote_bytes_total',
                                  l2_hit_bytes='geesefs_disk_cache_hit_bytes_total',
                                  l2_hits='geesefs_disk_cache_hits_total',
                                  l2_misses='geesefs_disk_cache_misses_total',
                                  dropped='geesefs_disk_cache_dropped_total',
                                   errors='geesefs_disk_cache_errors_total',
                                   admitted_demotions='geesefs_disk_cache_admitted_demotions_total',
                                   engine_admitted_demotions='geesefs_disk_cache_engine_admitted_demotions_total',
                                   spill_no_space='geesefs_disk_cache_spill_no_space_total',
                                   spill_errors='geesefs_disk_cache_spill_errors_total',
                                   region_reclaims_started='geesefs_disk_cache_region_reclaims_started_total',
                                   read_operations='geesefs_disk_cache_read_operations_total',
                                   read_operation_hits='geesefs_disk_cache_read_operation_hits_total',
                                   read_operation_misses='geesefs_disk_cache_read_operation_misses_total',
                                   read_operation_errors='geesefs_disk_cache_read_operation_errors_total',
                                   read_duration_seconds='geesefs_disk_cache_read_operation_duration_seconds_total',
                                   read_duration_max_seconds='geesefs_disk_cache_read_operation_duration_seconds_max',
                                   spill_operations='geesefs_disk_cache_spill_operations_total',
                                   spill_duration_seconds='geesefs_disk_cache_spill_operation_duration_seconds_total',
                                   spill_duration_max_seconds='geesefs_disk_cache_spill_operation_duration_seconds_max',
                                   opened_file_available='geesefs_disk_cache_opened_file_observation_available',
                                   opened_file_allocated_bytes='geesefs_disk_cache_opened_file_allocated_bytes',
                                   **{f'{phase}_{meaning}': f'geesefs_disk_cache_open_phase_{metric}{{phase="{phase}"}}'
                                      for phase in ['startup', 'allocation', 'recovery']
                                      for meaning, metric in [('completions', 'completions_total'),
                                          ('seconds', 'duration_seconds_total'), ('max_seconds', 'duration_seconds_max')]},
                                  index_entries='geesefs_disk_cache_index_entries',
                                  recent_entries='geesefs_disk_cache_recent_entries',
                                  submission_bytes='geesefs_disk_cache_submission_bytes{scope="queued_and_active"}',
                                  submission_items='geesefs_disk_cache_submission_items{scope="queued_and_active"}',
                                  active_jobs='geesefs_disk_cache_active_jobs',
                                  copy_owned_bytes='geesefs_disk_cache_copy_owned_bytes',
                                  io_read_bytes='geesefs_disk_cache_io_read_bytes_total',
                                  io_write_bytes='geesefs_disk_cache_io_write_bytes_total',
                                  managed_capacity_bytes='geesefs_disk_cache_managed_capacity_bytes',
                                  funding_bytes='geesefs_disk_cache_funding_bytes',
                                  effective_l1_policy_bytes='geesefs_effective_l1_policy_bytes'),
                 unavailable_disk_observations=['capacity_evictions', 'allocated_bytes',
                     'index_heap_bytes', 'latency_quantiles'],
                superiority_targets={'sequential/l2_hot_pagecache_hot': ['throughput'],
                                     'random/l2_hot_pagecache_hot': ['throughput']},
                non_inferiority_margins={},
                provenance={arm: dict(source_revision='REPLACE_WITH_FROZEN_REVISION',
                                     binary_sha256='REPLACE_WITH_EXECUTABLE_SHA256',
                                     build_command='REPLACE_WITH_EXACT_BUILD_COMMAND',
                                     toolchain='REPLACE_WITH_EXACT_TOOLCHAIN')
                            for arm in ['baseline', 'candidate']})


def validate(config, pairs, controls):
    for name, value in [('pairs', pairs), ('controls', controls)]:
        if type(value) is not int or value < 8:
            raise ValueError(f'{name} requires at least eight independent mounts')
    if config.get('schema') != 1:
        raise ValueError('manifest schema must be 1')
    for key in ['memory_limit_mb', 'entry_limit', 'dataset_bytes', 'l1_hot_bytes',
                'eviction_bytes', 'disk_capacity_bytes', 'metadata_entries', 'random_operations',
                'random_block_bytes', 'concurrency', 'writer_bytes', 'cpu_quota_us',
                'cpu_period_us', 'cgroup_memory_max_mb']:
        if type(config.get(key)) is not int or config[key] <= 0:
            raise ValueError(f'positive integer required: {key}')
    memory = config['memory_limit_mb'] * MIB
    if not config['l1_hot_bytes'] <= memory // 16 < memory < config['dataset_bytes'] < config['disk_capacity_bytes'] < config['eviction_bytes']:
        raise ValueError('require small L1 set, dataset > RAM, L2 capacity > dataset, eviction set > L2')
    if config['random_block_bytes'] >= config['l1_hot_bytes'] or config['eviction_bytes'] > 2 * 1024 ** 3:
        raise ValueError('range must fit every dataset; local fixture bounded at 2 GiB')
    if config.get('states') != STATES or config.get('workloads') != WORKLOADS:
        raise ValueError('explicit complete state/workload matrix required; cold requested is not verified cold')
    mapping = config.get('disk_metrics', {})
    if set(mapping) != set(example_manifest()['disk_metrics']) or any(
            not isinstance(value, str) or not re.fullmatch(r'[a-zA-Z_:][a-zA-Z0-9_:]*(?:\{(?:scope="queued_and_active"|phase="(?:startup|allocation|recovery)")\})?', value)
            for value in mapping.values()):
        raise ValueError('disk_metrics must explicitly map every required counter/gauge to an exact runtime name')
    if config.get('unavailable_disk_observations') != example_manifest()['unavailable_disk_observations']:
        raise ValueError('unavailable_disk_observations must retain actual adapter instrumentation gaps')
    if not isinstance(config.get('metadata_limit'), str) or not config['metadata_limit']:
        raise ValueError('explicit metadata_limit required independently of payload limit')
    duration = config.get('minimum_active_seconds')
    if type(duration) not in (int, float) or not math.isfinite(duration) or not 0 < duration <= 30:
        raise ValueError('minimum_active_seconds must be finite, positive and <= 30')
    targets = config.get('superiority_targets')
    groups = {f'{name}/{state}' for name in WORKLOADS for state in
              (STATES if name in READS else ['normal']) if not state.endswith('cold_requested')}
    if not isinstance(targets, dict) or not targets or any(
            group not in groups or not isinstance(metrics, list) or not metrics or set(metrics) - set(METRICS)
            for group, metrics in targets.items()):
        raise ValueError('superiority targets must declare observed workload/state metrics; advisory cold cannot qualify')
    for metric, margin in config.get('non_inferiority_margins', {}).items():
        if metric not in METRICS or type(margin) not in (int, float) or not math.isfinite(margin) or margin < 1:
            raise ValueError('invalid predeclared non-inferiority margin')


def cases(config, pairs, controls):
    orders = [('baseline', 'candidate', 'control'), ('candidate', 'control', 'baseline'),
              ('control', 'baseline', 'candidate'), ('control', 'candidate', 'baseline')]
    for workload in config['workloads']:
        for state in config['states'] if workload in READS else ['normal']:
            for pair in range(max(pairs, controls)):
                for arm in orders[pair % len(orders)]:
                    if (arm == 'control' and pair >= controls) or (arm == 'candidate' and pair >= pairs):
                        continue
                    yield dict(state=state, workload=workload, pair=pair, arm=arm,
                               mount_id=f'{workload}-{state}-{pair:02d}-{arm}')


def prepare_output(path):
    if path.exists() and any(path.iterdir()):
        raise ValueError('output is not empty; preserve existing evidence')
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.chmod(0o700)


def parse_metrics(text, mapping):
    samples = {}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) == 2 and not fields[0].startswith('#'):
            samples[fields[0]] = float(fields[1])
    result = {}
    for meaning, name in mapping.items():
        value = samples.get(name)
        if (meaning == 'opened_file_allocated_bytes' and value is None
                and samples.get(mapping.get('opened_file_available')) == 0):
            result[meaning] = None
            continue
        if value is None or not math.isfinite(value) or value < 0:
            raise RuntimeError(f'missing/invalid runtime metric {meaning}: {name}; runtime may still be pending')
        result[meaning] = value
    return result


def delta(before, after, name):
    if name not in before or name not in after:
        raise RuntimeError(f'missing cache evidence: {name}')
    if after[name] < before[name]:
        raise RuntimeError(f'counter reset within one mount: {name}')
    return after[name] - before[name]


def verify_state(state, before, after, arm):
    if state == 'l1_hot':
        if delta(before, after, 'l1_hit_bytes') <= 0:
            raise RuntimeError('L1-hot requested but no observed L1 hit; client pagecache is not an L1 hit')
        if delta(before, after, 'remote_bytes') != 0:
            raise RuntimeError('L1-hot incurred remote payload reads')
    if state.startswith('l2_hot') and arm == 'candidate':
        if delta(before, after, 'l2_hit_bytes') <= 0:
            raise RuntimeError('L2-hot requested but no observed L2 hit; runtime/eviction/recovery pending')
        if delta(before, after, 'remote_bytes') != 0:
            raise RuntimeError('L2-hot incurred remote payload reads; inventory coverage incomplete')


def eviction_probe(path, size):
    # A FUSE read can block: keep this untimed correctness probe in a bounded
    # child, using the same independent fixture-byte oracle as the worker.
    script = ('import sys; from integration import expected_range; '
              'f=open(sys.argv[1], "rb", buffering=0); n=int(sys.argv[2]); '
              'data=f.read(n)\nif data != expected_range(0, n):\n'
              ' raise AssertionError("eviction probe byte mismatch")')
    try:
        integration.command([sys.executable, '-c', script, str(path / 'payload'), str(size)],
                            cwd=Path(integration.__file__).resolve().parent,
                            capture_output=True, text=True, timeout=30)
    except subprocess.CalledProcessError as error:
        raise AssertionError('eviction probe failed: ' + error.stderr) from error
    return size


def verify_eviction(before, after, probe_before, probe_after):
    # Caller has already established a byte-correct L2 hit for this exact prefix
    # before pressure, and byte correctness of its post-pressure source fallback.
    reclaims = delta(before, after, 'region_reclaims_started')
    misses = delta(probe_before, probe_after, 'l2_misses')
    remote = delta(probe_before, probe_after, 'remote_bytes')
    if reclaims <= 0 or misses <= 0 or remote <= 0:
        raise RuntimeError('insufficient eviction evidence: require region pressure, probe miss and source bytes')
    return dict(region_reclaims_started=reclaims, probe_misses=misses, probe_remote_bytes=remote,
                completed_reclaims_measured=False, scope='previously L2-hit prefix, byte-correct source fallback after pressure; not per-record eviction accounting')


def budget_comparison(baseline, candidate, memory_mb):
    total = memory_mb * MIB
    if any(value.get('payload_limit_bytes') != total for value in [baseline, candidate]):
        raise RuntimeError('effective payload budgets missing or not constant')
    if any(type(value.get('read_cache_policy_bytes')) is not int for value in [baseline, candidate]):
        raise RuntimeError('missing effective L1 capacity')
    return dict(payload_limit_bytes=total, baseline_effective_l1_bytes=baseline['read_cache_policy_bytes'],
                candidate_effective_l1_bytes=candidate['read_cache_policy_bytes'],
                l1_reduction_bytes=baseline['read_cache_policy_bytes'] - candidate['read_cache_policy_bytes'],
                disk_memory_is_free=False)


def cache_page_state(directory, cold):
    """Advice only on this invocation's private cache; never drop host caches.

    DONTNEED is advisory and kernel/device residency is not proven. Cold results
    remain labelled requested/uncontrolled and cannot establish cold superiority.
    """
    files, total = 0, 0
    for path in sorted(directory.rglob('*')):
        if path.is_symlink():
            raise RuntimeError('refusing cache symlink traversal')
        if not path.is_file():
            continue
        with path.open('rb', buffering=0) as stream:
            if cold:
                os.posix_fadvise(stream.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
            else:
                while stream.read(MIB):
                    pass
            total += os.fstat(stream.fileno()).st_size
            files += 1
    return dict(files=files, logical_bytes=total,
                protocol='owned-file-POSIX_FADV_DONTNEED' if cold else 'owned-file-read-through',
                cold_verified=False, no_host_cache_drop=True)


def capabilities(cgroup_root):
    result = dict(system=platform.platform(), kernel=platform.release(), machine=platform.machine(),
                  cpu_count=os.cpu_count(), cgroup_root=str(cgroup_root) if cgroup_root else None,
                  cpu_affinity=sorted(os.sched_getaffinity(0)),
                  tools={name: shutil.which(name) for name in ['fusermount3', 'unshare', 'nginx', 'rsync', 'mount', 'umount', 'sha256sum']},
                  python=sys.version, modules={name: importlib.util.find_spec(name) is not None
                                              for name in ['boto3', 'moto', 'aiohttp']}, errors=[])
    try:
        with open('/dev/fuse', 'r+b', buffering=0):
            result['fuse_open'] = True
    except OSError as error:
        result['fuse_open'] = False
        result['errors'].append(f'/dev/fuse: {error}')
    if result['tools']['unshare']:
        probe = subprocess.run(['unshare', '--mount', '--propagation', 'private', 'true'],
                               capture_output=True, text=True, timeout=5)
        result['mount_namespace'] = probe.returncode == 0
        if probe.returncode:
            result['errors'].append('private mount namespace: ' + probe.stderr.strip())
    for name, path in result['tools'].items():
        if not path:
            result['errors'].append(f'missing tool: {name}')
    result['tool_sha256'] = {name: integration.sha_file(path) for name, path in result['tools'].items() if path}
    for name, present in result['modules'].items():
        if not present:
            result['errors'].append(f'missing Python module: {name}')
    try:
        root = compare.cgroup_preflight(cgroup_root)
        with compare.cgroup(root, example_manifest()) as probe:
            result['ephemeral_limits_probe'] = {name: (probe / name).read_text().strip()
                                                for name in ['cpu.max', 'memory.max']}
        result['cgroup_delegated'] = True
        result['cgroup_controllers'] = (root / 'cgroup.controllers').read_text().split()
    except (OSError, RuntimeError) as error:
        result['cgroup_delegated'] = False
        result['errors'].append(str(error))
    # Discovery is deliberately limited to an explicit environment handoff.
    reference = os.environ.get('GEESEFS_GO_REFERENCE')
    result['go_reference_handoff'] = reference
    result['go_reference_required_for_whole_acceptance'] = integration.UPSTREAM
    if reference:
        try:
            result['go_reference_revision'] = integration.command(['git', 'rev-parse', 'HEAD'],
                cwd=reference, capture_output=True, text=True, timeout=5).stdout.strip()
        except (OSError, subprocess.SubprocessError) as error:
            result['go_reference_error'] = str(error)
    return result


def provenance(path, receipt):
    path = path.resolve()
    if not path.is_file() or not os.access(path, os.X_OK):
        raise RuntimeError(f'executable unavailable: {path}')
    if integration.sha_file(path) != receipt.get('binary_sha256'):
        raise RuntimeError('frozen executable SHA256 differs from manifest receipt')
    if not re.fullmatch('[0-9a-f]{40}', receipt.get('source_revision', '')):
        raise RuntimeError('exact frozen source revision required')
    if not receipt.get('build_command') or not receipt.get('toolchain'):
        raise RuntimeError('exact build command/toolchain receipt required')
    help_text = integration.command([path, '--help'], capture_output=True, text=True, timeout=10).stdout
    return dict(receipt, path=str(path), help_sha256=hashlib.sha256(help_text.encode()).hexdigest(),
                linkage='operator-supplied frozen-build receipt; executable bytes independently checked'), help_text


def sample_endpoint(port, mapping):
    with urllib.request.urlopen(f'http://127.0.0.1:{port}/metrics', timeout=3) as response:
        return parse_metrics(response.read(2 * MIB).decode(), mapping)


def native_snapshot(port, config, output, label, require_adapter=True):
    """Preserve failure evidence, including SDK GET attempts independent of fill bytes."""
    with urllib.request.urlopen(f'http://127.0.0.1:{port}/metrics', timeout=3) as response:
        text = response.read(2 * MIB).decode()
    (output / (label + '-metrics.txt')).write_text(text)
    mapping = config['disk_metrics'] if require_adapter else {
        key: config['disk_metrics'][key] for key in ['l1_hit_bytes', 'remote_bytes', 'l1_resident_bytes', 'l1_loaned_bytes']}
    values = parse_metrics(text, mapping)
    if not require_adapter:
        values.update(parse_metrics(text, dict(state_bypass='geesefs_disk_cache_state{state="bypass"}',
            stats_available='geesefs_disk_cache_stats_available')))
    # Diagnostics expose transmission attempts, not guaranteed server receipts.
    # The family is sparse before the first GET, so enabled + absent means zero.
    enabled = parse_metrics(text, {'enabled': 'geesefs_diagnostics_enabled'})['enabled']
    if enabled != 1:
        raise RuntimeError('native GET-attempt verification requires enabled diagnostics')
    values.update(parse_metrics(text, dict(fuse_reads='geesefs_fuse_replies_total{opcode="read",errno="OK"}')))
    values['source_get_attempts'] = sum(float(line.split()[1]) for line in text.splitlines()
        if line.startswith('geesefs_s3_attempts_total{') and 'operation="get"' in line)
    (output / (label + '-metrics.json')).write_text(json.dumps(values, indent=2) + '\n')
    return values


def io_counters(pid):
    try:
        return {name.rstrip(':'): int(value) for name, value in
                (line.split() for line in Path(f'/proc/{pid}/io').read_text().splitlines())}
    except OSError:
        return None


def dataset_config(config, state, workload):
    result = dict(config)
    result['dataset_bytes'] = (config['l1_hot_bytes'] if state == 'l1_hot' else
                               config['eviction_bytes'] if workload == 'eviction' else config['dataset_bytes'])
    return result


def drop_client_pages(path):
    # Only clean fixture files in the invocation's own mount. Advice is recorded,
    # and hit counters, not the syscall succeeding, validate the serving tier.
    with (path / 'payload').open('rb', buffering=0) as stream:
        os.posix_fadvise(stream.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)


def mixed(path, config):
    import random
    from metrics import latency_percentiles
    generator = random.Random(1729)
    calls, transferred = [], 0
    started = time.perf_counter()
    with (path / 'payload').open('rb', buffering=0) as stream:
        for number in range(config['random_operations']):
            bound = min(config['dataset_bytes'], MIB) if number % 5 else config['dataset_bytes']
            offset = generator.randrange(bound - config['random_block_bytes'])
            before = time.perf_counter_ns()
            data = os.pread(stream.fileno(), config['random_block_bytes'], offset)
            calls.append((time.perf_counter_ns() - before) / 1e6)
            if data != integration.expected_range(offset, config['random_block_bytes']):
                raise AssertionError('mixed hot/cold bytes')
            transferred += len(data)
    elapsed = time.perf_counter() - started
    return dict(throughput=transferred / elapsed, throughput_unit='bytes/s', bytes=transferred,
                operations=len(calls), elapsed_seconds=elapsed, latencies_ms=calls, **latency_percentiles(calls))


def worker_main():
    import boto3
    from botocore.config import Config
    spec = json.loads(sys.stdin.readline())
    client = boto3.client('s3', endpoint_url=spec['endpoint'], region_name='us-east-1',
                          aws_access_key_id=os.environ['AWS_ACCESS_KEY_ID'],
                          aws_secret_access_key=os.environ['AWS_SECRET_ACCESS_KEY'],
                          config=Config(connect_timeout=3, read_timeout=30, retries={'max_attempts': 1}))
    try:
        path = Path(spec['path'])
        if spec['state'] == 'l1_hot':
            compare.workload(path, 'sequential', spec['config'], client, spec['bucket'],
                             spec['prefix'], spec['endpoint'], True, spec['expected_sha'])
        drop_client_pages(path)
        print(json.dumps({'ready': True}), flush=True)
        sys.stdin.readline()
        from metrics import latency_percentiles
        started, calls, transferred, cycles = time.perf_counter(), [], 0, 0
        while True:
            if spec['name'] in READS + ['eviction']:
                drop_client_pages(path)
            metrics = (mixed(path, spec['config']) if spec['name'] == 'mixed' else
                       compare.workload(path, spec['name'], spec['config'], client, spec['bucket'],
                                        spec['prefix'], spec['endpoint'], True, spec['expected_sha']))
            calls.extend(metrics['latencies_ms'])
            transferred += metrics['bytes']
            cycles += 1
            if (spec['state'].endswith('cold_requested') or spec['state'] == 'both_cold'
                    or time.perf_counter() - started >= spec['config']['minimum_active_seconds']):
                break
        elapsed = time.perf_counter() - started
        metrics.update(bytes=transferred, operations=len(calls), elapsed_seconds=elapsed,
                       throughput=(transferred or len(calls)) / elapsed, cycles=cycles,
                       independent_mounts=1, latencies_ms=calls, latency_variance_ms2=statistics.pvariance(calls),
                       **latency_percentiles(calls))
        print(json.dumps({'metrics': metrics}), flush=True)
        sys.stdin.readline()  # Remain alive for CPU/RSS accounting.
    except Exception as error:
        print(json.dumps({'error': str(error)}), flush=True)
        return 1
    finally:
        client.close()
    return 0


@contextlib.contextmanager
def worker(spec, environment, group, log):
    with log.open('wb') as stderr:
        process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '--worker'],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr,
                                   env=environment, text=True,
                                   preexec_fn=lambda: integration.enter_cgroup(group))
        try:
            process.stdin.write(json.dumps(spec) + '\n')
            process.stdin.flush()
            if compare.read_message(process, 120) != {'ready': True}:
                raise RuntimeError('L2 worker did not become ready')
            yield process
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            process.stdout.close()
            if process.returncode and sys.exc_info()[0] is None:
                raise RuntimeError(f'L2 worker failed ({process.returncode}); see {log.name}')


def mount_extra(config, arm, cache, stats, port, read_only=False):
    result = ['--entry-limit', str(config['entry_limit']), '--metadata-limit', config['metadata_limit'],
              '--stats-file', str(stats), '--metrics-listen', f'127.0.0.1:{port}']
    if arm == 'candidate':
        result += ['--disk-cache-dir', str(cache), '--disk-cache-max-size', str(config['disk_capacity_bytes']),
                   '--disk-cache-regex', r'/(payload|package\.rpm)$']
    if read_only:
        # The existing explicit read-only FUSE direct-I/O mode prevents the
        # client kernel page cache from masquerading as application L1/L2 hits.
        # Writer/native durability checks retain normal writable semantics.
        result += ['--read-only-direct-io', '-o', 'ro']
    return result


def disk_idle(port, config):
    deadline = time.monotonic() + 10
    while True:
        snapshot = sample_endpoint(port, config['disk_metrics'])
        if snapshot['active_jobs'] == 0 and snapshot['submission_bytes'] == 0 and snapshot['submission_items'] == 0:
            return snapshot
        if time.monotonic() >= deadline:
            raise RuntimeError('optional spill did not drain within ten seconds')
        time.sleep(0.02)


def disk_ready(port, config):
    """FUSE visibility does not imply that asynchronous disk recovery is Ready."""
    wait_disk_state(port, 'ready')
    return sample_endpoint(port, config['disk_metrics'])


def wait_disk_state(port, expected):
    deadline = time.monotonic() + 30
    while True:
        state = sample_endpoint(port, dict(ready='geesefs_disk_cache_state{state="ready"}',
            bypass='geesefs_disk_cache_state{state="bypass"}',
            closed='geesefs_disk_cache_state{state="closed"}', available='geesefs_disk_cache_stats_available'))
        if state[expected] and (expected != 'ready' or state['available']):
            return state
        if state['bypass'] or state['closed']:
            raise RuntimeError('disk adapter bypassed/closed before native workload: ' + json.dumps(state))
        if time.monotonic() >= deadline:
            raise RuntimeError(f'disk adapter not {expected.title()} within thirty seconds: ' + json.dumps(state))
        time.sleep(0.02)


def verify_native_hot(before, after, size):
    verify_state('l2_hot_pagecache_hot', before, after, 'candidate')
    if delta(before, after, 'source_get_attempts') != 0:
        raise RuntimeError('native recovered L2 package caused source GET attempts')
    if delta(before, after, 'l2_hit_bytes') < size or delta(before, after, 'fuse_reads') <= 0:
        raise RuntimeError('native warm declaration requires full L2 bytes and observed FUSE reads, not client pagecache')


def verify_startup_fallback(before, after, size):
    if any(row['state_bypass'] != 1 or row['stats_available'] != 0 for row in [before, after]):
        raise RuntimeError('startup fault requires Bypass without an attached adapter snapshot')
    if (delta(before, after, 'remote_bytes') < size or delta(before, after, 'source_get_attempts') <= 0
            or delta(before, after, 'fuse_reads') <= 0):
        raise RuntimeError('startup fault requires actual FUSE source reads and full validated payload bytes')


def population_extra(config, arm, cache, stats, port):
    # Priming only: identical logical windows in every arm. Measurement mounts
    # retain their ordinary adaptive read-ahead (initially 5 MiB).
    extra = mount_extra(config, arm, cache, stats, port, read_only=True)
    for flag in ['--read-ahead', '--read-ahead-small', '--read-ahead-large', '--read-ahead-parallel']:
        extra += [flag, str(POPULATION_READ_AHEAD_KIB)]
    return extra


def prime(binary, fixture, work, record, config, cache, group, output):
    # A fixed protocol in every baseline/candidate/control arm, not a
    # candidate-only retry-until-hot. Warmup mounts are not measured mount pairs.
    deadline = time.monotonic() + POPULATION_SECONDS
    rounds = record['population_rounds'] = []
    record['population_policy'] = dict(read_ahead_kib=POPULATION_READ_AHEAD_KIB,
        measurement_read_ahead='unchanged defaults; initial 5 MiB', fixed_mounts=POPULATION_ROUNDS,
        eviction_warm_bytes=EVICTION_PIPELINE_BYTES, eviction_requested_probe_bytes=min(MIB, config['l1_hot_bytes']))
    for number in range(POPULATION_ROUNDS):
        if time.monotonic() >= deadline:
            raise RuntimeError('population protocol exceeded its fixed deadline')
        label = record['mount_id'] + f'-prime-{number}'
        port = integration.free_port()
        extra = population_extra(config, record['arm'], cache, output / (label + '-stats.jsonl'), port)
        row = dict(round=number, independent_measured_mount=False)
        rounds.append(row)
        with integration.mount(binary, fixture['bucket'], fixture['prefix'], fixture['endpoint'], fixture['environment'],
                               work, label, config['memory_limit_mb'], extra, group, row) as (path, daemon):
            if record['arm'] == 'candidate':
                disk_ready(port, config)
            row['before'] = native_snapshot(port, config, output, label + '-before', require_adapter=record['arm'] == 'candidate')
            if record['workload'] == 'eviction':
                eviction_probe(path, EVICTION_PIPELINE_BYTES)
            else:
                native_hash(path / 'payload', fixture['expected_sha'], group, max(1, min(30, deadline - time.monotonic())))
            native_hash(path / 'pressure', fixture['pressure_sha'], group, max(1, min(30, deadline - time.monotonic())))
            if record['arm'] == 'candidate':
                disk_idle(port, config)
            row['after'] = native_snapshot(port, config, output, label + '-after', require_adapter=record['arm'] == 'candidate')
            if delta(row['before'], row['after'], 'fuse_reads') <= 0:
                raise RuntimeError('population I/O did not reach FUSE')
        row.update(daemon_exit=daemon.returncode, mount_present_after=os.path.ismount(path))
    if time.monotonic() >= deadline:
        raise RuntimeError('population protocol exceeded its fixed deadline')
    # Validate logical coverage BEFORE hot/advisory-cold pagecache preparation.
    # Otherwise this full read would itself invalidate the requested media state.
    label, port = record['mount_id'] + '-prime-validation', integration.free_port()
    extra = mount_extra(config, record['arm'], cache, output / (label + '-stats.jsonl'), port, read_only=True)
    validation = record['population_validation'] = dict(independent_measured_mount=False)
    with integration.mount(binary, fixture['bucket'], fixture['prefix'], fixture['endpoint'], fixture['environment'],
                           work, label, config['memory_limit_mb'], extra, group, validation) as (path, daemon):
        if record['arm'] == 'candidate':
            disk_ready(port, config)
        before = native_snapshot(port, config, output, label + '-before', require_adapter=record['arm'] == 'candidate')
        if record['workload'] == 'eviction':
            size = EVICTION_PIPELINE_BYTES
            eviction_probe(path, min(MIB, config['l1_hot_bytes']))
        else:
            size = config['dataset_bytes']
            native_hash(path / 'payload', fixture['expected_sha'], group, max(1, min(30, deadline - time.monotonic())))
        after = native_snapshot(port, config, output, label + '-after', require_adapter=record['arm'] == 'candidate')
        validation.update(before=before, after=after, byte_correct=True)
        if record['arm'] == 'candidate':
            verify_native_hot(before, after, size)
        elif delta(before, after, 'fuse_reads') <= 0:
            raise RuntimeError('population validation bypassed FUSE')
    validation.update(daemon_exit=daemon.returncode, mount_present_after=os.path.ismount(path))
    if time.monotonic() >= deadline:
        raise RuntimeError('population protocol exceeded its fixed deadline')


def allocator_snapshot(path, started_ns, finished_ns):
    # The daemon still appends while the worker is alive. Preserve the original
    # log and feed the shared accounting reader only newline-terminated frames;
    # malformed complete frames still fail rather than becoming missing samples.
    if not path.exists():
        return compare.allocator_rust(path, started_ns, finished_ns)
    data = path.read_bytes()
    snapshot = path.with_name(path.name + '.sampled')
    snapshot.write_bytes(data[:data.rfind(b'\n') + 1])
    return compare.allocator_rust(snapshot, started_ns, finished_ns)


def measured_run(binary, fixture, work, record, config, group_root, output):
    label = record['mount_id']
    cache = work / (label + '-cache')
    cache.mkdir(mode=0o700)
    space = os.statvfs(cache)
    record['storage'] = dict(device_number=cache.stat().st_dev, filesystem_block_size=space.f_frsize,
                            available_bytes=space.f_bavail * space.f_frsize,
                            media='unclassified local filesystem; no SSD/HDD generalization')
    stats, port = output / (label + '-stats.jsonl'), integration.free_port()
    with compare.cgroup(group_root, config) as group:
        if record['state'].startswith('l2_hot') or record['workload'] == 'eviction':
            prime(binary, fixture, work, record, config, cache, group, output)
            record['pagecache_protocol'] = cache_page_state(cache, cold=record['state'].endswith('cold_requested'))
        extra = mount_extra(config, record['arm'], cache, stats, port, read_only=record['workload'] != 'writer_fsync')
        record['client_pagecache_protocol'] = ('read-only FUSE direct-I/O; local L2 file pagecache separate'
                                               if record['workload'] != 'writer_fsync' else 'ordinary writable FUSE, fsync verified remotely')
        started = time.monotonic()
        with integration.mount(binary, fixture['bucket'], fixture['prefix'], fixture['endpoint'], fixture['environment'],
                               work, label, config['memory_limit_mb'], extra, group, record) as (path, daemon):
            if record['arm'] == 'candidate':
                disk_ready(port, config)
            record['mount_and_recovery_seconds'] = time.monotonic() - started
            if record['workload'] == 'eviction':
                # Outside the timed workload; warm the same prefix in every arm.
                # The candidate must prove it persisted, not just touched, the seed.
                seed_before = native_snapshot(port, config, output, label + '-eviction-seed-before') if record['arm'] == 'candidate' else None
                eviction_probe(path, min(MIB, config['l1_hot_bytes']))
                if seed_before is not None:
                    disk_idle(port, config)
                    seed_after = native_snapshot(port, config, output, label + '-eviction-seed-after')
                    verify_native_hot(seed_before, seed_after, EVICTION_PIPELINE_BYTES)
                    record['eviction_seed'] = dict(before=seed_before, after=seed_after, byte_correct=True)
            spec = {key: fixture[key] for key in ['bucket', 'prefix', 'endpoint', 'expected_sha']}
            spec.update(path=str(path), name=record['workload'], state=record['state'], config=config)
            with worker(spec, fixture['environment'], group, output / (label + '-client.log')) as consumer:
                mapping = config['disk_metrics'] if record['arm'] == 'candidate' else {
                    key: config['disk_metrics'][key] for key in ['l1_hit_bytes', 'remote_bytes', 'l1_resident_bytes', 'l1_loaned_bytes']}
                before = sample_endpoint(port, mapping)
                io_before = io_counters(daemon.pid)
                with compare.Sampler(daemon.pid, consumer.pid, group, output / (label + '-memory.jsonl')) as sampler:
                    start_ns = time.time_ns()
                    consumer.stdin.write('start\n')
                    consumer.stdin.flush()
                    message = compare.read_message(consumer, 300)
                    end_ns = time.time_ns()
                    if 'error' in message:
                        raise RuntimeError(message['error'])
                    metrics = message['metrics']
                    drain_start = time.monotonic()
                    if record['arm'] == 'candidate':
                        disk_idle(port, config)
                    drain_seconds = time.monotonic() - drain_start
                    metrics['spill_drain_seconds'] = drain_seconds
                    metrics['workload_elapsed_seconds'] = metrics['elapsed_seconds']
                    metrics['elapsed_seconds'] += drain_seconds
                    metrics['throughput'] = (metrics['bytes'] or metrics['operations']) / metrics['elapsed_seconds']
                    end_ns = time.time_ns()
                after = sample_endpoint(port, mapping)
                metrics.update(cpu_efficiency(sampler.metrics(), transferred=metrics['bytes'], operations=metrics['operations']))
                allocator_peak, allocator_steady, effective = allocator_snapshot(stats, start_ns, end_ns)
                metrics.update(allocator_peak_bytes=allocator_peak, allocator_steady_bytes=allocator_steady)
                # Keep observed resources even when a workload's state gate fails.
                record.update(metrics=metrics, cache_before=before, cache_after=after,
                    effective_budgets=effective,
                    cpu_counters={'before': sampler.cpu.before, 'after': sampler.cpu.after},
                    effective_cgroup_limits={name: (group / name).read_text().strip() for name in ['cpu.max', 'memory.max']},
                    l1_residency_scope='pre/post visible retained bytes and idle-flush loan bytes, not peak backing allocation; effective L1 policy is an upper bound including loans; Ready selected victims may exclude the idle flush lane')
                verify_state(record['state'], before, after, record['arm'])
                if record['workload'] == 'eviction' and record['arm'] == 'candidate':
                    eviction_probe(path, min(MIB, config['l1_hot_bytes']))
                    probe_after = disk_idle(port, config)
                    record['eviction_evidence'] = verify_eviction(before, after, after, probe_after)
                    record['eviction_evidence']['probe_byte_correct'] = True
                    record['eviction_evidence']['timing_scope'] = 'seed and fallback probes excluded from timed pressure workload'
                    record['eviction_probe_after'] = probe_after
                record.update(metrics=metrics, cache_before=before, cache_after=after, effective_budgets=effective,
                              daemon_io_before=io_before, daemon_io_after=io_counters(daemon.pid),
                              cpu_counters={'before': sampler.cpu.before, 'after': sampler.cpu.after},
                              correctness=True, independent_mount=True)
                record['effective_cgroup_limits'] = {name: (group / name).read_text().strip()
                                                     for name in ['cpu.max', 'memory.max']}
                # Expose mount-lifetime I/O, not fabricated hardware IOPS. Kernel
                # coalescing/read-ahead makes syscr/syscw syscall counts, not IOPS.
                record['disk_io_scope'] = '/proc daemon read_bytes/write_bytes/syscr/syscw; hardware IOPS unavailable'
                record['cache_resource_samples_scope'] = 'pre/post index/recency entry counts and queued+active submission ownership; no measured index heap or transient peaks'
                if record['daemon_io_before'] is not None and record['daemon_io_after'] is not None:
                    changes = {name: delta(record['daemon_io_before'], record['daemon_io_after'], name)
                               for name in ['read_bytes', 'write_bytes', 'syscr', 'syscw']}
                    record['daemon_io_delta'] = changes
                    record['write_amplification_vs_consumed_payload'] = changes['write_bytes'] / metrics['bytes'] if metrics['bytes'] else None
                    record['pagecache_observation'] = dict(storage_read_bytes=changes['read_bytes'],
                        cold_verified=False, note='DONTNEED is advisory; storage reads are not proof of fully cold media')
    record['disk_allocated_bytes'] = sum(p.stat().st_blocks * 512 for p in cache.rglob('*') if p.is_file())
    record['disk_allocated_bytes_scope'] = 'filesystem st_blocks times 512 summed over owned cache files after clean unmount; not live occupancy or unique physical device allocation'
    return record


def summarize(records, config, pairs, controls):
    errors, summaries, budgets = [], {}, []
    indexed = {(r['workload'], r['state'], r['pair'], r['arm']): r for r in records}
    if not records:
        errors.append('missing independent mount observations')
    for workload in config['workloads']:
        for state in config['states'] if workload in READS else ['normal']:
            group_errors = []
            ratios, aa = {key: [] for key in METRICS}, {key: [] for key in METRICS}
            for pair in range(max(pairs, controls)):
                base, candidate, control = [indexed.get((workload, state, pair, arm))
                                            for arm in ['baseline', 'candidate', 'control']]
                for other, target, required in [(candidate, ratios, pair < pairs), (control, aa, pair < controls)]:
                    if not required:
                        continue
                    if not base or not other or not all(r.get('correctness') and r.get('independent_mount') for r in [base, other]):
                        group_errors.append(f'missing/failed mount: {workload}/{state}/{pair}')
                        continue
                    for name in METRICS:
                        left, right = other.get('metrics', {}).get(name), base.get('metrics', {}).get(name)
                        if (type(left) not in (int, float) or type(right) not in (int, float)
                                or not math.isfinite(left) or not math.isfinite(right) or left <= 0 or right <= 0):
                            group_errors.append(f'missing/nonpositive measurement: {workload}/{state}/{pair}/{name}')
                            continue
                        target[name].append(right / left if name == 'throughput' else left / right)
                if base and candidate:
                    if workload == 'eviction':
                        try:
                            seed = candidate['eviction_seed']
                            if not seed.get('byte_correct') or not candidate['eviction_evidence'].get('probe_byte_correct'):
                                raise RuntimeError('eviction probe byte correctness missing')
                            verify_state('l2_hot_pagecache_hot', seed['before'], seed['after'], 'candidate')
                            verify_eviction(candidate['cache_before'], candidate['cache_after'],
                                            candidate['cache_after'], candidate['eviction_probe_after'])
                        except (KeyError, RuntimeError) as error:
                            group_errors.append(f'missing/invalid eviction evidence: {error}')
                    try:
                        budgets.append(budget_comparison(base.get('effective_budgets') or {}, candidate.get('effective_budgets') or {}, config['memory_limit_mb']))
                    except RuntimeError as error:
                        group_errors.append(str(error))
            group = {name: assess(ratios[name], aa[name], margin=config.get('non_inferiority_margins', {}).get(name))
                     for name in METRICS}
            summaries[f'{workload}/{state}'] = group
            for name, result in group.items():
                targets = config['superiority_targets'].get(f'{workload}/{state}', [])
                allowed = ['improved'] if name in targets else ['improved', 'non_inferior', 'within_noise']
                if result['status'] not in allowed:
                    group_errors.append(f'{workload}/{state}/{name}: {result["status"]}')
            if state.endswith('cold_requested'):
                summaries[f'{workload}/{state}']['cold_state_evidence'] = dict(
                    qualified=False, status='advisory/uncontrolled', errors=sorted(set(group_errors)))
            else:
                errors.extend(group_errors)
    observed_passed = bool(records) and not errors
    gaps = config.get('unavailable_disk_observations', [])
    return dict(qualified=observed_passed, observed_state_screen_passed=observed_passed,
                full_matrix_qualified=False,
                production_accepted=False, errors=sorted(set(errors)),
                 summaries=summaries, effective_budget_comparisons=budgets,
                 unavailable_disk_observations=gaps,
                qualification_scope='local observed-state Rust screening; excludes advisory cold, hardware-media claims and Go-relative acceptance',
                limitations=['page-cache-cold is advisory/unverified; cold superiority remains inconclusive',
                             'hardware IOPS and SSD/HDD applicability unavailable',
                              'submission ownership/index entry counts sampled only before/after; not measured heap or transient peaks',
                              'adapter durations are count/sum/max aggregates, not percentiles; file allocation is captured once at open, not live occupancy',
                              'region picker callbacks are pressure evidence only; not completed per-record evictions'],
                go_reference_required=integration.UPSTREAM,
                scope='local Rust qualification only; no Go 0.43.9 comparative acceptance')


def native_hash(path, expected, group, timeout=30):
    result = integration.command(['sha256sum', '--', path], capture_output=True, text=True, timeout=timeout,
                                 preexec_fn=lambda: integration.enter_cgroup(group))
    if result.stdout.split()[0] != expected:
        raise AssertionError('native independent SHA256 mismatch: ' + str(path))


def native_read_cache_phase(binary, fixture, package, work, config, group_root, output):
    """Bounded warmup, then a separate fresh-mount full-coverage declaration.

    A warmup mount is NOT an independent measured pair. Best-effort drops are
    allowed. Fresh RO/direct-I/O mounts ensure both empty L1 and actual FUSE reads
    each round; no writer uses the direct-I/O flag. Budgets/data stay unchanged.
    """
    cache = work / 'native-reader-cache'
    cache.mkdir(mode=0o700)
    deadline = time.monotonic() + POPULATION_SECONDS
    expected_records = (len(package) + 128 * 1024 - 1) // (128 * 1024)
    receipt = dict(warm_declared=False, max_rounds=POPULATION_ROUNDS, fixed_rounds=POPULATION_ROUNDS, warmup_deadline_seconds=POPULATION_SECONDS,
                   protocol='fixed eight fresh RO/direct-I/O population mounts, full package + excluded pressure, drain, clean shutdown; fresh RO validation mount; same fixed population schedule in matrix baseline/candidate/control',
                   independent_measured_mounts=0, expected_package_records=expected_records, rounds=[])
    package_sha = hashlib.sha256(package).hexdigest()
    try:
        with compare.cgroup(group_root, config) as group:
            for number in range(receipt['max_rounds']):
                if time.monotonic() >= deadline:
                    raise RuntimeError('native cache population exceeded 180-second deadline')
                label, port = f'reader-warm-{number}', integration.free_port()
                # Keep the established small native-package protocol. Its 1 MiB
                # object ends before the default 5 MiB window; reducing excluded
                # pressure windows here only multiplies Moto full-object reads.
                extra = mount_extra(config, 'candidate', cache, output / (label + '-stats.jsonl'), port, read_only=True)
                row = dict(round=number, independent_measured_mount=False)
                receipt['rounds'].append(row)
                with integration.mount(binary, fixture['bucket'], fixture['prefix'], fixture['endpoint'], fixture['environment'],
                                       work, label, config['memory_limit_mb'], extra, group, row) as (path, daemon):
                    disk_ready(port, config)
                    before = native_snapshot(port, config, output, label + '-before')
                    row['before'] = before
                    native_hash(path / 'package.rpm', package_sha, group, max(1, min(30, deadline - time.monotonic())))
                    native_hash(path / 'pressure', fixture['pressure_sha'], group, max(1, min(30, deadline - time.monotonic())))
                    disk_idle(port, config)
                    after = native_snapshot(port, config, output, label + '-after')
                    row.update(after=after, byte_correct=True)
                    if delta(before, after, 'fuse_reads') <= 0:
                        raise RuntimeError('population round bypassed FUSE; kernel pagecache is not a warmup protocol')
                row.update(daemon_exit=daemon.returncode, mount_present_after=os.path.ismount(path))
                if time.monotonic() >= deadline:
                    raise RuntimeError('native cache population exceeded 180-second deadline')
                (output / 'native-reader-protocol.json').write_text(json.dumps(receipt, indent=2) + '\n')
            # Inventory is only a population condition, NOT proof of identity
            # or coverage. Never vary round count to obtain a favorable result.
            if after['index_entries'] < expected_records:
                raise RuntimeError('native population condition unmet after eight bounded warmup mounts; allowed drops are not a runtime correctness failure')
            label, port = 'reader-validate', integration.free_port()
            extra = mount_extra(config, 'candidate', cache, output / (label + '-stats.jsonl'), port, read_only=True)
            validation = {}
            receipt['validation'] = validation
            with integration.mount(binary, fixture['bucket'], fixture['prefix'], fixture['endpoint'], fixture['environment'],
                                   work, label, config['memory_limit_mb'], extra, group, validation) as (path, daemon):
                disk_ready(port, config)
                before = native_snapshot(port, config, output, label + '-before')
                native_hash(path / 'package.rpm', package_sha, group)
                after = native_snapshot(port, config, output, label + '-after')
                validation.update(before=before, after=after, byte_correct=True)
                verify_native_hot(before, after, len(package))
                receipt['warm_declared'] = True
                compare.workload(path, 'metadata', config, fixture['client'], fixture['bucket'], fixture['prefix'],
                                 fixture['endpoint'], True, fixture['expected_sha'])
                attrs = (path / 'package.rpm').stat()
                if attrs.st_mode & 0o777 != 0o640 or int(attrs.st_mtime) != 1700000000:
                    raise AssertionError('native RO metadata mode/mtime')
                with (path / 'package.rpm').open('rb', buffering=0) as stream:
                    if os.pread(stream.fileno(), 8, len(package) - 2) != package[-2:]:
                        raise AssertionError('native RO short EOF')
                nginx_range(path, package, work, group)
                copy = work / 'rsync-ro'
                copy.mkdir()
                integration.command(['rsync', '-a', '--checksum', str(path / 'package.rpm'), str(copy)], timeout=60)
                if (copy / 'package.rpm').read_bytes() != package:
                    raise AssertionError('rsync RO bytes')
            validation.update(daemon_exit=daemon.returncode, mount_present_after=os.path.ismount(path))
        return 'RO/direct-I/O: bounded observed population, full byte-correct recovered L2 with zero GET, nginx/rsync/EOF/metadata'
    except Exception as error:
        receipt['error'] = str(error)
        raise
    finally:
        (output / 'native-reader-protocol.json').write_text(json.dumps(receipt, indent=2) + '\n')


def native_checks(binary, fixture, work, config, group_root, output):
    """Real FUSE+localhost S3+nginx+rsync. Only owned mounts/data are touched."""
    from sealed_s3_fixture import blocks
    package = b''.join(blocks(7, MIB))
    s3, bucket, prefix = fixture['client'], fixture['bucket'], fixture['prefix']
    for name in ['package.rpm', 'excluded.txt']:
        s3.put_object(Bucket=bucket, Key=prefix + '/' + name, Body=package,
                      Metadata={'mode': str(0o100640), 'mtime': '1700000000'})
    cache = work / 'native-cache'
    cache.mkdir(mode=0o700)
    checks = [native_read_cache_phase(binary, fixture, package, work, config, group_root, output)]
    with compare.cgroup(group_root, config) as group:
        # Separate RW compatibility mount: never enable RO direct-I/O here and
        # never claim that buffered writer reads establish recovered L2 warmth.
        for cycle in range(1):
            port = integration.free_port()
            extra = mount_extra(config, 'candidate', cache, output / f'native-{cycle}-stats.jsonl', port)
            compatibility = dict(phase='RW compatibility, not L2-hot qualification', passed=False)
            with integration.mount(binary, bucket, prefix, fixture['endpoint'], fixture['environment'], work,
                                   f'native-{cycle}', config['memory_limit_mb'], extra, group, compatibility) as (path, daemon):
                disk_ready(port, config)
                compare.workload(path, 'metadata', config, s3, bucket, prefix,
                                 fixture['endpoint'], True, fixture['expected_sha'])
                attrs = (path / 'package.rpm').stat()
                if attrs.st_mode & 0o777 != 0o640 or int(attrs.st_mtime) != 1700000000:
                    raise AssertionError('native metadata-limit traversal and fixed mode/mtime')
                before = native_snapshot(port, config, output, f'native-{cycle}-before')
                compatibility['before'] = before
                (output / 'native-writer-protocol.json').write_text(json.dumps(compatibility, indent=2) + '\n')
                for name in ['package.rpm', 'excluded.txt']:
                    if integration.sha_file(path / name) != hashlib.sha256(package).hexdigest():
                        raise AssertionError('native independent package hash')
                with (path / 'package.rpm').open('rb', buffering=0) as stream:
                    if os.pread(stream.fileno(), 8, MIB - 2) != package[-2:]:
                        raise AssertionError('native short EOF')
                nginx_range(path, package, work, group)
                copy = work / f'rsync-{cycle}'
                copy.mkdir()
                integration.command(['rsync', '-a', '--checksum', str(path / 'package.rpm'), str(copy)], timeout=60)
                if (copy / 'package.rpm').read_bytes() != package:
                    raise AssertionError('rsync native read bytes')
                if cycle == 0:
                    source = work / 'rsync-writer.rpm'
                    source.write_bytes(package[:4096])
                    integration.command(['rsync', '-a', '--checksum', str(source), str(path / 'written.rpm')], timeout=60)
                    with (path / 'written.rpm').open('r+b', buffering=0) as stream:
                        os.fsync(stream.fileno())
                    if integration.remote_bytes(s3, bucket, prefix + '/written.rpm') != package[:4096]:
                        raise AssertionError('rsync fsync backend durability')
                    compatibility['rsync_fsync_remote_sha256'] = hashlib.sha256(package[:4096]).hexdigest()
                    compatibility['fsync_before'] = native_snapshot(port, config, output, 'writer-fsync-before')
                    integration.fault(fixture['endpoint'], 'put')
                    try:
                        with (path / 'fault').open('w+b', buffering=0) as stream:
                            stream.write(b'retained independent bytes')
                            try:
                                os.fsync(stream.fileno())
                            except OSError as error:
                                compatibility['injected_upload_fsync_error'] = dict(errno=error.errno, message=str(error))
                                (output / 'native-writer-protocol.json').write_text(json.dumps(compatibility, indent=2) + '\n')
                            else:
                                raise AssertionError('failed upload acknowledged fsync durability')
                            integration.fault(fixture['endpoint'])
                            integration.recover_fsync(stream.fileno())
                        if integration.remote_bytes(s3, bucket, prefix + '/fault') != b'retained independent bytes':
                            raise AssertionError('failed fsync recovery bytes')
                        compatibility['failed_fsync_recovery_byte_correct'] = True
                        compatibility['fsync_after_recovery'] = native_snapshot(port, config, output, 'writer-fsync-after')
                    finally:
                        integration.fault(fixture['endpoint'])
                checks.append(f'cycle-{cycle}: package/excluded/full/range/EOF/nginx/rsync/metadata-limit')
                # Force selected payload victims before graceful recovery cycle.
                if integration.sha_file(path / 'pressure') != fixture['pressure_sha']:
                    raise AssertionError('native pressure bytes')
                disk_idle(port, config)
                compatibility['after_pressure'] = native_snapshot(port, config, output, f'native-{cycle}-after-pressure')
            compatibility.update(passed=True, daemon_exit=daemon.returncode, mount_present_after=os.path.ismount(path))
            (output / 'native-writer-protocol.json').write_text(json.dumps(compatibility, indent=2) + '\n')
    checks.append(local_disk_failure(binary, fixture, work, config, group_root, output))
    return checks


def local_disk_failure(binary, fixture, work, config, group_root, output):
    """Exhaust only owned tmpfs BEFORE opening an empty cache: real fallocate failure.

    This verifies optional startup Bypass + authoritative-source reads, not an
    ongoing engine I/O failure against already preallocated media.
    """
    volume = work / 'fault-volume'
    volume.mkdir()
    capacity = config['disk_capacity_bytes']
    if config['cgroup_memory_max_mb'] * MIB < capacity + 4 * config['memory_limit_mb'] * MIB + 128 * MIB:
        raise ValueError('native tmpfs fault requires an explicit cgroup memory ceiling covering cache plus daemon/client headroom')
    integration.command(['mount', '-t', 'tmpfs', '-o', f'size={capacity + 8 * MIB},mode=0700',
                         'geesefs-disposable-l2-fault', volume], timeout=5)
    receipt = {'injection': 'owned-tmpfs-startup-preallocation-ENOSPC', 'capacity_bytes': capacity + 8 * MIB,
               'source_fallback_verified': False, 'ongoing_engine_io_failure_verified': False}
    try:
        cache = volume / 'cache'
        cache.mkdir(mode=0o700)
        with compare.cgroup(group_root, config) as group:
            receipt['cache_inventory_before'] = [p.name for p in cache.iterdir()]
            filler = integration.command([sys.executable, str(Path(__file__).resolve()), '--fill-owned-volume',
                str(volume / 'space-blocker'), '--fill-bytes', str(capacity + 8 * MIB)],
                capture_output=True, text=True, timeout=15,
                preexec_fn=lambda: integration.enter_cgroup(group))
            receipt['filler_enospc_observed'] = json.loads(filler.stdout)['enospc']
            receipt['available_bytes_before_startup'] = os.statvfs(volume).f_bavail * os.statvfs(volume).f_frsize
            if not receipt['filler_enospc_observed'] or receipt['cache_inventory_before']:
                raise RuntimeError('startup fault requires exhausted owned volume and empty private cache')
            port = integration.free_port()
            extra = mount_extra(config, 'candidate', cache, output / 'disk-fault-stats.jsonl', port, read_only=True)
            mount_receipt = {}
            receipt['mount'] = mount_receipt
            with integration.mount(binary, fixture['bucket'], fixture['prefix'], fixture['endpoint'], fixture['environment'],
                                   work, 'disk-fault', config['memory_limit_mb'], extra, group, mount_receipt) as (path, daemon):
                wait_disk_state(port, 'bypass')
                before = native_snapshot(port, config, output, 'disk-fault-before', require_adapter=False)
                native_hash(path / 'payload', fixture['expected_sha'], group)
                with (path / 'payload').open('rb', buffering=0) as stream:
                    for offset in [0, config['dataset_bytes'] // 2, config['dataset_bytes'] - 37]:
                        if os.pread(stream.fileno(), 37, offset) != integration.expected_range(offset, 37):
                            raise AssertionError('startup fallback range bytes')
                    if os.pread(stream.fileno(), 1, config['dataset_bytes']) != b'':
                        raise AssertionError('startup fallback EOF')
                after = native_snapshot(port, config, output, 'disk-fault-after', require_adapter=False)
                receipt.update(before=before, after=after)
                verify_startup_fallback(before, after, config['dataset_bytes'])
                error_log = (work / 'disk-fault.log').read_text()
                receipt['startup_error_log'] = error_log
                if 'disk cache unavailable; using source reads: filesystem error: 28' not in error_log:
                    raise RuntimeError('startup Bypass did not report actual ENOSPC preallocation failure')
                receipt['source_fallback_verified'] = True
            mount_receipt.update(daemon_exit=daemon.returncode, mount_present_after=os.path.ismount(path))
        return 'owned startup-preallocation ENOSPC: observed Bypass and exact full/range/EOF source bytes; ongoing I/O-error injection unverified'
    except Exception as error:
        receipt['error'] = str(error)
        raise
    finally:
        (output / 'native-disk-failure.json').write_text(json.dumps(receipt, indent=2) + '\n')
        integration.command(['umount', volume], timeout=10)


def nginx_range(path, expected, work, group):
    port = integration.free_port()
    directory = work / ('nginx-' + str(port))
    directory.mkdir()
    config = directory / 'nginx.conf'
    # Keep the server in the same test identity so the private 0700 parent is
    # accessible. This does not enable a public listener or shared mount access.
    user = f'user {pwd.getpwuid(os.getuid()).pw_name}; ' if os.geteuid() == 0 else ''
    config.write_text(f'{user}daemon off; master_process off; pid {directory}/pid; error_log {directory}/error.log; lock_file {directory}/lock;\n'
                      f'events {{ worker_connections 64; }} http {{ access_log off; sendfile on; '
                      f'client_body_temp_path {directory}/body; proxy_temp_path {directory}/proxy; '
                      f'fastcgi_temp_path {directory}/fastcgi; uwsgi_temp_path {directory}/uwsgi; scgi_temp_path {directory}/scgi; '
                      f'server {{ listen 127.0.0.1:{port}; '
                      f'root "{path}"; }} }}\n')
    with (directory / 'stdout.log').open('wb') as log:
        process = subprocess.Popen(['nginx', '-p', str(directory), '-c', str(config)], stdout=log, stderr=log,
                                   preexec_fn=lambda: integration.enter_cgroup(group))
        try:
            deadline = time.monotonic() + 10
            while True:
                try:
                    with urllib.request.urlopen(f'http://127.0.0.1:{port}/package.rpm', timeout=3) as response:
                        if response.read(len(expected) + 1) != expected:
                            raise AssertionError('nginx full bytes')
                    break
                except OSError:
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError('owned nginx did not become ready') from None
                    time.sleep(0.05)
            request = urllib.request.Request(f'http://127.0.0.1:{port}/package.rpm', headers={'Range': 'bytes=3-259'})
            with urllib.request.urlopen(request, timeout=3) as response:
                if response.status != 206 or response.read(258) != expected[3:260]:
                    raise AssertionError('nginx Range status/bytes')
            request = urllib.request.Request(f'http://127.0.0.1:{port}/package.rpm',
                                             headers={'Range': f'bytes={len(expected)}-'})
            try:
                urllib.request.urlopen(request, timeout=3).close()
            except urllib.error.HTTPError as error:
                if error.code != 416 or error.headers.get('Content-Range') != f'bytes */{len(expected)}':
                    raise AssertionError('nginx out-of-range EOF response') from error
                error.close()
            else:
                raise AssertionError('nginx accepted Range beyond EOF')
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


@contextlib.contextmanager
def evidence_work(output):
    with tempfile.TemporaryDirectory(prefix='geesefs-l2-native-', dir=output) as temporary:
        work = Path(temporary)
        try:
            yield work
        finally:
            for log in work.glob('*.log'):
                shutil.copyfile(log, output / log.name)
            for directory in work.glob('nginx-*'):
                shutil.copytree(directory, output / directory.name)


def assert_owned_cleanup(group_root, work):
    groups_remain = any(p.is_dir() and p.name.startswith('geesefs-bench-') for p in Path(group_root).iterdir())
    mounts_remain = any(p.name.endswith('-mount') and os.path.ismount(p) for p in work.iterdir())
    if groups_remain or mounts_remain:
        raise RuntimeError('owned benchmark resource cleanup incomplete')


def run_native_preflight(args, report, binary, fixture, work, config, group_root):
    try:
        report['native_checks'] = native_checks(binary, fixture, work, config, group_root, args.output)
        report['native_passed'] = True
    except compare.BenchmarkDeadline:
        raise
    except Exception as error:
        report['native_passed'] = False
        report['errors'].append('native preflight failed: ' + str(error))
        assert_owned_cleanup(group_root, work)
        if args.native_only:
            raise
    finally:
        (args.output / 'native-preflight.json').write_text(json.dumps(dict(
            passed=report['native_passed'], checks=report.get('native_checks', []), errors=report['errors']), indent=2) + '\n')


def run_matrix_cases(args, config, report, binaries, fixtures, work, group_root):
    """Retain every failed case and finish independent cases, never select wins.

    Source changes, aggregate deadlines or failed owned-resource cleanup remain
    fatal. An unmet warm-state gate is not permission to discard the whole arm.
    """
    for record in cases(config, args.pairs, args.controls):
        current = dataset_config(config, record['state'], record['workload'])
        expected_arm = 'baseline' if record['arm'] == 'control' else record['arm']
        if integration.sha_file(binaries[record['arm']]) != report['provenance'][expected_arm]['binary_sha256']:
            raise RuntimeError('frozen executable changed during the matrix')
        try:
            measured_run(binaries[record['arm']], fixtures[current['dataset_bytes']], work, record, current, group_root, args.output)
        except compare.BenchmarkDeadline as error:
            record.update(correctness=False, error=str(error))
            raise
        except Exception as error:
            record.update(correctness=False, error=str(error))
        finally:
            try:
                # Continue only after existing mount/worker/cgroup owners have
                # completed cleanup. Never retain every case's preallocated
                # cache until the matrix ends (in particular on tmpfs).
                assert_owned_cleanup(group_root, work)
                cache = work / (record['mount_id'] + '-cache')
                if cache.exists():
                    if 'disk_allocated_bytes' not in record:
                        record['disk_allocated_bytes'] = sum(p.stat().st_blocks * 512 for p in cache.rglob('*') if p.is_file())
                        record['disk_allocated_bytes_scope'] = 'owned cache after failed case and clean shutdown, before untimed removal'
                    shutil.rmtree(cache)
                    record['owned_cache_removed'] = True
            finally:
                report['runs'].append(record)
                (args.output / 'runs.json').write_text(json.dumps(report['runs'], indent=2) + '\n')


def execute(args, config, report):
    group_root = compare.cgroup_preflight(args.cgroup_root)
    if integration.command(['git', 'status', '--porcelain', '--', 'src', 'Cargo.toml', 'Cargo.lock'],
                           cwd=ROOT, capture_output=True, text=True, timeout=5).stdout:
        raise RuntimeError('runtime source writes pending; require clean frozen source checkpoint before execution')
    binaries = {}
    report['provenance'] = {}
    for arm, path in [('baseline', args.baseline), ('candidate', args.candidate)]:
        if path is None:
            raise RuntimeError('explicit --baseline and --candidate executables are required')
        receipt, help_text = provenance(path, config.get('provenance', {}).get(arm, {}))
        required = ['--metadata-limit', '--metrics-listen', '--stats-file', '--read-only-direct-io']
        if arm == 'candidate':
            required.append('--disk-cache-dir')
        if any(flag not in help_text for flag in required):
            raise RuntimeError(f'{arm} Rust CLI/runtime capability pending')
        report['provenance'][arm] = receipt
        binaries[arm] = path.resolve()
    binaries['control'] = binaries['baseline']
    revision = integration.command(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                                   capture_output=True, text=True, timeout=5).stdout.strip()
    if report['provenance']['candidate']['source_revision'] != revision:
        raise RuntimeError('candidate frozen receipt does not match the clean runtime source checkpoint')
    report['control_executable_sha256'] = report['provenance']['baseline']['binary_sha256']
    report['accounting_scope'] = 'daemon + fresh workload client in one owned cgroup; localhost S3 fixture is outside, not whole-machine cost'
    report['state_protocols'] = dict(
        both_cold='new mount and empty app caches; S3 fixture/device media not cold',
        l1_hot='small verified working set primed inside same mount; FUSE direct-I/O; observed L1 hits/no remote bytes',
        l2_hot_pagecache_hot='target demoted using excluded pressure, clean shutdown, owned cache file read-through, new mount',
        l2_hot_pagecache_cold_requested='same persisted target, owned-file DONTNEED before reopen; advisory, not verified cold')
    # Always local, uniquely owned; never consumes GEESEFS_TEST_ENDPOINT.
    with evidence_work(args.output) as work:
        with integration.service(None, work) as (client, bucket, endpoint, environment, _):
            report['s3_fixture'] = 'owned localhost Moto HTTP fixture; not provider interoperability acceptance'
            fixtures = {}
            for size in [config['l1_hot_bytes'], config['dataset_bytes'], config['eviction_bytes']]:
                prefix = f'dataset-{size}'
                sha = integration.seed(client, bucket, prefix, size, config['metadata_entries'])
                pressure_sha = integration.seed(client, bucket, prefix + '-pressure', 2 * config['memory_limit_mb'] * MIB, 1)
                client.copy_object(Bucket=bucket, Key=prefix + '/pressure', CopySource={'Bucket': bucket, 'Key': prefix + '-pressure/payload'})
                head = client.head_object(Bucket=bucket, Key=prefix + '/payload')
                fixtures[size] = dict(client=client, bucket=bucket, prefix=prefix, endpoint=endpoint,
                                      environment=environment, expected_sha=sha, pressure_sha=pressure_sha,
                                      etag=head['ETag'], size=head['ContentLength'])
            report['fixture_generations'] = {size: {key: value[key] for key in ['prefix', 'expected_sha', 'pressure_sha', 'etag', 'size']}
                                             for size, value in fixtures.items()}
            run_native_preflight(args, report, binaries['candidate'], fixtures[config['dataset_bytes']], work, config, group_root)
            if not args.native_only:
                run_matrix_cases(args, config, report, binaries, fixtures, work, group_root)
            if not args.native_only:
                report.update(summarize(report['runs'], config, args.pairs, args.controls))
            if not report['native_passed']:
                report.update(qualified=False, full_matrix_qualified=False)
            if integration.command(['git', 'status', '--porcelain', '--', 'src', 'Cargo.toml', 'Cargo.lock'],
                                   cwd=ROOT, capture_output=True, text=True, timeout=5).stdout:
                report['native_passed'] = False
                raise RuntimeError('runtime source changed during the frozen matrix')
            final_revision = integration.command(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                capture_output=True, text=True, timeout=5).stdout.strip()
            if final_revision != revision or any(integration.sha_file(ROOT / name) != digest
                    for name, digest in report['harness_sha256'].items()):
                report['native_passed'] = False
                raise RuntimeError('source checkpoint or harness changed during the matrix')


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument('--baseline', type=Path)
    result.add_argument('--candidate', type=Path)
    result.add_argument('--manifest', type=Path)
    result.add_argument('--output', type=Path)
    result.add_argument('--pairs', type=int, default=8)
    result.add_argument('--controls', type=int, default=8)
    result.add_argument('--deadline', type=int, default=7200)
    result.add_argument('--cgroup-root', default=os.environ.get('GEESEFS_BENCH_CGROUP_ROOT'))
    result.add_argument('--execute', action='store_true')
    result.add_argument('--preflight', action='store_true')
    result.add_argument('--native-only', action='store_true')
    result.add_argument('--example-manifest', action='store_true')
    result.add_argument('--worker', action='store_true', help=argparse.SUPPRESS)
    result.add_argument('--namespace-owned', action='store_true', help=argparse.SUPPRESS)
    result.add_argument('--fill-owned-volume', type=Path, help=argparse.SUPPRESS)
    result.add_argument('--fill-bytes', type=int, help=argparse.SUPPRESS)
    return result


def main():
    args = parser().parse_args()
    if args.fill_owned_volume:
        import errno
        enospc = False
        if not args.fill_bytes or not 0 < args.fill_bytes <= 3 * 1024 ** 3:
            raise ValueError('bounded owned tmpfs filler size required')
        with args.fill_owned_volume.open('xb', buffering=0) as stream:
            try:
                for _ in range(args.fill_bytes // MIB + 1):
                    stream.write(b'\x00' * MIB)
            except OSError as error:
                if error.errno != errno.ENOSPC:
                    raise
                enospc = True
        print(json.dumps({'enospc': enospc}))
        return 0
    if args.worker:
        return worker_main()
    if args.example_manifest:
        print(json.dumps(example_manifest(), indent=2))
        return 0
    if not args.output or args.deadline <= 0:
        raise SystemExit('--output and a positive --deadline are required')
    if args.execute and not args.namespace_owned:
        # The wrapper owns the new namespace. Never mount/remount a scratch
        # filesystem in the caller's namespace, even when root is available.
        child = subprocess.Popen(['unshare', '--mount', '--propagation', 'private', sys.executable,
                                  str(Path(__file__).resolve()), *sys.argv[1:], '--namespace-owned'],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        try:
            stdout, stderr = child.communicate(timeout=args.deadline + 90)
        except subprocess.TimeoutExpired:
            # Only this invocation's newly owned process group, never host
            # processes or shared mounts. Allow bounded cleanup before SIGKILL.
            os.killpg(child.pid, signal.SIGTERM)
            try:
                stdout, stderr = child.communicate(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                stdout, stderr = child.communicate(timeout=5)
            stderr += '\nprivate native process-group deadline exceeded\n'
        if not (args.output / 'report.json').exists():
            prepare_output(args.output)
            (args.output / 'report.json').write_text(json.dumps(dict(qualified=False, production_accepted=False,
                native_passed=False, runs=[], errors=['private namespace launcher failed: ' + stderr]), indent=2) + '\n')
        sys.stdout.write(stdout)
        sys.stderr.write(stderr)
        return child.returncode
    prepare_output(args.output)
    args.output = args.output.resolve()
    report = dict(qualified=False, production_accepted=False, native_passed=False, runs=[], errors=[])
    report['harness_sha256'] = {name: integration.sha_file(ROOT / name) for name in [
        'bench/disk_cache.py', 'bench/accounting.py', 'bench/compare.py', 'bench/paired.py',
        'bench/sealed_s3_fixture.py', 'scripts/integration.py']}
    def expired(_signum, _frame):
        raise compare.BenchmarkDeadline('local disk-cache aggregate deadline exceeded')
    signal.signal(signal.SIGALRM, expired)
    signal.alarm(args.deadline)
    try:
        report['capabilities'] = capabilities(args.cgroup_root)
        report['errors'].extend(report['capabilities']['errors'])
        report['command'] = sys.argv[:]
        if args.manifest:
            report['manifest_sha256'] = integration.sha_file(args.manifest)
        if args.execute:
            if args.preflight:
                raise ValueError('--execute and --preflight are mutually exclusive')
            if not args.manifest:
                raise ValueError('explicit workload/provenance --manifest required')
            config = json.loads(args.manifest.read_text())
            validate(config, args.pairs, args.controls)
            report['manifest'] = config
            if report['errors']:
                raise RuntimeError('native capability preflight failed; matrix not started')
            execute(args, config, report)
    except Exception as error:
        report['errors'].append(str(error))
    finally:
        signal.alarm(0)
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    if args.execute:
        return 0 if report['native_passed'] and (args.native_only or report['qualified']) else 1
    return 0 if not report['errors'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
