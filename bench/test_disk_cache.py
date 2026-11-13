"""Contract tests for the mounted L2 harness; no FUSE or external service needed."""
import copy
import contextlib
import io
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch
from types import SimpleNamespace

import disk_cache as disk


class Contracts(unittest.TestCase):
    def test_lazy_prepared_is_absent_adapter_not_fake_ready_or_zero_gauges(self):
        mapping = {key: disk.example_manifest()['disk_metrics'][key] for key in
                   ['state_prepared', 'stats_available', 'io_read_bytes', 'active_read_jobs', 'opened_file_available']}
        text = 'geesefs_disk_cache_state{state="prepared"} 1\ngeesefs_disk_cache_stats_available 0\n'
        actual = disk.parse_metrics(text, mapping)
        self.assertIsNone(actual['io_read_bytes'])
        self.assertIsNone(actual['active_read_jobs'])
        self.assertIsNone(actual['opened_file_available'])
        self.assertEqual(disk.delta(actual, actual, 'io_read_bytes'), 0)
        for changed in [text.replace('prepared"} 1', 'prepared"} 0'), text.replace('available 0', 'available 1')]:
            with self.assertRaisesRegex(RuntimeError, 'missing/invalid'):
                disk.parse_metrics(changed, mapping)

    def test_lazy_started_never_satisfies_existing_inventory_ready_gate(self):
        config = disk.example_manifest()
        lazy = dict(state_prepared=1, state_ready=0, stats_available=0)
        with patch.object(disk, 'sample_endpoint', return_value=lazy):
            self.assertEqual(disk.disk_started(1, config), lazy)
        with patch.object(disk, 'disk_ready', side_effect=RuntimeError('existing inventory not Ready')):
            with self.assertRaisesRegex(RuntimeError, 'not Ready'):
                disk.disk_started(1, config, require_ready=True)

    def test_preparation_and_reason_lane_receipts_are_explicit(self):
        config = disk.example_manifest()
        disk.validate(config, 8, 8)
        self.assertEqual(config['eviction_preparation']['rounds'], 1)
        self.assertEqual(config['eviction_preparation']['size'], 536870912)
        self.assertEqual(config['disk_metrics']['active_read_jobs'], 'geesefs_disk_cache_active_read_jobs')
        self.assertEqual(config['disk_metrics']['queued_reads'], 'geesefs_disk_cache_queued_reads')
        self.assertEqual(config['disk_metrics']['reason_jobs'], 'geesefs_disk_cache_read_fallbacks_total{reason="jobs"}')
        config['eviction_preparation']['rounds'] = 2
        with self.assertRaisesRegex(ValueError, 'fixed, matched'):
            disk.validate(config, 8, 8)

    def test_debug_option_cannot_enter_qualification_comparisons(self):
        config = dict(disk.example_manifest(), diagnostic_debug=True)
        self.assertIn('--diagnostic-log-level', disk.mount_extra(config, 'candidate', Path('/cache'), Path('/stats'), 123))
        with self.assertRaisesRegex(ValueError, 'diagnosis-only'):
            disk.validate(config, 8, 8)

    def test_explicit_dirty_build_receipt_checks_every_source_and_rejects_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'src').mkdir()
            for name in ['src/lib.rs', 'Cargo.toml', 'Cargo.lock']:
                (root / name).write_bytes(b'fixture source\n')
            expected = {name: disk.hashlib.sha256(b'fixture source\n').hexdigest()
                        for name in ['src/lib.rs', 'Cargo.toml', 'Cargo.lock']}
            with patch.object(disk, 'ROOT', root):
                self.assertEqual(disk.runtime_sources(dict(runtime_source_sha256=expected)), expected)
                with self.assertRaisesRegex(RuntimeError, 'cover all'):
                    disk.runtime_sources(dict(runtime_source_sha256={'src/lib.rs': expected['src/lib.rs']}))
                (root / 'src/lib.rs').write_text('changed source\n')
                with self.assertRaisesRegex(RuntimeError, 'differ'):
                    disk.runtime_sources(dict(runtime_source_sha256=expected))

    def test_moto_cannot_be_selected_for_performance(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(ValueError, 'Moto is functional/fault-only'):
                with disk.performance_service(None, Path(temporary), disk.example_manifest(), [], True):
                    self.fail('unverified performance provider admitted')
        args = SimpleNamespace(native_only=False, fixture_weed=None)
        with self.assertRaisesRegex(ValueError, 'explicit --fixture-weed'):
            disk.execute(args, disk.example_manifest(), {})

    def test_native_provider_flag_is_explicit(self):
        args = disk.parser().parse_args(['--fixture-weed', '/private/verified/weed'])
        self.assertEqual(args.fixture_weed, Path('/private/verified/weed'))

    def test_allocator_snapshot_ignores_only_unfinished_trailing_json_frame(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'stats.jsonl'
            complete = json.dumps(dict(unix_time_ns=150, allocated_bytes=41,
                effective_budgets=dict(payload_limit_bytes=64, read_cache_policy_bytes=57))) + '\n'
            source = complete + '{"unix_time_ns":160,"allocated_bytes"'
            path.write_text(source)
            peak, steady, budgets = disk.allocator_snapshot(path, 100, 200)
            self.assertEqual((peak, steady), (41, 41))
            self.assertEqual(budgets, dict(payload_limit_bytes=64, read_cache_policy_bytes=57))
            self.assertEqual(path.read_text(), source)
            path.write_text(complete + '{malformed complete frame}\n')
            with self.assertRaises(json.JSONDecodeError):
                disk.allocator_snapshot(path, 100, 200)

    def test_native_state_failure_is_retained_without_blocking_independent_matrix(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = SimpleNamespace(native_only=False, output=root)
            report = dict(native_passed=False, errors=[])
            with patch.object(disk, 'native_checks', side_effect=RuntimeError('seed coverage incomplete')):
                disk.run_native_preflight(args, report, '/unused', {}, root, disk.example_manifest(), root)
            self.assertFalse(report['native_passed'])
            self.assertEqual(report['errors'], ['native preflight failed: seed coverage incomplete'])
            self.assertEqual(json.loads((root / 'native-preflight.json').read_text())['passed'], False)
            (root / 'geesefs-bench-owned').mkdir()
            with patch.object(disk, 'native_checks', side_effect=RuntimeError('cleanup failed')):
                with self.assertRaisesRegex(RuntimeError, 'owned benchmark resource cleanup incomplete'):
                    disk.run_native_preflight(args, report, '/unused', {}, root, disk.example_manifest(), root)

    def test_matrix_retains_failed_case_and_executes_later_independent_arms(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / 'binary'
            binary.write_bytes(b'frozen-artifact')
            records = [dict(workload='sequential', state='both_cold', pair=0, arm=arm, mount_id=arm)
                       for arm in ['baseline', 'candidate', 'control']]
            report = dict(runs=[], provenance={arm: dict(binary_sha256=disk.hashlib.sha256(b'frozen-artifact').hexdigest())
                                               for arm in ['baseline', 'candidate']})
            def measured(*args):
                record = args[3]
                cache = root / (record['mount_id'] + '-cache')
                cache.mkdir()
                (cache / 'allocated-record').write_bytes(b'cache evidence')
                if record['arm'] == 'baseline':
                    raise RuntimeError('owned fixture dependency failed')
                record.update(correctness=True, independent_mount=True, metrics={'throughput': 7})
            args = SimpleNamespace(pairs=8, controls=8, output=root)
            with patch.object(disk, 'cases', return_value=iter(records)), patch.object(disk, 'measured_run', measured):
                disk.run_matrix_cases(args, disk.example_manifest(), report, {arm: binary for arm in ['baseline', 'candidate', 'control']},
                                      {128 * disk.MIB: {}}, root, root)
            saved = json.loads((root / 'runs.json').read_text())
            self.assertEqual(len(saved), 3)
            self.assertFalse(saved[0]['correctness'])
            self.assertEqual(saved[0]['error'], 'owned fixture dependency failed')
            self.assertTrue(saved[1]['correctness'])
            self.assertTrue(saved[2]['correctness'])
            self.assertEqual(saved[2]['metrics']['throughput'], 7)
            self.assertTrue(all(row['owned_cache_removed'] for row in saved))
            self.assertFalse(any(root.glob('*-cache')))

    def test_matrix_stops_on_deadline_artifact_change_or_incomplete_cleanup(self):
        for failure in ['deadline', 'artifact', 'cleanup', 'mount-cleanup']:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                binary = root / 'binary'
                binary.write_bytes(b'frozen-artifact')
                records = [dict(workload='sequential', state='both_cold', pair=0, arm=arm, mount_id=arm)
                           for arm in ['baseline', 'candidate']]
                report = dict(runs=[], provenance={arm: dict(binary_sha256=disk.hashlib.sha256(b'frozen-artifact').hexdigest())
                                                   for arm in ['baseline', 'candidate']})
                if failure == 'artifact':
                    binary.write_bytes(b'changed-artifact')
                def measured(*args):
                    if failure == 'deadline':
                        raise disk.compare.BenchmarkDeadline('aggregate expired')
                    (root / ('owned-mount' if failure == 'mount-cleanup' else 'geesefs-bench-owned')).mkdir()
                    raise RuntimeError('cleanup failed')
                args = SimpleNamespace(pairs=8, controls=8, output=root)
                expected = {'deadline': 'aggregate expired', 'artifact': 'frozen executable changed',
                            'cleanup': 'owned benchmark resource cleanup incomplete',
                            'mount-cleanup': 'owned benchmark resource cleanup incomplete'}[failure]
                with patch.object(disk, 'cases', return_value=iter(records)), patch.object(disk, 'measured_run', side_effect=measured) as run, \
                     patch.object(disk.os.path, 'ismount', return_value=failure == 'mount-cleanup'):
                    with self.assertRaisesRegex((RuntimeError, disk.compare.BenchmarkDeadline), expected):
                        disk.run_matrix_cases(args, disk.example_manifest(), report, {'baseline': binary, 'candidate': binary},
                                              {128 * disk.MIB: {}}, root, root)
                self.assertEqual(run.call_count, 0 if failure == 'artifact' else 1)
                if failure in ['deadline', 'cleanup', 'mount-cleanup']:
                    saved = json.loads((root / 'runs.json').read_text())
                    self.assertEqual(len(saved), 1)
                    self.assertFalse(saved[0]['correctness'])
                    self.assertEqual(saved[0]['error'], 'aggregate expired' if failure == 'deadline' else 'cleanup failed')

    def test_population_has_eight_identical_io_rounds_per_role_not_extra_measured_pairs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'payload').write_bytes(b'payload')
            (root / 'pressure').write_bytes(b'pressure')
            fixture = dict(bucket='fixture', prefix='data', endpoint='http://127.0.0.1:9', environment={},
                           expected_sha=disk.hashlib.sha256(b'payload').hexdigest(),
                           pressure_sha=disk.hashlib.sha256(b'pressure').hexdigest())
            mounts = []
            @contextlib.contextmanager
            def mount(*args):
                mounts.append(args[8])  # Public mount helper's explicit CLI extras.
                yield root, SimpleNamespace(returncode=0)
            for role in ['baseline', 'candidate', 'control']:
                record = dict(mount_id=role, arm=role, workload='sequential')
                cache = root / (role + '-cache')
                cache.mkdir()
                config = disk.example_manifest()
                config['dataset_bytes'] = 7
                with patch.object(disk.integration, 'mount', mount), patch.object(disk, 'disk_ready'), patch.object(disk, 'disk_started'), \
                     patch.object(disk, 'disk_idle'), patch.object(disk, 'native_snapshot',
                         side_effect=[dict(fuse_reads=n, l2_hit_bytes=7 * n, remote_bytes=0, source_get_attempts=0)
                                      for _ in range(9) for n in [0, 1]]):
                    disk.prime('/unused', fixture, root, record, config, cache, None, root)
                self.assertEqual(len(record['population_rounds']), 8)
                self.assertTrue(all(not row['independent_measured_mount'] for row in record['population_rounds']))
                self.assertFalse(record['population_validation']['independent_measured_mount'])
                for extra in mounts[-9:]:
                    self.assertIn('--read-only-direct-io', extra)
                    self.assertEqual(extra[extra.index('-o') + 1], 'ro')
                    self.assertEqual('--disk-cache-dir' in extra, role == 'candidate')
                for extra in mounts[-9:-1]:
                    for flag in ['--read-ahead', '--read-ahead-small', '--read-ahead-large', '--read-ahead-parallel']:
                        self.assertIn(flag, extra)
                        self.assertEqual(extra[extra.index(flag) + 1], '128')
                self.assertNotIn('--read-ahead', mounts[-1])
            self.assertEqual(len(mounts), 27)
            writer = disk.mount_extra(disk.example_manifest(), 'candidate', root, root / 'writer-stats', 123)
            self.assertNotIn('--read-only-direct-io', writer)

    def test_eviction_population_covers_entire_five_mib_pipeline_window(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'pressure').write_bytes(b'pressure')
            fixture = dict(bucket='fixture', prefix='data', endpoint='http://127.0.0.1:9', environment={},
                           pressure_sha=disk.hashlib.sha256(b'pressure').hexdigest())
            @contextlib.contextmanager
            def mount(*args):
                yield root, SimpleNamespace(returncode=0)
            record = dict(mount_id='eviction', arm='candidate', workload='eviction')
            with patch.object(disk.integration, 'mount', mount), patch.object(disk, 'disk_ready'), patch.object(disk, 'disk_started'), \
                 patch.object(disk, 'disk_idle'), patch.object(disk, 'eviction_probe') as probe, \
                 patch.object(disk, 'native_snapshot', side_effect=[
                     dict(fuse_reads=n, l2_hit_bytes=5 * disk.MIB * n, remote_bytes=0, source_get_attempts=0)
                     for _ in range(9) for n in [0, 1]]):
                disk.prime('/unused', fixture, root, record, disk.example_manifest(), root, None, root)
            self.assertEqual(probe.call_count, 9)
            self.assertEqual([call.args[1] for call in probe.call_args_list], [5 * disk.MIB] * 8 + [disk.MIB])

    def test_native_hot_gate_requires_full_coverage_get_absence_and_real_fuse_reads(self):
        before = dict(l2_hit_bytes=0, remote_bytes=0, source_get_attempts=0, fuse_reads=0)
        hot = dict(l2_hit_bytes=1024, remote_bytes=0, source_get_attempts=0, fuse_reads=8)
        disk.verify_native_hot(before, hot, 1024)
        for changed in [dict(hot, l2_hit_bytes=512), dict(hot, source_get_attempts=1),
                        dict(hot, remote_bytes=512), dict(hot, fuse_reads=0)]:
            with self.assertRaises(RuntimeError):
                disk.verify_native_hot(before, changed, 1024)

    def test_startup_failure_gate_requires_bypass_and_actual_source_fallback_not_disk_error_counters(self):
        before = dict(state_bypass=1, stats_available=0, remote_bytes=0, source_get_attempts=0, fuse_reads=0)
        after = dict(before, remote_bytes=1024, source_get_attempts=1, fuse_reads=8)
        disk.verify_startup_fallback(before, after, 1024)
        prepared = dict(before, state_bypass=0, state_prepared=1, funding_bytes=5672960)
        disk.verify_startup_fallback(prepared, after, 1024)
        for changed in [dict(after, state_bypass=0), dict(after, stats_available=1),
                        dict(after, remote_bytes=0), dict(after, source_get_attempts=0), dict(after, fuse_reads=0)]:
            with self.assertRaises(RuntimeError):
                disk.verify_startup_fallback(before, changed, 1024)

    def test_engine_open_enospc_log_accepts_lazy_error_relay_not_unrelated_errno(self):
        disk.verify_enospc_log('disk cache unavailable; using source reads: filesystem error: 28\n')
        disk.verify_enospc_log('disk cache close failed: invalid configuration: filesystem error: 28\n')
        for text in ['filesystem error: 28', 'disk cache close failed: invalid configuration: filesystem error: 5',
                     'disk cache unavailable; using source reads: timeout']:
            with self.assertRaisesRegex(RuntimeError, 'actual ENOSPC'):
                disk.verify_enospc_log(text)

    def test_fuse_mount_visibility_is_not_disk_readiness_or_a_zero_snapshot(self):
        recovering = dict(ready=0, bypass=0, closed=0, available=0)
        ready = dict(ready=1, bypass=0, closed=0, available=1)
        with patch.object(disk, 'sample_endpoint', side_effect=[recovering, ready, {'l2_hits': 7}]), patch.object(disk.time, 'sleep'):
            self.assertEqual(disk.disk_ready(1, {'disk_metrics': {}}), {'l2_hits': 7})
        with patch.object(disk, 'sample_endpoint', return_value=dict(recovering, bypass=1)):
            with self.assertRaisesRegex(RuntimeError, 'bypassed/closed'):
                disk.disk_ready(1, {'disk_metrics': {}})
        with patch.object(disk, 'sample_endpoint', return_value=recovering), patch.object(disk.time, 'monotonic', side_effect=[0, 31]):
            with self.assertRaisesRegex(RuntimeError, 'not Ready within thirty seconds'):
                disk.disk_ready(1, {'disk_metrics': {}})

    def test_native_snapshot_preserves_failure_metrics_and_counts_get_attempts_not_list(self):
        text = ('geesefs_diagnostics_enabled 1\nx_total 11\ngeesefs_fuse_replies_total{opcode="read",errno="OK"} 8\n'
                'geesefs_s3_attempts_total{operation="get",transport="sdk"} 3\n'
                'geesefs_s3_attempts_total{operation="get",transport="extension"} 4\n'
                'geesefs_s3_attempts_total{operation="list",transport="sdk"} 99\n'
                'geesefs_stage_completions_total{stage="cache_read",outcome="cancelled"} 2\n'
                'geesefs_stage_completions_total{stage="cache_read",outcome="ok"} 100\n'
                'geesefs_diagnostic_logs_dropped_total 5\n')
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            with patch.object(disk.urllib.request, 'urlopen', return_value=io.BytesIO(text.encode())):
                result = disk.native_snapshot(1, {'disk_metrics': {'l2_hits': 'x_total'}}, output, 'observed')
            self.assertEqual(result, dict(l2_hits=11, source_get_attempts=7, fuse_reads=8,
                                          cancelled_phases=2, diagnostic_logs_dropped=5))
            self.assertEqual((output / 'observed-metrics.txt').read_text(), text)
            self.assertEqual(json.loads((output / 'observed-metrics.json').read_text()), result)
            with patch.object(disk.urllib.request, 'urlopen', return_value=io.BytesIO(b'geesefs_diagnostics_enabled 1\nx_total 0\ngeesefs_fuse_replies_total{opcode="read",errno="OK"} 0\n')):
                self.assertEqual(disk.native_snapshot(1, {'disk_metrics': {'l2_hits': 'x_total'}}, output, 'empty')['source_get_attempts'], 0)
            with patch.object(disk.urllib.request, 'urlopen', return_value=io.BytesIO(b'geesefs_diagnostics_enabled 0\nx_total 0\n')):
                with self.assertRaisesRegex(RuntimeError, 'enabled diagnostics'):
                    disk.native_snapshot(1, {'disk_metrics': {'l2_hits': 'x_total'}}, output, 'disabled')

    def test_pairs_are_independent_mounts_with_same_executable_controls(self):
        config = disk.example_manifest()
        cases = list(disk.cases(config, pairs=8, controls=8))
        keys = {(c['state'], c['workload'], c['pair']) for c in cases}
        for key in keys:
            arms = [c['arm'] for c in cases if (c['state'], c['workload'], c['pair']) == key]
            self.assertCountEqual(arms, ['baseline', 'candidate', 'control'])
        self.assertEqual(len({c['mount_id'] for c in cases}), len(cases))
        self.assertNotEqual([c['arm'] for c in cases[:3]], [c['arm'] for c in cases[3:6]])
        extra = list(disk.cases(config, pairs=8, controls=9))
        last = [c['arm'] for c in extra if c['workload'] == 'metadata' and c['pair'] == 8]
        self.assertCountEqual(last, ['baseline', 'control'])

    def test_no_tiny_pairs_or_dataset_that_fits_l1_or_fake_cold(self):
        config = disk.example_manifest()
        disk.validate(config, 8, 8)
        for pairs, controls in [(7, 8), (8, 7), (True, 8)]:
            with self.assertRaises(ValueError):
                disk.validate(config, pairs, controls)
        for field, value in [('dataset_bytes', 1024), ('eviction_bytes', 1024),
                             ('states', ['cold']), ('memory_limit_mb', True)]:
            changed = copy.deepcopy(config)
            changed[field] = value
            with self.assertRaises(ValueError, msg=field):
                disk.validate(changed, 8, 8)

    def test_runtime_pending_does_not_become_a_successful_cache_hit(self):
        before = {'l1_hit_bytes': 0, 'l2_hit_bytes': 0, 'remote_bytes': 0, 'source_get_attempts': 0, 'fuse_reads': 0}
        with self.assertRaisesRegex(RuntimeError, 'L2'):
            disk.verify_state('l2_hot_pagecache_hot', before, dict(before), 'candidate')
        after = dict(before, l2_hit_bytes=4096, fuse_reads=1)
        disk.verify_state('l2_hot_pagecache_hot', before, after, 'candidate')
        for changed in [dict(after, source_get_attempts=1), dict(after, fuse_reads=0)]:
            with self.assertRaises(RuntimeError):
                disk.verify_state('l2_hot_pagecache_hot', before, changed, 'candidate')
        after['remote_bytes'] = 1
        with self.assertRaisesRegex(RuntimeError, 'remote'):
            disk.verify_state('l2_hot_pagecache_hot', before, after, 'candidate')
        with self.assertRaisesRegex(RuntimeError, 'missing'):
            disk.verify_state('l1_hot', {}, {}, 'candidate')

    def test_metrics_exact_names_not_substrings_or_reset_counters(self):
        mapping = {'l2_hit_bytes': 'test_hit_total'}
        self.assertEqual(disk.parse_metrics('test_hit_total 17\ntest_hit_total_extra 99\n', mapping), {'l2_hit_bytes': 17})
        with self.assertRaisesRegex(RuntimeError, 'missing'):
            disk.parse_metrics('test_hit_total_extra 99\n', mapping)
        with self.assertRaisesRegex(RuntimeError, 'reset'):
            disk.delta({'x': 2}, {'x': 1}, 'x')

    def test_constant_total_budget_reports_reduced_effective_l1(self):
        baseline = {'payload_limit_bytes': 64 << 20, 'read_cache_policy_bytes': 32 << 20}
        candidate = dict(baseline, read_cache_policy_bytes=24 << 20)
        receipt = disk.budget_comparison(baseline, candidate, 64)
        self.assertEqual(receipt['l1_reduction_bytes'], 8 << 20)
        self.assertFalse(receipt['disk_memory_is_free'])
        with self.assertRaisesRegex(RuntimeError, 'payload'):
            disk.budget_comparison(baseline, dict(candidate, payload_limit_bytes=128 << 20), 64)

    def test_missing_measurements_and_unverified_cold_never_qualify(self):
        report = disk.summarize([], disk.example_manifest(), 8, 8)
        self.assertFalse(report['qualified'])
        self.assertFalse(report['production_accepted'])
        self.assertIn('missing independent mount observations', report['errors'])

    def test_observed_state_screen_is_not_cold_or_go_acceptance_and_requires_controls(self):
        config = disk.example_manifest()
        records = []
        for case in disk.cases(config, 8, 8):
            candidate = case['arm'] == 'candidate'
            metrics = {name: 80 if candidate else 100 for name in disk.METRICS}
            metrics['throughput'] = 125 if candidate else 100
            plan = disk.operation_plan(disk.dataset_config(config, case['state'], case['workload']),
                                       case['workload'], case['state'])
            metrics.update(operation_plan=plan, cycles=plan['cycles'], bytes=plan['bytes'], operations=plan['operations'])
            row = dict(case, metrics=metrics, correctness=True, independent_mount=True,
                effective_budgets={'payload_limit_bytes': 64 << 20,
                                    'read_cache_policy_bytes': (24 if candidate else 32) << 20})
            if case['workload'] in disk.READS:
                row['cache_before'] = dict(l1_hit_bytes=0, l2_hit_bytes=0, remote_bytes=0, source_get_attempts=0, fuse_reads=0)
                row['cache_after'] = dict(l1_hit_bytes=4096, l2_hit_bytes=4096 if candidate else 0,
                                         remote_bytes=0, source_get_attempts=0, fuse_reads=1)
            if case['workload'] == 'eviction':
                row['capacity_preparation'] = dict(byte_correct=True, bytes=536870912, reads=4096,
                                                   protocol=config['eviction_preparation'])
            if candidate and case['workload'] == 'eviction':
                row.update(eviction_seed=dict(byte_correct=True,
                                              before=dict(l2_hit_bytes=0, remote_bytes=0, source_get_attempts=0, fuse_reads=0),
                                              after=dict(l2_hit_bytes=5242880, remote_bytes=0, source_get_attempts=0, fuse_reads=1, region_reclaims_started=0)),
                           cache_before=dict(region_reclaims_started=0),
                           cache_after=dict(region_reclaims_started=1, l2_misses=0, remote_bytes=0),
                           eviction_probe_after=dict(l2_misses=1, remote_bytes=4096),
                           eviction_evidence=dict(probe_byte_correct=True))
            records.append(row)
        result = disk.summarize(records, config, 8, 8)
        self.assertTrue(result['observed_state_screen_passed'], result['errors'])
        self.assertTrue(result['qualified'], 'unsupported heap/physical-I/O facts do not veto observed-state screening')
        self.assertIn('index_heap_bytes', result['unavailable_disk_observations'])
        self.assertFalse(result['production_accepted'])
        self.assertFalse(result['full_matrix_qualified'])
        self.assertFalse(result['summaries']['random/l2_hot_pagecache_cold_requested']['cold_state_evidence']['qualified'])
        hot = next(row for row in records if row['workload'] == 'sequential' and row['state'] == 'l2_hot_pagecache_hot' and row['arm'] == 'candidate')
        hot['cache_after']['l2_hit_bytes'] = 0
        self.assertFalse(disk.summarize(records, config, 8, 8)['qualified'], 'no validated L2 hit must never qualify')
        hot['cache_after']['l2_hit_bytes'] = 4096
        hot['metrics']['bytes'] += 1
        self.assertFalse(disk.summarize(records, config, 8, 8)['qualified'], 'unequal work must never qualify')
        hot['metrics']['bytes'] -= 1
        eviction = next(row for row in records if row['workload'] == 'eviction' and row['arm'] == 'candidate')
        evidence = eviction.pop('eviction_seed')
        unproven = disk.summarize(records, config, 8, 8)
        self.assertFalse(unproven['qualified'])
        self.assertTrue(any('eviction evidence' in error for error in unproven['errors']))
        eviction['eviction_seed'] = evidence
        del records[next(i for i, row in enumerate(records) if row['arm'] == 'control')]
        result = disk.summarize(records, config, 8, 8)
        self.assertFalse(result['qualified'])
        self.assertTrue(any('insufficient_controls' in error for error in result['errors']))

    def test_advisory_cold_cannot_be_declared_as_proven_superiority(self):
        config = disk.example_manifest()
        config['superiority_targets'] = {'random/l2_hot_pagecache_cold_requested': ['throughput']}
        with self.assertRaisesRegex(ValueError, 'advisory cold'):
            disk.validate(config, 8, 8)

    def test_private_output_is_non_overwriting(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'evidence'
            disk.prepare_output(path)
            (path / 'receipt').write_text('keep\n')
            with self.assertRaisesRegex(ValueError, 'not empty'):
                disk.prepare_output(path)
            self.assertEqual((path / 'receipt').read_text(), 'keep\n')

    def test_only_owned_cache_files_receive_pagecache_advice(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cache = root / 'cache'
            cache.mkdir()
            (root / 'outside').write_bytes(b'never traverse')
            (cache / 'escape').symlink_to(root / 'outside')
            with self.assertRaisesRegex(RuntimeError, 'symlink'):
                disk.cache_page_state(cache, cold=True)

    def test_worker_checks_real_bytes_and_does_not_count_cycles_as_mounts(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)
            expected = disk.integration.expected_range(0, 1024)
            (path / 'payload').write_bytes(expected)
            config = disk.example_manifest()
            config.update(dataset_bytes=1024, measurement_cycles=2, l1_sequential_cycles=2)
            spec = dict(path=str(path), endpoint='http://127.0.0.1:9', bucket='fixture', prefix='data',
                        expected_sha=disk.hashlib.sha256(expected).hexdigest(), name='sequential',
                        state='l1_hot', config=config)
            environment = disk.integration.private_environment('disposable-access', 'disposable-secret')
            command = [sys.executable, str(Path(disk.__file__)), '--worker']
            result = subprocess.run(command, input=json.dumps(spec) + '\nstart\ndone\n',
                                    capture_output=True, text=True, env=environment, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            messages = [json.loads(line) for line in result.stdout.splitlines()]
            metrics = messages[1]['metrics']
            self.assertEqual(metrics['independent_mounts'], 1)
            self.assertEqual(metrics['cycles'], 2)
            self.assertEqual(metrics['bytes'], 2048)
            (path / 'payload').write_bytes(b'wrong-generation')
            result = subprocess.run(command, input=json.dumps(spec) + '\nstart\ndone\n',
                                    capture_output=True, text=True, env=environment, timeout=15)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('checksum', result.stdout)

    def test_measured_pressure_is_one_fixed_pass_and_never_refreshes_witness(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'payload').write_bytes(b'untouched witness')
            (root / 'capacity-pressure').mkdir()
            payload = b'fixed independently checked pressure' * 32
            (root / 'capacity-pressure/payload').write_bytes(payload)
            config = dict(disk.example_manifest(), dataset_bytes=len(payload), measurement_cycles=8)
            spec = dict(path=str(root), endpoint='http://127.0.0.1:9', bucket='fixture', prefix='data',
                        expected_sha=disk.hashlib.sha256(payload).hexdigest(), name='eviction', state='normal', config=config)
            result = subprocess.run([sys.executable, disk.__file__, '--worker'],
                input=json.dumps(spec) + '\nstart\ndone\n', capture_output=True, text=True,
                env=disk.integration.private_environment('disposable-access', 'disposable-secret'), timeout=15)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            metrics = json.loads(result.stdout.splitlines()[1])['metrics']
            self.assertEqual(metrics['cycles'], 1)
            self.assertEqual(metrics['bytes'], len(payload))
            self.assertEqual((root / 'payload').read_bytes(), b'untouched witness')

    def test_manifest_cannot_silently_omit_required_cache_evidence(self):
        config = disk.example_manifest()
        del config['disk_metrics']['l2_hit_bytes']
        with self.assertRaisesRegex(ValueError, 'disk_metrics'):
            disk.validate(config, 8, 8)

    def test_mapping_does_not_relabel_counts_funding_or_active_owners(self):
        config = disk.example_manifest()
        mapping = config['disk_metrics']
        self.assertEqual(mapping['index_entries'], 'geesefs_disk_cache_index_entries')
        self.assertEqual(mapping['submission_bytes'], 'geesefs_disk_cache_submission_bytes{scope="queued_and_active"}')
        self.assertEqual(mapping['active_jobs'], 'geesefs_disk_cache_active_jobs')
        self.assertNotIn('index_bytes', mapping)
        self.assertNotIn('queued_jobs', mapping)
        self.assertNotIn('evictions', mapping)
        self.assertNotIn('demotions', mapping)
        self.assertEqual(mapping['admitted_demotions'], 'geesefs_disk_cache_admitted_demotions_total')
        self.assertEqual(mapping['region_reclaims_started'], 'geesefs_disk_cache_region_reclaims_started_total')
        self.assertEqual(mapping['read_duration_seconds'], 'geesefs_disk_cache_read_operation_duration_seconds_total')
        self.assertEqual(mapping['recovery_seconds'], 'geesefs_disk_cache_open_phase_duration_seconds_total{phase="recovery"}')
        self.assertNotIn('read_latency', config['unavailable_disk_observations'])
        self.assertIn('capacity_evictions', config['unavailable_disk_observations'])
        scoped = {'submission_bytes': mapping['submission_bytes']}
        self.assertEqual(disk.parse_metrics('geesefs_disk_cache_submission_bytes{scope="queued_and_active"} 42\n', scoped), {'submission_bytes': 42})
        with self.assertRaisesRegex(RuntimeError, 'missing'):
            disk.parse_metrics('geesefs_disk_cache_submission_bytes{scope="queued_only"} 42\n', scoped)
        changed = copy.deepcopy(config)
        changed['unavailable_disk_observations'] = []
        with self.assertRaisesRegex(ValueError, 'instrumentation gaps'):
            disk.validate(changed, 8, 8)

    def test_region_callbacks_require_a_previously_cached_probe_and_actual_source_fallback(self):
        before = dict(region_reclaims_started=2)
        probe_before = dict(l2_misses=3, remote_bytes=17)
        probe_after = dict(l2_misses=4, remote_bytes=4113)
        result = disk.verify_eviction(before, dict(region_reclaims_started=3), probe_before, probe_after)
        self.assertEqual(result['region_reclaims_started'], 1)
        self.assertEqual(result['probe_remote_bytes'], 4096)
        self.assertFalse(result['completed_reclaims_measured'])
        for after, probe in [(before, probe_after), (dict(region_reclaims_started=3), probe_before),
                             (dict(region_reclaims_started=3), dict(probe_after, remote_bytes=17))]:
            with self.assertRaisesRegex(RuntimeError, 'eviction evidence'):
                disk.verify_eviction(before, after, probe_before, probe)

    def test_once_at_open_allocation_is_optional_not_fake_zero(self):
        mapping = {'opened_file_available': 'available', 'opened_file_allocated_bytes': 'blocks'}
        self.assertEqual(disk.parse_metrics('available 0\n', mapping),
                         dict(opened_file_available=0, opened_file_allocated_bytes=None))
        with self.assertRaisesRegex(RuntimeError, 'missing'):
            disk.parse_metrics('available 1\n', mapping)

    def test_eviction_probe_checks_exact_source_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)
            (path / 'payload').write_bytes(disk.integration.expected_range(0, 4096))
            self.assertEqual(disk.eviction_probe(path, 4096), 4096)
            (path / 'payload').write_bytes(b'wrong-generation')
            with self.assertRaisesRegex(AssertionError, 'eviction probe'):
                disk.eviction_probe(path, 4096)

    def test_run_failure_preserves_owned_logs(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            with self.assertRaisesRegex(RuntimeError, 'injected'):
                with disk.evidence_work(output) as work:
                    self.assertEqual(work.parent, output, 'scratch must use the explicitly selected evidence filesystem, not ambient tmpfs')
                    (work / 'daemon.log').write_text('failure evidence\n')
                    raise RuntimeError('injected')
            self.assertEqual((output / 'daemon.log').read_text(), 'failure evidence\n')


if __name__ == '__main__':
    unittest.main()
