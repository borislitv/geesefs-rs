import copy
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock

import compare


class BaselineBuildTests(unittest.TestCase):
    def build(self, embedded_version):
        revision = compare.REVISIONS[0]
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            binary = work / "baseline"

            def command(arguments, **options):
                arguments = [str(value) for value in arguments]
                output = ""
                if arguments[:2] == ["git", "rev-parse"]:
                    output = revision + "\n"
                elif arguments[:2] == ["git", "archive"]:
                    with tarfile.open(fileobj=options["stdout"], mode="w") as archive:
                        source = b"package main\nfunc main() {}\n"
                        entry = tarfile.TarInfo("main.go")
                        entry.size = len(source)
                        archive.addfile(entry, io.BytesIO(source))
                elif arguments[:2] == ["go", "build"]:
                    binary.write_bytes(b"executable fixture\n")
                elif arguments == ["go", "version"]:
                    output = "go version go1.22.2 linux/amd64\n"
                elif arguments == ["go", "version", "-m", str(binary)]:
                    output = f"{binary}: {embedded_version}\n\tbuild\tGOARCH=amd64\n"
                else:
                    self.fail(f"unexpected build command: {arguments}")
                return subprocess.CompletedProcess(arguments, 0, stdout=output)

            with mock.patch.object(compare.integration, "command", side_effect=command):
                return compare.build_baseline(work, revision, binary, work)

    def test_auto_selected_toolchain_comes_from_built_binary_not_launcher(self):
        provenance = self.build("go1.25.9")
        self.assertEqual(provenance["toolchain"], "go1.25.9")
        self.assertEqual(provenance["source_revision"], compare.REVISIONS[0])
        self.assertRegex(provenance["binary_sha256"], r"^[0-9a-f]{64}$")

    def test_missing_embedded_toolchain_is_not_replaced_with_launcher_version(self):
        with self.assertRaisesRegex(RuntimeError, "embedded Go toolchain"):
            self.build("")


class GateTests(unittest.TestCase):
    def setUp(self):
        self.config = json.loads((Path(__file__).parent / "workloads.json").read_text())
        self.compatibility = {"accepted": True, "optional_mode_gaps": [], "capabilities": dict.fromkeys(compare.REQUIRED_COMPATIBILITY, True)}
        self.provenance = {revision: {"source_revision": revision, "binary_sha256": "a" * 64} for revision in compare.REVISIONS}
        self.provenance["rust"] = {"source_revision": "d" * 40, "binary_sha256": "b" * 64}
        self.runs = []
        for revision in compare.REVISIONS:
            for state in self.config["states"]:
                for name in self.config["workloads"]:
                    for repetition in range(self.config["repetitions"]):
                        for implementation in ["go", "rust"]:
                            metrics = dict.fromkeys(compare.REQUIRED_METRICS, 10 if implementation == "go" else 8)
                            metrics["throughput"] = 10 if implementation == "go" else 12
                            self.runs.append({"baseline": revision, "state": state, "workload": name, "repetition": repetition,
                                              "implementation": implementation, "correctness": True, "metrics": metrics,
                                               "limits": copy.deepcopy(self.config), "data_generation": {"sha256": "c" * 64},
                                               "mount_arguments": ["binary", "--memory-limit", "64", "--entry-limit", "100000"],
                                               "effective_budgets": {"payload_limit_bytes": 67108864, "cache_entry_limit": 100000},
                                               "machine": {"kernel": "fixture", "cpu": 1}})

    def gate(self):
        return compare.evaluate(self.config, self.runs, self.compatibility, self.provenance)

    def test_complete_repeated_equivalent_evidence(self):
        self.assertTrue(self.gate()["accepted"])

    def test_no_missing_results_can_pass(self):
        self.runs.pop()
        self.assertFalse(self.gate()["accepted"])

    def test_absent_cgroup_or_allocator_is_not_zero_memory(self):
        for metric in ["cgroup_peak_bytes", "allocator_steady_bytes", "file_cache_steady_bytes"]:
            saved = self.runs[0]["metrics"][metric]
            self.runs[0]["metrics"][metric] = None
            self.assertFalse(self.gate()["accepted"], metric)
            self.runs[0]["metrics"][metric] = saved

    def test_unsupported_required_capability_blocks_acceptance(self):
        self.compatibility["capabilities"]["patch"] = False
        self.assertFalse(self.gate()["accepted"])

    def test_one_unfavorable_sample_cannot_be_discarded(self):
        self.runs[1]["metrics"]["rss_peak_bytes"] = 11
        self.assertFalse(self.gate()["accepted"])

    def test_changed_budget_baseline_or_tail_latency_blocks_acceptance(self):
        self.runs[0]["limits"]["memory_limit_mb"] = 32
        self.assertFalse(self.gate()["accepted"])
        self.runs[0]["limits"] = copy.deepcopy(self.config)
        self.provenance[compare.REVISIONS[0]]["source_revision"] = "unknown"
        self.assertFalse(self.gate()["accepted"])

    def test_small_dataset_or_shortened_repetition_is_rejected(self):
        self.config["repetitions"] = 4
        self.assertFalse(self.gate()["accepted"])
        self.config["repetitions"] = 5
        self.config["dataset_bytes"] = 1
        self.assertFalse(self.gate()["accepted"])

    def test_error_bad_checksum_nonfinite_and_duplicate_are_failures(self):
        self.runs[0]["correctness"] = False
        self.runs[1]["metrics"]["throughput"] = float("nan")
        self.runs.append(self.runs[0])
        self.assertFalse(self.gate()["accepted"])

    def test_correctness_failure_alone_blocks_otherwise_valid_evidence(self):
        self.assertTrue(self.gate()["accepted"])
        self.runs[0]["correctness"] = False
        self.assertFalse(self.gate()["accepted"])

    def test_echoed_limits_do_not_substitute_for_applied_mount_budgets(self):
        self.runs[0]["mount_arguments"][2] = "32"
        self.assertFalse(self.gate()["accepted"])

    def test_actual_rust_payload_budget_must_match(self):
        self.runs[1]["effective_budgets"]["payload_limit_bytes"] = 33554432
        self.assertFalse(self.gate()["accepted"])
