import contextlib
import io
import json
import os
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
from unittest import mock


HERE = Path(__file__).resolve().parent


class CommandLineTests(unittest.TestCase):
    def execute(self, script, *arguments):
        output, errors = io.StringIO(), io.StringIO()
        with mock.patch.object(sys, "argv", [str(HERE / script), *map(str, arguments)]), \
             contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            try:
                runpy.run_path(str(HERE / script), run_name="__main__")
            except SystemExit as exit:
                return exit.code, output.getvalue(), errors.getvalue()
        return 0, output.getvalue(), errors.getvalue()

    def test_invalid_workloads_produce_a_failed_report_and_preserve_cpu_affinity(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            workloads = work / "workloads.json"
            workloads.write_text('{"schema": 1, "repetitions": 4}\n')
            affinity = os.sched_getaffinity(0)
            status, output, _ = self.execute("compare.py", "--workloads", workloads,
                                             "--output", work / "evidence", "--timeout", "10")
            self.assertEqual(status, 1)
            report = json.loads((work / "evidence/report.json").read_text())
            self.assertFalse(report["gate"]["accepted"])
            self.assertIn("at least five repetitions", report["gate"]["errors"][0])
            self.assertEqual(report["runs"], [])
            self.assertFalse(json.loads(output)["accepted"])
            self.assertEqual(os.sched_getaffinity(0), affinity)

    def test_existing_evidence_is_not_overwritten_and_deadline_must_be_positive(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            report = work / "report.json"
            report.write_text("previous evidence\n")
            status, _, error = self.execute("compare.py", "--workloads", HERE / "workloads.json",
                                            "--output", work)
            self.assertEqual(status, 2)
            self.assertIn("output directory is not empty", error)
            self.assertEqual(report.read_text(), "previous evidence\n")
            status, _, _ = self.execute("compare.py", "--workloads", HERE / "workloads.json",
                                        "--output", work / "new", "--timeout", "0")
            self.assertEqual(status, "--timeout must be positive")
            self.assertFalse((work / "new").exists())

    def test_summary_cli_keeps_missing_values_and_exclusive_export(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            report = work / "report.json"
            report.write_text(json.dumps({"gate": {"accepted": False}, "runs": [{
                "baseline": "a" * 40, "state": "warm", "workload": "random",
                "implementation": "rust", "repetition": 0, "correctness": True,
                "metrics": {"throughput": 12},
            }]}))
            status, table, _ = self.execute("summarize.py", report, "--table", "--metrics",
                                            "throughput", "cgroup_cpu_seconds")
            self.assertEqual(status, 0)
            self.assertIn("accepted=False runs=1", table)
            self.assertIn("1/1 12 missing", table)
            export = work / "summary.json"
            self.assertEqual(self.execute("summarize.py", report, "--output", export)[0], 0)
            saved = export.read_bytes()
            self.assertEqual(json.loads(saved)["groups"][0]["metrics"]["cgroup_cpu_seconds"],
                             {"n": 0, "missing": 1})
            with self.assertRaises(FileExistsError):
                self.execute("summarize.py", report, "--output", export)
            self.assertEqual(export.read_bytes(), saved)
            self.assertEqual(self.execute("summarize.py", report, "--metrics", "throughput")[0], 2)
            self.assertEqual(self.execute("summarize.py", report, "--output", export, "--table")[0], 2)
