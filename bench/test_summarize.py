import unittest

from summarize import summarize


class SummaryTests(unittest.TestCase):
    def test_failed_and_missing_measurements_are_retained_not_replaced_by_zero(self):
        common = {"baseline": "a" * 40, "state": "cold", "workload": "sequential", "implementation": "rust"}
        runs = [dict(common, repetition=0, correctness=True, metrics={"throughput": 10, "rss_peak_bytes": 8}),
                dict(common, repetition=1, correctness=False, error="checksum mismatch", metrics={"throughput": 20}),
                dict(common, repetition=2, correctness=False, metrics={"throughput": float("nan"), "rss_peak_bytes": True})]
        report = {"runs": runs, "gate": {"accepted": False, "errors": ["incomplete"]},
                  "limits": {"memory_limit_mb": 64, "private_path": "private"},
                  "provenance": {"rust": {"source_revision": "b" * 40, "binary_sha256": "c" * 64, "private_path": "private"}}}
        result = summarize(report)
        self.assertFalse(result["accepted"])
        self.assertEqual(result["runs"], 3)
        self.assertEqual(result["gate_error_count"], 1)
        self.assertNotIn("private_path", result["provenance"]["rust"])
        self.assertEqual(result["limits"], {"memory_limit_mb": 64})
        group, = result["groups"]
        self.assertEqual(group["correct"], 1)
        self.assertEqual(group["errors"], [{"repetition": 1, "error": "checksum mismatch"}])
        self.assertEqual(group["metrics"]["throughput"], {"n": 2, "missing": 1, "mean": 15, "stdev": 5, "min": 10, "max": 20})
        self.assertEqual(group["metrics"]["rss_peak_bytes"]["n"], 1)
        self.assertEqual(group["metrics"]["rss_peak_bytes"]["mean"], 8)
        self.assertEqual(group["metrics"]["allocator_peak_bytes"], {"n": 0, "missing": 3})

    def test_cpu_cost_and_missing_old_cpu_receipts_remain_distinct(self):
        common = {"baseline": "a" * 40, "state": "cold", "workload": "sequential", "implementation": "rust"}
        result = summarize({"runs": [dict(common, repetition=0, correctness=True,
            metrics={"daemon_cpu_seconds": 2, "cgroup_cpu_seconds": 3, "cgroup_cpu_seconds_per_gib": 6}),
            dict(common, repetition=1, correctness=True, metrics={})]})
        group, = result["groups"]
        self.assertEqual(group["metrics"]["cgroup_cpu_seconds"], {"n": 1, "missing": 1, "mean": 3, "stdev": 0, "min": 3, "max": 3})
        self.assertEqual(group["metrics"]["cgroup_cpu_seconds_per_gib"]["mean"], 6)
