#!/usr/bin/env python3
"""Paired Go/Rust runs with raw evidence and a fail-closed acceptance gate.

No global cache drops, forced Go GC or ambient production credentials. Full
accounting requires an explicitly delegated cgroup v2 subtree with cpu+memory
enabled by its owner. This tool never reconfigures the hosting service's cgroup.
"""
import argparse
from collections import deque
from concurrent.futures import ThreadPoolExecutor
import contextlib
import hashlib
import json
import math
import os
from pathlib import Path
import random
import re
import select
import signal
import statistics
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import uuid

from accounting import CpuWindow, cpu_efficiency

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import integration as integration

REVISIONS = [integration.EXPERIMENTAL, integration.UPSTREAM]
REQUIRED_METRICS = ["rss_peak_bytes", "rss_steady_bytes", "cgroup_peak_bytes", "cgroup_steady_bytes",
                    "file_cache_steady_bytes", "kernel_steady_bytes", "allocator_peak_bytes",
                    "allocator_steady_bytes", "throughput", "p50_ms", "p95_ms", "p99_ms",
                    "daemon_cpu_seconds", "client_cpu_seconds", "cgroup_cpu_seconds"]
REQUIRED_COMPATIBILITY = ["linux-cli", "iam", "encryption", "signing", "patch", "permissions-specials",
                          "link-emulation", "list-pages", "metadata", "range-eof", "write-truncate",
                          "rename-delete", "fsync-failure", "full-budget", "mounted-rsync-http", "shutdown"]


class BenchmarkDeadline(TimeoutError):
    pass


def validate(config):
    if config.get("schema") != 1 or config.get("repetitions", 0) < 5:
        raise ValueError("schema 1 and at least five repetitions are required")
    for key in ["memory_limit_mb", "entry_limit", "dataset_bytes", "metadata_entries", "concurrency",
                "random_operations", "random_block_bytes", "writer_bytes", "cpu_quota_us", "cpu_period_us", "cgroup_memory_max_mb"]:
        if not isinstance(config.get(key), int) or config[key] <= 0:
            raise ValueError(f"positive integer required: {key}")
    if config["dataset_bytes"] <= config["memory_limit_mb"] * 1024 * 1024:
        raise ValueError("dataset must exceed unchanged cache budget")
    if set(config.get("states", [])) != {"cold", "warm"}:
        raise ValueError("both cold and warm states are required")
    if set(config.get("workloads", [])) != {"metadata", "sequential", "random", "concurrent", "eviction", "writer_fsync", "writer_failure"}:
        raise ValueError("complete workload matrix is required")


