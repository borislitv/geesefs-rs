"""Contract tests for the mounted L2 harness; no FUSE or external service needed."""
import copy
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest

import disk_cache as disk


class Contracts(unittest.TestCase):
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
        before = {'l1_hit_bytes': 0, 'l2_hit_bytes': 0, 'remote_bytes': 0}
        with self.assertRaisesRegex(RuntimeError, 'L2'):
            disk.verify_state('l2_hot_pagecache_hot', before, dict(before), 'candidate')
        after = dict(before, l2_hit_bytes=4096)
        disk.verify_state('l2_hot_pagecache_hot', before, after, 'candidate')
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
            row = dict(case, metrics=metrics, correctness=True, independent_mount=True,
                effective_budgets={'payload_limit_bytes': 64 << 20,
                                   'read_cache_policy_bytes': (24 if candidate else 32) << 20})
            if candidate and case['workload'] == 'eviction':
                row.update(eviction_seed=dict(byte_correct=True,
                                              before=dict(l2_hit_bytes=0, remote_bytes=0),
                                              after=dict(l2_hit_bytes=4096, remote_bytes=0)),
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
            config.update(dataset_bytes=1024, minimum_active_seconds=0.005)
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
            self.assertGreater(metrics['cycles'], 1)
            self.assertEqual(metrics['bytes'], metrics['cycles'] * 1024)
            (path / 'payload').write_bytes(b'wrong-generation')
            result = subprocess.run(command, input=json.dumps(spec) + '\nstart\ndone\n',
                                    capture_output=True, text=True, env=environment, timeout=15)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('checksum', result.stdout)

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
                    (work / 'daemon.log').write_text('failure evidence\n')
                    raise RuntimeError('injected')
            self.assertEqual((output / 'daemon.log').read_text(), 'failure evidence\n')


if __name__ == '__main__':
    unittest.main()
