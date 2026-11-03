import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import compare


class MemoryWindowTests(unittest.TestCase):
    def sample(self, rows, finished, client_scale=1):
        with tempfile.TemporaryDirectory() as temporary:
            trace = Path(temporary) / 'memory.jsonl'
            sampler = compare.Sampler(os.getpid(), os.getppid(), None, trace)
            position = 0

            def clock():
                return rows[position][0] if position < len(rows) else finished

            def memory(_pid):
                value = rows[position][1]
                scale = 1 if _pid == os.getpid() else client_scale
                return {'rss': scale * value} if value is not None else {}

            def advance(_seconds):
                nonlocal position
                position += 1
                if position == len(rows):
                    sampler.done.set()

            # Control only OS observations and timer ticks, not aggregation.
            with mock.patch.object(compare, 'proc_memory', side_effect=memory), \
                 mock.patch.object(compare.time, 'monotonic_ns', side_effect=clock), \
                 mock.patch.object(sampler.done, 'wait', side_effect=advance):
                with sampler:
                    sampler.thread.join(timeout=3)
                    self.assertFalse(sampler.thread.is_alive())
            self.assertEqual(len(trace.read_text().splitlines()), len(rows))
            return sampler.metrics()

    def test_late_window_keeps_sustained_high_memory_after_one_thousand_samples(self):
        result = self.sample([(at, 100 if at < 1100 else 10) for at in range(1600)], 1600)
        self.assertEqual(result['rss_peak_bytes'], 200)
        # Temporal latter half is 800..1599: 300 high and 500 low samples,
        # for daemon plus client. A capped deque silently reports only 20.
        self.assertEqual(result['rss_steady_bytes'], 87.5)

    def test_late_means_follow_time_not_sample_count(self):
        result = self.sample([(0, 30), (30, 90), (70, 50)], 100)
        self.assertEqual(result['rss_peak_bytes'], 180)
        self.assertEqual(result['rss_steady_bytes'], 100)

    def test_missing_process_memory_is_not_zero_or_a_partial_mean(self):
        result = self.sample([(0, 30), (70, None), (90, 50)], 100)
        self.assertIsNone(result['rss_peak_bytes'])
        self.assertIsNone(result['rss_steady_bytes'])

    def test_missing_early_sample_does_not_hide_a_complete_late_window(self):
        result = self.sample([(0, None), (30, 90), (70, 50)], 100)
        self.assertIsNone(result['rss_peak_bytes'])
        self.assertEqual(result['rss_steady_bytes'], 100)

    def test_zero_is_a_valid_observation_not_a_missing_sample(self):
        result = self.sample([(0, 0), (70, 0)], 100)
        self.assertEqual(result['rss_peak_bytes'], 0)
        self.assertEqual(result['rss_steady_bytes'], 0)

    def test_daemon_and_client_remain_separate_from_combined_rss(self):
        result = self.sample([(0, 30), (30, 90), (70, 50)], 100, client_scale=2)
        self.assertEqual(result['daemon_rss_peak_bytes'], 90)
        self.assertEqual(result['client_rss_peak_bytes'], 180)
        self.assertEqual(result['daemon_rss_steady_bytes'], 50)
        self.assertEqual(result['client_rss_steady_bytes'], 100)
        self.assertEqual(result['rss_peak_bytes'], 270)
        self.assertEqual(result['rss_steady_bytes'], 150)


class RustAllocatorWindowTests(unittest.TestCase):
    def test_allocator_late_mean_uses_the_temporal_half(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'allocator.jsonl'
            path.write_text(''.join(json.dumps({'unix_time_ns': at, 'allocated_bytes': size}) + '\n'
                                    for at, size in [(10, 30), (30, 90), (70, 50)]))
            peak, late, _effective = compare.allocator_rust(path, 10, 100)
        self.assertEqual(peak, 90)
        self.assertEqual(late, 50)

    def test_empty_or_invalid_allocator_trace_is_not_zero_allocated_memory(self):
        for rows in [[], [(10, None), (70, 50)], [(10, True), (70, 50)], [(10, -1), (70, 50)]]:
            with self.subTest(rows=rows), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / 'allocator.jsonl'
                path.write_text(''.join(json.dumps({'unix_time_ns': at, 'allocated_bytes': size}) + '\n'
                                       for at, size in rows))
                peak, late, _effective = compare.allocator_rust(path, 10, 100)
                self.assertIsNone(peak)
                self.assertIsNone(late)