def evaluate(config, runs, compatibility, provenance):
    """Independent evidence validation; absence is failure, never a zero/win."""
    errors, summaries = [], {}
    try:
        validate(config)
    except (ValueError, TypeError) as error:
        return {"accepted": False, "errors": [str(error)], "summaries": {}}
    if compatibility.get("accepted") is not True or compatibility.get("optional_mode_gaps"):
        errors.append("complete independent compatibility acceptance is missing")
    if any(compatibility.get("capabilities", {}).get(name) is not True for name in REQUIRED_COMPATIBILITY):
        errors.append("required capability receipts are incomplete")
    for revision in REVISIONS:
        item = provenance.get(revision, {})
        if item.get("source_revision") != revision or not re.fullmatch(r"[0-9a-f]{64}", item.get("binary_sha256", "")):
            errors.append(f"unverified baseline {revision}")
    if not re.fullmatch(r"[0-9a-f]{64}", provenance.get("rust", {}).get("binary_sha256", "")):
        errors.append("unverified Rust executable")
    if not re.fullmatch(r"[0-9a-f]{40}", provenance.get("rust", {}).get("source_revision", "")):
        errors.append("unverified Rust source revision")
    expected = {(revision, state, workload, repetition, implementation)
                for revision in REVISIONS for state in config["states"] for workload in config["workloads"]
                for repetition in range(config["repetitions"]) for implementation in ["go", "rust"]}
    indexed = {}
    for run in runs:
        key = tuple(run.get(k) for k in ["baseline", "state", "workload", "repetition", "implementation"])
        if key in indexed:
            errors.append(f"duplicate result {key}")
        indexed[key] = run
        if run.get("correctness") is not True or run.get("error"):
            errors.append(f"failed correctness/run {key}")
        if run.get("limits") != config:
            errors.append(f"changed budgets/workloads {key}")
        arguments = run.get("mount_arguments", [])
        for flag, value in [("--memory-limit", config["memory_limit_mb"]), ("--entry-limit", config["entry_limit"])]:
            try:
                if arguments.count(flag) != 1 or arguments[arguments.index(flag) + 1] != str(value):
                    errors.append(f"mount did not receive {flag}={value}: {key}")
            except (IndexError, ValueError):
                errors.append(f"mount argument receipt missing {flag}: {key}")
        if run.get("implementation") == "rust":
            effective = run.get("effective_budgets", {})
            if (effective.get("payload_limit_bytes") != config["memory_limit_mb"] * 1024 * 1024
                    or effective.get("cache_entry_limit") != config["entry_limit"]):
                errors.append(f"effective Rust budgets missing or different: {key}")
        for metric in REQUIRED_METRICS:
            value = run.get("metrics", {}).get(metric)
            if not isinstance(value, (int, float)) or isinstance(value, bool) or not math.isfinite(value) or value < 0:
                errors.append(f"missing/invalid {metric} {key}")
        if run.get("metrics", {}).get("throughput", 0) <= 0:
            errors.append(f"missing throughput {key}")
    if set(indexed) != expected:
        errors.append(f"incomplete matrix: expected {len(expected)}, received {len(indexed)}")
    for revision in REVISIONS:
        for state in config["states"]:
            for workload in config["workloads"]:
                key = (revision, state, workload)
                ratios = {metric: [] for metric in REQUIRED_METRICS}
                for repetition in range(config["repetitions"]):
                    go = indexed.get((*key, repetition, "go"))
                    rust = indexed.get((*key, repetition, "rust"))
                    if not go or not rust:
                        continue
                    if not go.get("data_generation") or go.get("data_generation") != rust.get("data_generation") or not go.get("machine") or go.get("machine") != rust.get("machine"):
                        errors.append(f"non-equivalent data/machine {key}")
                    for metric in REQUIRED_METRICS:
                        left, right = rust.get("metrics", {}).get(metric), go.get("metrics", {}).get(metric)
                        if isinstance(left, (int, float)) and isinstance(right, (int, float)) and math.isfinite(left) and math.isfinite(right) and left >= 0 and right >= 0:
                            if right > 0:
                                ratios[metric].append(left / right)
                            elif metric.endswith("_cpu_seconds") and left == 0:
                                ratios[metric].append(1.0)
                summaries["/".join(key)] = {metric: {"ratios": values, "mean": statistics.mean(values), "variance": statistics.pvariance(values)} for metric, values in ratios.items() if values}
                # Cold runs remain required, byte-correct and fully accounted;
                # comparative performance acceptance concerns sustained warm work.
                if state == "cold":
                    continue
                # Require repeatable improvement, not one selected favorable run.
                for metric in ["rss_peak_bytes", "rss_steady_bytes", "cgroup_peak_bytes", "cgroup_steady_bytes"]:
                    if len(ratios[metric]) != config["repetitions"] or any(ratio >= 1 for ratio in ratios[metric]):
                        errors.append(f"lower memory not established: {key} {metric}")
                if len(ratios["throughput"]) != config["repetitions"] or any(ratio <= 1 for ratio in ratios["throughput"]):
                    errors.append(f"faster operation not established: {key}")
                for metric in ["p50_ms", "p95_ms", "p99_ms"]:
                    if len(ratios[metric]) != config["repetitions"] or statistics.mean(ratios[metric]) > 1:
                        errors.append(f"tail latency non-regression not established: {key} {metric}")
                for metric in ["daemon_cpu_seconds", "cgroup_cpu_seconds"]:
                    if len(ratios[metric]) != config["repetitions"] or statistics.mean(ratios[metric]) > 1:
                        errors.append(f"CPU non-regression not established: {key} {metric}")
    return {"accepted": not errors, "errors": errors, "summaries": summaries}


