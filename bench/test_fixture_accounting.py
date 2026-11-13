"""Independent accounting scopes, with controlled OS counter boundaries."""
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

from fixture_accounting import FixtureSampler


class Contracts(unittest.TestCase):
    def test_simultaneous_fixture_rss_does_not_omit_provider_or_mix_target_metrics(self):
        sampled = threading.Event()
        class Stop(threading.Event):
            def wait(self, timeout=None):
                sampled.set()  # run() has written/flushed a complete sample.
                return super().wait(timeout)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'memory.current').write_text('65536')
            (root / 'memory.peak').write_text('81920')
            processes = dict(provider=11, supervisor=12, daemon=13, client=14)
            resident = {11: 4096, 12: 8192, 13: 12288, 14: 16384}
            with patch('fixture_accounting.CpuWindow'), \
                 patch('fixture_accounting.proc_memory', side_effect=lambda pid: {'rss': resident[pid]}), \
                 patch('fixture_accounting.process_io', return_value=dict(read_bytes=0, write_bytes=0)):
                sampler = FixtureSampler(processes, root, root / 'trace.jsonl')
                sampler.done = Stop()
                with sampler:
                    self.assertTrue(sampled.wait(5), 'fixture sampler did not produce a sample')
                result = sampler.metrics()
            self.assertEqual(result['memory']['process_rss']['peak'], 40960)
            self.assertEqual(result['memory']['provider']['peak'], 4096)
            self.assertEqual(result['memory']['cgroup_memory']['peak'], 65536)
            self.assertEqual(result['cgroup_peak_bytes'], 81920)
            self.assertEqual(set(result['process_io_delta']), set(processes))
            self.assertIn('not sum of independent peaks', result['rss_scope'])


if __name__ == '__main__':
    unittest.main()
