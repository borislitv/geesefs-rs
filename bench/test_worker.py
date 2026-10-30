import hashlib
import io
import json
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
from unittest import mock

import compare


class WorkerWarmupTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        payload = b"independent worker fixture"
        (self.path / "payload").write_bytes(payload)
        (self.path / "implicit").mkdir()
        (self.path / "implicit/000000").write_bytes(b"entry-0")
        (self.path / "implicit/000001").write_bytes(b"entry-1")
        self.spec = {"path": str(self.path), "name": "metadata", "config": {"metadata_entries": 2},
                     "bucket": "unused", "prefix": "unused", "endpoint": "http://127.0.0.1:1",
                     "local": True, "expected_sha": hashlib.sha256(payload).hexdigest()}

    def execute(self, warm):
        spec = dict(self.spec, warmup=warm)
        stdin = io.StringIO(json.dumps(spec) + "\nstart\nstop\n")
        self.output = io.StringIO()
        client = mock.Mock()
        with mock.patch.dict("os.environ", AWS_ACCESS_KEY_ID="fixture", AWS_SECRET_ACCESS_KEY="fixture"), \
             mock.patch("boto3.client", return_value=client), \
             mock.patch.object(compare, "workload", wraps=compare.workload) as operation, \
             mock.patch.object(sys, "stdin", stdin), mock.patch.object(sys, "stdout", self.output), \
             mock.patch.object(sys, "path", list(sys.path)):
            try:
                runpy.run_path(str(Path(__file__).resolve().parents[1] / "scripts/bench_worker.py"))
            finally:
                self.workloads = [call.args[1] for call in operation.call_args_list]
                self.client_closed = client.close.called
        return [json.loads(line) for line in self.output.getvalue().splitlines()]

    def test_warm_metadata_primes_the_measured_operation_before_reporting_ready(self):
        messages = self.execute(True)
        self.assertEqual(self.workloads, ["metadata", "metadata"])
        self.assertEqual(messages[0], {"ready": True})
        self.assertEqual(messages[1]["metrics"]["operations"], 2)
        self.assertTrue(self.client_closed)

    def test_cold_metadata_has_no_hidden_warmup(self):
        messages = self.execute(False)
        self.assertEqual(self.workloads, ["metadata"])
        self.assertEqual(messages[1]["metrics"]["operations"], 2)

    def test_invalid_warmup_does_not_report_ready_and_releases_the_client(self):
        (self.path / "implicit/000000").write_bytes(b"bad")
        with self.assertRaisesRegex(AssertionError, "metadata size"):
            self.execute(True)
        self.assertEqual(self.output.getvalue(), "")
        self.assertTrue(self.client_closed)