def build_baseline(reference, revision, binary, work):
    reference = Path(reference).resolve()
    actual = integration.command(["git", "rev-parse", revision + "^{commit}"], cwd=reference, capture_output=True, text=True).stdout.strip()
    if actual != revision:
        raise RuntimeError("baseline source object identity differs")
    source = work / ("source-" + revision)
    source.mkdir()
    with tempfile.TemporaryFile() as archive:
        integration.command(["git", "archive", revision], cwd=reference, stdout=archive, timeout=30)
        archive.seek(0)
        with tarfile.open(fileobj=archive, mode="r|") as stream:
            stream.extractall(source, filter="data")
    integration.command(["go", "build", "-ldflags=-X github.com/yandex-cloud/geesefs/core/cfg.VersionHash=" + revision, "-o", binary, "."], cwd=source, timeout=600)
    # GOTOOLCHAIN=auto may build with a different compiler than the launcher.
    # Read the executable's embedded build information, not ambient `go version`.
    build_info = integration.command(["go", "version", "-m", binary], capture_output=True, text=True).stdout
    first_line = build_info.splitlines()[0] if build_info else ""
    toolchain = first_line.rpartition(": ")[2].strip()
    if not re.fullmatch(r"go\d+\.\d+(?:\.\d+)?(?:(?:beta|rc)\d+)?", toolchain):
        raise RuntimeError("built baseline has no verified embedded Go toolchain")
    return {"source_revision": revision, "binary_sha256": integration.sha_file(binary),
             "toolchain": toolchain}


def cgroup_preflight(root):
    if not root:
        raise RuntimeError("complete accounting requires GEESEFS_BENCH_CGROUP_ROOT or --cgroup-root: an owned, delegated cgroup v2 subtree with cpu+memory enabled")
    root = Path(root).resolve()
    enabled = (root / "cgroup.subtree_control").read_text().split()
    if not {"cpu", "memory"}.issubset(enabled) or not os.access(root, os.W_OK):
        raise RuntimeError("cgroup root must already delegate cpu+memory; refusing to reconfigure the hosting service")
    return root


@contextlib.contextmanager
def cgroup(root, config):
    if root is None:
        yield None
        return
    path = root / ("geesefs-bench-" + uuid.uuid4().hex)
    path.mkdir()
    try:
        (path / "cpu.max").write_text(f"{config['cpu_quota_us']} {config['cpu_period_us']}")
        (path / "memory.max").write_text(str(config["cgroup_memory_max_mb"] * 1024 * 1024))
        yield path
    finally:
        # Only this unique owned cgroup is ever removed. Do not kill/move other
        # processes to force cleanup or enable controllers on an ancestor.
        path.rmdir()


def proc_memory(pid):
    result = {}
    try:
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                result["rss"] = int(line.split()[1]) * 1024
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
            if line.startswith(("Pss:", "Pss_File:")):
                result[line.split(":")[0]] = int(line.split()[1]) * 1024
    except (FileNotFoundError, ProcessLookupError):
        pass
    return result


