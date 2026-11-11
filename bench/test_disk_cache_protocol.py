"""Public orchestration, equal-work and measurement-boundary regressions."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import disk_cache as disk


class Contracts(unittest.TestCase):
    def test_default_operation_plan_is_fixed_before_any_arm_runs(self):
        config = disk.example_manifest()
        for name, state, cycles, transferred, operations in [
                ('sequential', 'l2_hot_pagecache_hot', 8, 1073741824, 1024),
                ('sequential', 'l1_hot', 64, 67108864, 64),
                ('concurrent', 'l2_hot_pagecache_cold_requested', 1, 134217728, 1024),
                ('concurrent', 'l2_hot_pagecache_hot', 8, 1073741824, 8192),
                ('random', 'l2_hot_pagecache_hot', 8, 268435456, 2048),
                ('metadata', 'normal', 8, 0, 16384),
                ('eviction', 'normal', 1, 536870912, 512),
                ('writer_fsync', 'normal', 1, 8388608, 8)]:
            with self.subTest(name=name, state=state):
                plan = disk.operation_plan(disk.dataset_config(config, state, name), name, state)
                self.assertEqual((plan['cycles'], plan['bytes'], plan['operations']), (cycles, transferred, operations))
        legacy = dict(config, minimum_active_seconds=1)
        with self.assertRaisesRegex(ValueError, 'time-driven work'):
            disk.validate(legacy, 8, 8)

    def test_eviction_workload_io_excludes_the_untimed_fallback_probe(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / 'out'
            output.mkdir()
            for name, value in [('cpu.max', '200000 100000'), ('memory.max', '2147483648')]:
                (root / name).write_text(value)
            config = disk.dataset_config(disk.example_manifest(), 'normal', 'eviction')
            fixture = dict(bucket='fixture', prefix='data', endpoint='http://127.0.0.1:9', environment={}, expected_sha='unused')
            record = dict(mount_id='io-scope', workload='eviction', state='normal', arm='candidate')
            counter = {'value': 0}
            seed_before = dict(l2_hit_bytes=0, source_get_attempts=0, remote_bytes=0, fuse_reads=0)
            seed_after = dict(seed_before, l2_hit_bytes=5242880, fuse_reads=40, region_reclaims_started=0)
            prepared = dict(seed_after, region_reclaims_started=18, l2_misses=0)
            measured = dict(prepared, region_reclaims_started=23, fuse_reads=552)
            probe = dict(measured, l2_misses=40, remote_bytes=5242880)
            @contextlib.contextmanager
            def mount(*_):
                yield root, SimpleNamespace(pid=os.getpid())
            @contextlib.contextmanager
            def worker(*_):
                yield SimpleNamespace(pid=os.getpid(), stdin=io.StringIO())
            def measured_work(*_):
                counter['value'] = 100
                return dict(metrics=dict(bytes=536870912, operations=512, elapsed_seconds=1.0,
                    cycles=1, throughput=536870912, latencies_ms=[2.0] * 512,
                    operation_plan=disk.operation_plan(config, 'eviction', 'normal')))
            def correctness_probe(*_):
                if counter['value']:
                    counter['value'] = 900
            @contextlib.contextmanager
            def sampler(*_):
                yield SimpleNamespace(metrics=lambda: {}, cpu=SimpleNamespace(before={}, after={}))
            def prime(_binary, _fixture, _work, _record, _config, cache, *_):
                (cache / 'payload-v1.cache').write_bytes(b'external-cache-fixture')
            with patch.object(disk.compare, 'cgroup', side_effect=lambda *_: contextlib.nullcontext(root)), \
                 patch.object(disk.integration, 'mount', mount), patch.object(disk, 'prime', prime), \
                 patch.object(disk, 'disk_started'), patch.object(disk, 'worker', worker), \
                 patch.object(disk.compare, 'Sampler', sampler), \
                 patch.object(disk.compare, 'read_message', side_effect=measured_work), \
                 patch.object(disk, 'eviction_probe', side_effect=correctness_probe), \
                 patch.object(disk, 'disk_idle', side_effect=lambda *_: probe if counter['value'] == 900 else measured), \
                 patch.object(disk, 'native_snapshot', side_effect=[seed_before, seed_after, prepared, prepared, measured]), \
                 patch.object(disk.capacity_population, 'populate', return_value=dict(byte_correct=True)), \
                 patch.object(disk, 'io_counters', side_effect=lambda _: dict(read_bytes=counter['value'],
                      write_bytes=2 * counter['value'], syscr=counter['value'], syscw=counter['value'])), \
                 patch.object(disk, 'allocator_snapshot', return_value=(0, 0, {})):
                disk.measured_run('/external-daemon', fixture, root, record, config, root, output)
            self.assertEqual(record['daemon_io_delta']['read_bytes'], 100)
            self.assertEqual(record['daemon_io_delta']['write_bytes'], 200)
            self.assertEqual(record['eviction_probe_io_delta']['read_bytes'], 800)
            self.assertEqual(record['eviction_probe_io_delta']['write_bytes'], 1600)
            self.assertEqual(record['write_amplification_vs_consumed_payload'], 200 / 536870912)

    def test_late_real_source_receipt_rejection_invalidates_success_and_retains_results(self):
        for native_only in [False, True]:
            with self.subTest(native_only=native_only), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                manifest = root / 'manifest.json'
                manifest.write_text(json.dumps(disk.example_manifest()))
                args = disk.parser().parse_args(['--execute', '--namespace-owned', '--manifest', str(manifest),
                    '--output', str(root / 'out')] + (['--native-only'] if native_only else []))
                def aggregate_then_reject_receipt(_args, _config, report):
                    report.update(native_passed=True, qualified=True, observed_state_screen_passed=True,
                                  full_matrix_qualified=True, runs=[dict(mount_id='completed-byte-checked-case')])
                    inputs = {str(path.relative_to(disk.ROOT)): disk.integration.sha_file(path)
                              for path in (disk.ROOT / 'src').rglob('*.rs')}
                    inputs.update({name: disk.integration.sha_file(disk.ROOT / name)
                                   for name in ['Cargo.toml', 'Cargo.lock']})
                    inputs['Cargo.toml'] = '0' * 64
                    disk.runtime_sources(dict(runtime_source_sha256=inputs))
                with patch.object(disk, 'parser', return_value=SimpleNamespace(parse_args=lambda: args)), \
                     patch.object(disk, 'capabilities', return_value=dict(errors=[])), \
                     patch.object(disk, 'execute', side_effect=aggregate_then_reject_receipt), \
                     patch.object(disk.signal, 'signal'), patch.object(disk.signal, 'alarm'), \
                     contextlib.redirect_stdout(io.StringIO()):
                    code = disk.main()
                saved = json.loads((root / 'out/report.json').read_text())
                self.assertEqual(code, 1)
                for key in ['qualified', 'native_passed', 'full_matrix_qualified', 'observed_state_screen_passed']:
                    self.assertFalse(saved[key], key)
                self.assertEqual(saved['runs'], [dict(mount_id='completed-byte-checked-case')])
                self.assertIn('runtime source hashes differ', saved['errors'][0])

    def test_worker_executes_same_declared_work_despite_different_arm_clocks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = b'independently expected checked bytes\n'
            (root / 'payload').write_bytes(payload)
            config = dict(disk.example_manifest(), dataset_bytes=len(payload), measurement_cycles=2)
            spec = dict(path=str(root), endpoint='http://127.0.0.1:9', bucket='fixture', prefix='data',
                        expected_sha=hashlib.sha256(payload).hexdigest(), name='sequential',
                        state='l2_hot_pagecache_hot', config=config)
            observations = []
            for times in [[0, 1.1, 1.2, 1.3], [0, 0.6, 1.1, 1.2]]:
                ticks, output = iter(times), io.StringIO()
                with patch.object(disk, 'time', SimpleNamespace(perf_counter=lambda: next(ticks))), \
                     patch.object(sys, 'stdin', io.StringIO(json.dumps(spec) + '\nstart\ndone\n')), \
                     patch.dict(disk.os.environ, AWS_ACCESS_KEY_ID='public-test', AWS_SECRET_ACCESS_KEY='public-test'), \
                     contextlib.redirect_stdout(output):
                    self.assertEqual(disk.worker_main(), 0, output.getvalue())
                metrics = json.loads(output.getvalue().splitlines()[1])['metrics']
                observations.append({key: metrics[key] for key in ['cycles', 'bytes', 'operations', 'independent_mounts']})
            self.assertEqual(observations, [dict(cycles=2, bytes=74, operations=2, independent_mounts=1)] * 2)


if __name__ == '__main__':
    unittest.main()
