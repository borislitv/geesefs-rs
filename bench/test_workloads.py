import errno
import hashlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import compare


class WorkloadTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name)
        # The public position fixture is input data, not the workload's result.
        self.payload = compare.integration.fixture_chunk(0)
        (self.path / "payload").write_bytes(self.payload)
        (self.path / "implicit").mkdir()
        for number in range(3):
            (self.path / "implicit" / f"{number:06d}").write_text(f"entry-{number}")
        self.config = dict(metadata_entries=3, dataset_bytes=len(self.payload),
                           random_operations=4, random_block_bytes=17, concurrency=3,
                           writer_bytes=64)
        self.sha = hashlib.sha256(self.payload).hexdigest()

    def execute(self, name, client=None, local=True):
        return compare.workload(self.path, name, self.config, client, "bucket", "prefix",
                                "http://127.0.0.1:1", local, self.sha)

    def assert_metrics(self, metrics, operations, size):
        self.assertEqual(metrics["operations"], operations)
        self.assertEqual(metrics["bytes"], size)
        self.assertEqual(len(metrics["latencies_ms"]), operations)
        self.assertGreater(metrics["throughput"], 0)
        self.assertGreater(metrics["elapsed_seconds"], 0)
        self.assertLessEqual(metrics["p50_ms"], metrics["p95_ms"])
        self.assertLessEqual(metrics["p95_ms"], metrics["p99_ms"])

    def test_read_workloads_validate_bytes_and_keep_exact_operation_counts(self):
        self.assert_metrics(self.execute("metadata"), 3, 0)
        for name in ["sequential", "eviction"]:
            with self.subTest(name=name):
                self.assert_metrics(self.execute(name), 1, len(self.payload))
        self.assert_metrics(self.execute("random"), 4, 68)
        self.assert_metrics(self.execute("concurrent"), 12, 204)

    def test_corrupt_or_short_payload_and_incomplete_metadata_do_not_produce_rates(self):
        (self.path / "payload").write_bytes(b"corrupt")
        for name in ["sequential", "eviction", "random", "concurrent"]:
            with self.subTest(name=name), self.assertRaises(AssertionError):
                self.execute(name)
        (self.path / "implicit/000001").unlink()
        with self.assertRaisesRegex(AssertionError, "metadata traversal completeness"):
            self.execute("metadata")

    def remote(self, wrong_bytes=False):
        owner = self

        class Remote:
            body = None

            def get_object(self, *, Bucket, Key):
                owner.assertEqual(Bucket, "bucket")
                owner.assertTrue(Key.startswith("prefix/"))
                written = (owner.path / Key.removeprefix("prefix/")).read_bytes()
                owner.assertEqual(written, owner.payload[:64])
                self.body = io.BytesIO(b"wrong remote generation" if wrong_bytes else written)
                return {"Body": self.body}

        return Remote()

    def test_writer_verifies_full_remote_bytes_closes_the_body_and_removes_its_file(self):
        remote = self.remote()
        self.assert_metrics(self.execute("writer_fsync", remote), 1, 64)
        self.assertTrue(remote.body.closed)
        self.assertEqual(sorted(path.name for path in self.path.iterdir()), ["implicit", "payload"])
        remote = self.remote(wrong_bytes=True)
        with self.assertRaisesRegex(AssertionError, "fresh writer/fsync backend checksum"):
            self.execute("writer_fsync", remote)
        self.assertTrue(remote.body.closed)

    def test_faulted_writer_requires_injection_and_recovers_before_remote_verification(self):
        with self.assertRaisesRegex(RuntimeError, "controlled disposable fault injector"):
            self.execute("writer_failure", local=False)
        active = False
        fsync = os.fsync

        def fault(_endpoint, action=None):
            nonlocal active
            active = action == "put"

        def sync(fd):
            if active:
                raise OSError(errno.EIO, "injected upload failure")
            fsync(fd)

        remote = self.remote()
        with mock.patch.object(compare.integration, "fault", side_effect=fault), \
             mock.patch.object(os, "fsync", side_effect=sync):
            self.assert_metrics(self.execute("writer_failure", remote), 1, 64)
        self.assertFalse(active)
        self.assertTrue(remote.body.closed)
        self.assertFalse((self.path / "fault").exists())

    def test_failed_recovery_clears_injection_and_keeps_the_failed_work_out_of_results(self):
        with mock.patch.object(compare.integration, "fault") as fault, \
             mock.patch.object(os, "fsync", side_effect=OSError(errno.EIO, "injected")), \
             mock.patch.object(compare.integration, "recover_fsync", side_effect=OSError(errno.EIO, "recovery failed")):
            with self.assertRaisesRegex(OSError, "recovery failed"):
                self.execute("writer_failure", self.remote())
        self.assertEqual(fault.call_args.args, ("http://127.0.0.1:1",))
        self.assertEqual((self.path / "fault").read_bytes(), self.payload[:64])