class Sampler:
    def __init__(self, pid, client_pid, group, trace):
        self.pid, self.client_pid, self.group, self.trace = pid, client_pid, group, trace
        self.done = threading.Event()
        self.recent = deque(maxlen=1000)
        self.peaks = {}
        self.failure = None
        self.thread = threading.Thread(target=self.run)
        self.cpu = CpuWindow({"daemon": pid, "client": client_pid}, group)

    def run(self):
        try:
            with open(self.trace, "w") as output:
                while not self.done.is_set():
                    daemon, client = proc_memory(self.pid), proc_memory(self.client_pid)
                    sample = {"monotonic_ns": time.monotonic_ns(), "daemon": daemon, "client": client,
                              "rss_bytes": daemon.get("rss", 0) + client.get("rss", 0)}
                    if self.group:
                        sample["cgroup_bytes"] = int((self.group / "memory.current").read_text())
                        stats = dict(line.split() for line in (self.group / "memory.stat").read_text().splitlines())
                        sample["file_cache_bytes"] = int(stats["file"])
                        sample["kernel_bytes"] = int(stats["kernel"])
                    output.write(json.dumps(sample) + "\n")
                    output.flush()
                    self.recent.append(sample)
                    for name in ["rss_bytes", "cgroup_bytes"]:
                        if name in sample:
                            self.peaks[name] = max(self.peaks.get(name, 0), sample[name])
                    self.done.wait(0.02)
        except Exception as error:
            self.failure = str(error)

    def __enter__(self):
        self.cpu.start()
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.done.set()
        try:
            self.cpu.finish()
        finally:
            self.thread.join(timeout=10)
        if self.thread.is_alive():
            raise RuntimeError("memory sampler did not stop")
        if self.failure:
            raise RuntimeError("memory accounting failed: " + self.failure)

    def metrics(self):
        recent = list(self.recent)[len(self.recent) // 2:]
        def mean(name):
            values = [item[name] for item in recent if name in item]
            return statistics.mean(values) if values else None
        peak = int((self.group / "memory.peak").read_text()) if self.group else None
        return {"rss_peak_bytes": self.peaks.get("rss_bytes"), "rss_steady_bytes": mean("rss_bytes"),
                "rss_peak_scope": "20ms-sampled-active-workload",
                "cgroup_peak_bytes": peak, "cgroup_steady_bytes": mean("cgroup_bytes"),
                "cgroup_peak_scope": "owned-mount-lifetime-including-startup-and-warmup",
                "file_cache_steady_bytes": mean("file_cache_bytes"), "kernel_steady_bytes": mean("kernel_bytes"),
                **self.cpu.metrics()}


def workload(path, name, config, client, bucket, prefix, endpoint, local, expected_sha):
    latencies, transferred, operations = [], 0, 0
    started = time.perf_counter()
    if name == "metadata":
        count = 0
        with os.scandir(path / "implicit") as entries:
            while True:
                before = time.perf_counter_ns()
                try:
                    entry = next(entries)
                except StopIteration:
                    break
                number = int(entry.name)
                assert entry.stat().st_size == len(f"entry-{number}"), "metadata size"
                count += 1
                latencies.append((time.perf_counter_ns() - before) / 1e6)
        assert count == config["metadata_entries"], "metadata traversal completeness"
        operations = count
    elif name in ["sequential", "eviction"]:
        digest = hashlib.sha256()
        with open(path / "payload", "rb", buffering=0) as stream:
            while True:
                before = time.perf_counter_ns()
                block = stream.read(1024 * 1024)
                if not block:
                    break
                latencies.append((time.perf_counter_ns() - before) / 1e6)
                digest.update(block)
                transferred += len(block)
        assert digest.hexdigest() == expected_sha, "independent full-byte checksum"
        operations = len(latencies)
    elif name in ["random", "concurrent"]:
        def reader(worker):
            generator = random.Random(1729 + worker)
            times = []
            with open(path / "payload", "rb", buffering=0) as stream:
                for _ in range(config["random_operations"]):
                    offset = generator.randrange(config["dataset_bytes"] - config["random_block_bytes"])
                    before = time.perf_counter_ns()
                    block = os.pread(stream.fileno(), config["random_block_bytes"], offset)
                    times.append((time.perf_counter_ns() - before) / 1e6)
                    assert block == integration.expected_range(offset, len(block)) and len(block) == config["random_block_bytes"], "random Range bytes"
            return times
        workers = config["concurrency"] if name == "concurrent" else 1
        with ThreadPoolExecutor(max_workers=workers) as pool:
            for times in pool.map(reader, range(workers)):
                latencies.extend(times)
        operations = len(latencies)
        transferred = operations * config["random_block_bytes"]
    else:
        failing = name == "writer_failure"
        if failing and not local:
            raise RuntimeError("writer failure benchmark needs a controlled disposable fault injector")
        filename = "fault" if failing else "writer-" + uuid.uuid4().hex
        if failing:
            integration.fault(endpoint, "put")
        try:
            digest = hashlib.sha256()
            with open(path / filename, "w+b", buffering=0) as stream:
                remaining = min(config["writer_bytes"], 1024 * 1024) if failing else config["writer_bytes"]
                while remaining:
                    block = integration.expected_range(transferred, min(remaining, 1024 * 1024))
                    before = time.perf_counter_ns()
                    stream.write(block)
                    try:
                        os.fsync(stream.fileno())
                    except OSError:
                        if not failing:
                            raise
                    else:
                        assert not failing, "failed fsync acknowledged durability"
                    latencies.append((time.perf_counter_ns() - before) / 1e6)
                    transferred += len(block)
                    remaining -= len(block)
                    digest.update(block)
                if failing:
                    integration.fault(endpoint)
                    integration.recover_fsync(stream.fileno())
            remote = client.get_object(Bucket=bucket, Key=prefix + "/" + filename)
            actual = hashlib.sha256()
            try:
                while block := remote["Body"].read(1024 * 1024):
                    actual.update(block)
            finally:
                remote["Body"].close()
            assert actual.digest() == digest.digest(), "fresh writer/fsync backend checksum"
            operations = len(latencies)
            os.unlink(path / filename)
        finally:
            if failing:
                integration.fault(endpoint)
    elapsed = time.perf_counter() - started
    ordered = sorted(latencies)
    def percentile(fraction):
        return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]
    return {"throughput": (transferred or operations) / elapsed, "throughput_unit": "bytes/s" if transferred else "ops/s",
            "elapsed_seconds": elapsed, "bytes": transferred, "operations": operations,
            "p50_ms": percentile(0.5), "p95_ms": percentile(0.95), "p99_ms": percentile(0.99),
            "latency_variance_ms2": statistics.pvariance(latencies), "latencies_ms": latencies}


