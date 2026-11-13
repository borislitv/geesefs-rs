"""Reachable byte, object-selection and finite preparation contracts."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import capacity_population as pressure


class Contracts(unittest.TestCase):
    def test_fixed_schedule_covers_each_block_exactly_once(self):
        actual = list(pressure.offsets(4 * pressure.BLOCK, 1729))
        self.assertEqual(sorted(actual), [0, 131072, 262144, 393216])
        self.assertNotEqual(actual, sorted(actual))
        self.assertEqual(actual, list(pressure.offsets(4 * pressure.BLOCK, 1729)))

    def test_invalid_size_does_not_create_an_unbounded_schedule(self):
        for size in [0, -1, 1, pressure.MAX_DATASET + pressure.BLOCK]:
            with self.subTest(size=size), self.assertRaises(ValueError):
                pressure.offsets(size, 1729)

    def test_block_size_cannot_expand_the_bounded_schedule(self):
        for block in [0, -1, 1, 4096, 2 * pressure.BLOCK, True]:
            with self.subTest(block=block), self.assertRaises(ValueError):
                pressure.offsets(pressure.BLOCK, 1729, block=block)

    def invoke(self, path, size, rounds=1, commands='next\nnext\n'):
        spec = dict(path=str(path), size=size, rounds=rounds, seed=1729)
        return subprocess.run([sys.executable, pressure.__file__], input=json.dumps(spec) + '\n' + commands,
                              text=True, capture_output=True, timeout=10)

    def correct_payload(self, temporary):
        parent = Path(temporary) / 'capacity-pressure'
        parent.mkdir()
        path = parent / 'payload'
        # Fixture format: independently generate its two 64-KiB source chunks.
        path.write_bytes(b''.join(hashlib.shake_256(b'geesefs-rs-position-fixture-v1' +
                         index.to_bytes(8, 'little')).digest(65536) for index in [0, 1]))
        return path

    def test_checked_rounds_report_exact_bytes_and_complete(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = self.correct_payload(temporary)
            result = self.invoke(path, pressure.BLOCK, rounds=2, commands='next\n' * 4)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([json.loads(line) for line in result.stdout.splitlines()], [
            {'ready_round': 0}, {'offset': 0, 'bytes': 131072},
            {'round_complete': 0, 'bytes': 131072}, {'ready_round': 1},
            {'offset': 0, 'bytes': 131072}, {'round_complete': 1, 'bytes': 131072}])

    def test_invalid_round_bounds_are_rejected_before_any_read(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = self.correct_payload(temporary)
            for rounds in [0, -1, 5, True, 1.5]:
                with self.subTest(rounds=rounds):
                    result = self.invoke(path, pressure.BLOCK, rounds=rounds)
                    self.assertEqual(result.returncode, 1)
                    self.assertIn('four rounds or 2 GiB', json.loads(result.stdout)['error'])

    def test_lost_controller_is_not_successful_population(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = self.correct_payload(temporary)
            result = self.invoke(path, pressure.BLOCK, commands='next\n')
        self.assertEqual(result.returncode, 1)
        self.assertIn('controller ended', json.loads(result.stdout.splitlines()[-1])['error'])

    def test_total_read_limit_is_checked_before_any_pressure_read(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = self.correct_payload(temporary)
            with path.open('r+b') as stream:
                stream.truncate(512 * 1024**2 + pressure.BLOCK)
            result = self.invoke(path, path.stat().st_size, rounds=4)
        self.assertEqual(result.returncode, 1)
        self.assertIn('four rounds or 2 GiB', json.loads(result.stdout)['error'])

    def test_fixed_controller_checks_bytes_and_drains_each_read(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.correct_payload(temporary)
            drained = []
            result = pressure.populate(root, pressure.BLOCK, os.environ.copy(), None,
                                       root / 'client.log', lambda: drained.append('idle'))
        self.assertEqual(result['bytes'], 131072)
        self.assertEqual(result['reads'], 1)
        self.assertTrue(result['byte_correct'])
        self.assertEqual(result['protocol']['rounds'], 1)
        self.assertEqual(drained, ['idle'])

    def test_fixed_controller_does_not_count_corrupt_reads_or_hide_drain_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = self.correct_payload(temporary)
            def failing_drain():
                raise RuntimeError('real optional work failed to drain')
            with self.assertRaisesRegex(RuntimeError, 'failed to drain'):
                pressure.populate(root, pressure.BLOCK, os.environ.copy(), None, root / 'drain.log', failing_drain)
            path.write_bytes(b'\x00' * pressure.BLOCK)
            drained = []
            with self.assertRaisesRegex(RuntimeError, 'bytes mismatch'):
                pressure.populate(root, pressure.BLOCK, os.environ.copy(), None,
                                  root / 'corrupt.log', lambda: drained.append('unexpected'))
            self.assertEqual(drained, [])

    def test_explicit_bounded_stop_does_not_claim_round_completion(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = self.correct_payload(temporary)
            result = self.invoke(path, pressure.BLOCK, commands='next\nstop\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(any('round_complete' in json.loads(line) for line in result.stdout.splitlines()))

    def test_wrong_filename_cannot_refresh_the_target_witness(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'payload'
            path.write_bytes(b'not a pressure identity')
            result = self.invoke(path, 23)
        self.assertEqual(result.returncode, 1)
        self.assertIn('distinct capacity-pressure/payload', json.loads(result.stdout)['error'])

    def test_corrupt_bytes_are_rejected_not_counted_as_pressure(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary) / 'capacity-pressure'
            parent.mkdir()
            path = parent / 'payload'
            path.write_bytes(b'\x00' * pressure.BLOCK)
            result = self.invoke(path, pressure.BLOCK)
        self.assertEqual(result.returncode, 1)
        messages = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertIn('bytes mismatch', messages[-1]['error'])
        self.assertFalse(any('bytes' in row for row in messages))


if __name__ == '__main__':
    unittest.main()