def allocator_go(port, output):
    result = integration.command(["go", "tool", "pprof", "-top", "-unit=bytes", "-sample_index=inuse_space", f"http://127.0.0.1:{port}/debug/pprof/heap"], capture_output=True, text=True, timeout=30)
    output.write_text(result.stdout + result.stderr)
    match = re.search(r"of\s+([0-9.]+)B\s+total", result.stdout)
    return float(match.group(1)) if match else None


def allocator_rust(path, started_ns, finished_ns):
    values = []
    effective = None
    if path.exists():
        with open(path) as stream:
            for line in stream:
                item = json.loads(line)
                if not started_ns <= item.get("unix_time_ns", -1) <= finished_ns:
                    continue
                effective = item.get("effective_budgets")
                if isinstance(item.get("allocated_bytes"), int):
                    values.append(item["allocated_bytes"])
    return (max(values), statistics.mean(values[len(values) // 2:]), effective) if values else (None, None, effective)


def read_message(process, timeout=600):
    if not select.select([process.stdout], [], [], timeout)[0]:
        raise RuntimeError("benchmark worker timed out")
    line = process.stdout.readline()
    if not line:
        raise RuntimeError("benchmark worker exited without a result")
    return json.loads(line)


@contextlib.contextmanager
def worker(spec, environment, group, log):
    with open(log, "wb") as stderr:
        process = subprocess.Popen([sys.executable, str(ROOT / "scripts/bench_worker.py")], env=environment,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True,
                                   preexec_fn=(lambda: integration.enter_cgroup(group)) if group else None)
        try:
            process.stdin.write(json.dumps(spec) + "\n")
            process.stdin.flush()
            if read_message(process, 30) != {"ready": True}:
                raise RuntimeError("benchmark worker failed initialization")
            yield process
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            process.stdout.close()
            if process.returncode and sys.exc_info()[0] is None:
                raise RuntimeError("benchmark worker exited abnormally")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go-reference", default=os.environ.get("GEESEFS_GO_REFERENCE"))
    parser.add_argument("--rust-binary", type=Path, default=ROOT / "target/release/geesefs-rs")
    parser.add_argument("--workloads", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compatibility", type=Path, default=ROOT / "benchmark-results/private/integration/report.json")
    parser.add_argument("--endpoint", default=os.environ.get("GEESEFS_TEST_ENDPOINT"))
    parser.add_argument("--cgroup-root", default=os.environ.get("GEESEFS_BENCH_CGROUP_ROOT"))
    parser.add_argument("--allow-incomplete-accounting", action="store_true", help="diagnostic only; missing accounting cannot pass")
    parser.add_argument("--smoke", action="store_true", help="one pair only, always unaccepted")
    parser.add_argument("--diagnostic-workload", action="append", choices=["metadata", "sequential", "random", "concurrent", "eviction", "writer_fsync", "writer_failure"],
                        help="run only selected workloads without changing limits or full-matrix acceptance; repeatable, always unaccepted")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--timeout", type=int, default=7200, help="overall operation deadline in seconds")
    args = parser.parse_args()
    if args.self_test:
        suite = __import__("unittest").defaultTestLoader.discover(str(ROOT / "bench"), pattern="test_*.py")
        return 0 if __import__("unittest").TextTestRunner().run(suite).wasSuccessful() else 1
    if args.timeout <= 0:
        raise SystemExit("--timeout must be positive")
    args.output = args.output.resolve()
    if args.output.exists() and any(args.output.iterdir()):
        parser.error("output directory is not empty; choose a new path to preserve prior evidence")
    def expired(_signum, _frame):
        raise BenchmarkDeadline("paired benchmark deadline exceeded")
    signal.signal(signal.SIGALRM, expired)
    signal.alarm(args.timeout)
    deadline = time.monotonic() + args.timeout
    args.output.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(args.output, 0o700)
    runs, provenance, errors = [], {}, []
    config, compatibility = {}, {}
    affinity = os.sched_getaffinity(0)
    try:
        config = json.loads(args.workloads.read_text())
        validate(config)
        if not args.go_reference:
            raise RuntimeError("verified --go-reference checkout is required")
        rust_revision = integration.command(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
        if integration.command(["git", "status", "--porcelain"], cwd=ROOT, capture_output=True, text=True).stdout:
            raise RuntimeError("commit the Rust source before comparison; a dirty tree has no exact source provenance")
        try:
            group_root = cgroup_preflight(args.cgroup_root)
        except RuntimeError:
            if not args.allow_incomplete_accounting:
                raise
            group_root = None
        integration.command(["cargo", "build", "--release", "--locked", "-j", "2"], cwd=ROOT, timeout=900)
        built = ROOT / "target/release/geesefs-rs"
        if not args.rust_binary.is_file() or integration.sha_file(args.rust_binary) != integration.sha_file(built):
            raise RuntimeError("Rust executable does not match the current verified release build")
        if args.compatibility.exists():
            compatibility = json.loads(args.compatibility.read_text())
        machine = {"kernel": os.uname().release, "machine": os.uname().machine, "cpu": min(affinity), "boot_id_sha256": hashlib.sha256(Path("/proc/sys/kernel/random/boot_id").read_bytes()).hexdigest()}
        os.sched_setaffinity(0, {min(affinity)})
        with tempfile.TemporaryDirectory(prefix="geesefs-bench-") as temporary:
            work = Path(temporary)
            binaries = {}
            for revision in REVISIONS:
                binaries[revision] = (args.output / ("go-" + revision)).resolve()
                provenance[revision] = build_baseline(args.go_reference, revision, binaries[revision], work)
            provenance["rust"] = {"source_revision": rust_revision, "binary_sha256": integration.sha_file(args.rust_binary), "toolchain": integration.command(["rustc", "--version"], capture_output=True, text=True).stdout.strip()}
            with integration.service(args.endpoint, work) as (client, bucket, endpoint, environment, local):
                prefix = uuid.uuid4().hex
                expected_sha = integration.seed(client, bucket, prefix, config["dataset_bytes"], config["metadata_entries"])
                generation = {"sha256": expected_sha, "object_etag": client.head_object(Bucket=bucket, Key=prefix + "/payload")["ETag"], "bytes": config["dataset_bytes"]}
                stop = False
                for revision in REVISIONS:
                    for state in config["states"]:
                        for name in config["workloads"]:
                            if args.diagnostic_workload and name not in args.diagnostic_workload:
                                continue
                            for repetition in range(config["repetitions"]):
                                order = ["go", "rust"] if repetition % 2 == 0 else ["rust", "go"]
                                for implementation in order:
                                    if time.monotonic() >= deadline:
                                        raise BenchmarkDeadline("paired benchmark deadline exceeded")
                                    label = f"{revision[:8]}-{state}-{name}-{repetition}-{implementation}"
                                    record = {"baseline": revision, "state": state, "workload": name, "repetition": repetition,
                                              "implementation": implementation, "limits": config, "data_generation": generation, "machine": machine, "correctness": False}
                                    try:
                                        binary = binaries[revision] if implementation == "go" else args.rust_binary.resolve()
                                        stats = args.output / (label + "-allocator.jsonl")
                                        pprof = integration.free_port()
                                        extra = ["--pprof", f"127.0.0.1:{pprof}"] if implementation == "go" else ["--stats-file", str(stats.resolve())]
                                        extra += ["--entry-limit", str(config["entry_limit"])]
                                        with cgroup(group_root, config) as group:
                                            with integration.mount(binary, bucket, prefix, endpoint, environment, work, label, config["memory_limit_mb"], extra, group, record) as (path, process):
                                                spec = {"path": str(path), "name": name, "config": config, "bucket": bucket,
                                                        "prefix": prefix, "endpoint": endpoint, "local": local, "expected_sha": expected_sha, "warmup": state == "warm"}
                                                with worker(spec, environment, group, args.output / (label + "-client.log")) as consumer:
                                                    with Sampler(process.pid, consumer.pid, group, args.output / (label + "-memory.jsonl")) as sampler:
                                                        started_ns = time.time_ns()
                                                        consumer.stdin.write("start\n")
                                                        consumer.stdin.flush()
                                                        message = read_message(consumer)
                                                        if message.get("error"):
                                                            raise RuntimeError(message["error"])
                                                        metrics = message["metrics"]
                                                        finished_ns = time.time_ns()
                                                    metrics.update(cpu_efficiency(sampler.metrics(), transferred=metrics["bytes"], operations=metrics["operations"]))
                                                    record["cpu_counters"] = {"before": sampler.cpu.before, "after": sampler.cpu.after}
                                                    record["measurement_window_unix_ns"] = [started_ns, finished_ns]
                                                    if implementation == "go":
                                                        # Keep pprof outside the active timing/memory window.
                                                        allocated = allocator_go(pprof, args.output / (label + "-heap.txt"))
                                                        metrics.update(allocator_peak_bytes=None, allocator_steady_bytes=None,
                                                                       allocator_end_snapshot_bytes=allocated)
                                                    consumer.stdin.write("stop\n")
                                                    consumer.stdin.flush()
                                                record["metrics"] = metrics
                                        if implementation == "rust":
                                            peak, steady, effective = allocator_rust(stats, started_ns, finished_ns)
                                            record["metrics"].update(allocator_peak_bytes=peak, allocator_steady_bytes=steady)
                                            record["effective_budgets"] = effective
                                        record["correctness"] = True
                                    except BenchmarkDeadline:
                                        raise
                                    except Exception as error:
                                        record["error"] = str(error)
                                    finally:
                                        for log in work.glob("*.log"):
                                            if not (args.output / log.name).exists():
                                                __import__("shutil").copyfile(log, args.output / log.name)
                                    runs.append(record)
                                    (args.output / (label + ".json")).write_text(json.dumps(record, indent=2) + "\n")
                                if args.smoke:
                                    stop = True
                                    break
                            if stop:
                                break
                        if stop:
                            break
                    if stop:
                        break
    except Exception as error:
        errors.append(str(error))
    finally:
        signal.alarm(0)
        os.sched_setaffinity(0, affinity)
    gate = evaluate(config, runs, compatibility, provenance)
    gate["errors"] = errors + gate["errors"]
    gate["accepted"] = gate["accepted"] and not errors and not args.smoke and not args.diagnostic_workload and not args.allow_incomplete_accounting
    report = {"gate": gate, "provenance": provenance, "limits": config, "runs": runs,
               "diagnostic_workloads": args.diagnostic_workload or [],
               "accounting_notes": ["RSS includes filesystem and client process", "raw memory traces retain daemon/client PSS separately", "RSS peak is sampled at 20ms; kernel cgroup memory.peak includes startup/warmup", "CPU is user/system counter delta over the active workload, with daemon/client separate and the owned cgroup including both; startup, warmup and end pprof are outside the CPU window", "CPU seconds/GiB or operation and average occupied cores are distinct from wall time; cgroup throttling is recorded without treating it as useful CPU work", "allocator sampling is restricted to the active workload; Go uses a separate end snapshot without gc=1 and cannot pass peak/steady heap acceptance", "cold means a new mount/process; warmup occurs in the owned client cgroup; never a global cache drop"]}
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"accepted": gate["accepted"], "runs": len(runs), "errors": gate["errors"][:10]}, indent=2))
    return 0 if gate["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
