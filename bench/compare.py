#!/usr/bin/env python3
"""Paired Go/Rust runs with raw evidence and a fail-closed acceptance gate.

No global cache drops, forced Go GC or ambient production credentials. Full
accounting requires an explicitly delegated cgroup v2 subtree with cpu+memory
enabled by its owner. This tool never reconfigures the hosting service's cgroup.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import contextlib
import hashlib
from itertools import product
import json
import os
from pathlib import Path
import random
import re
import select
import shutil
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
from go_heap import GoHeapSampler
from metrics import SampledBytes, finite_nonnegative, latency_percentiles, latency_samples

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import integration as integration

REVISIONS = [integration.EXPERIMENTAL, integration.UPSTREAM]
REQUIRED_METRICS = ["rss_peak_bytes", "rss_steady_bytes", "cgroup_peak_bytes", "cgroup_steady_bytes",
                    "daemon_rss_peak_bytes", "daemon_rss_steady_bytes", "client_rss_peak_bytes", "client_rss_steady_bytes",
                    "file_cache_steady_bytes", "kernel_steady_bytes", "allocator_peak_bytes",
                    "allocator_steady_bytes", "throughput", "p50_ms", "p95_ms", "p99_ms",
                    "daemon_cpu_seconds", "client_cpu_seconds", "cgroup_cpu_seconds"]
REQUIRED_COMPATIBILITY = ["linux-cli", "iam", "encryption", "signing", "patch", "permissions-specials",
                          "link-emulation", "list-pages", "metadata", "range-eof", "write-truncate",
                          "rename-delete", "fsync-failure", "full-budget", "mounted-rsync-http", "shutdown"]


class BenchmarkDeadline(TimeoutError):
    pass


def validate(config):
    _validate_schema(config)
    for key in ["memory_limit_mb", "entry_limit", "dataset_bytes", "metadata_entries", "concurrency",
                "random_operations", "random_block_bytes", "writer_bytes", "cpu_quota_us", "cpu_period_us", "cgroup_memory_max_mb"]:
        if type(config.get(key)) is not int or config[key] <= 0:
            raise ValueError(f"positive integer required: {key}")
    if config["dataset_bytes"] <= config["memory_limit_mb"] * 1024 * 1024:
        raise ValueError("dataset must exceed unchanged cache budget")
    if set(config.get("states", [])) != {"cold", "warm"}:
        raise ValueError("both cold and warm states are required")
    if set(config.get("workloads", [])) != {"metadata", "sequential", "random", "concurrent", "eviction", "writer_fsync", "writer_failure"}:
        raise ValueError("complete workload matrix is required")


def _validate_schema(config):
    if type(config.get("schema")) is not int or config["schema"] != 1:
        raise ValueError("schema 1 and at least five repetitions are required")
    if type(config.get("repetitions")) is not int or config["repetitions"] < 5:
        raise ValueError("schema 1 and at least five repetitions are required")


def _compatibility_errors(compatibility):
    errors = []
    if compatibility.get("accepted") is not True or compatibility.get("optional_mode_gaps"):
        errors.append("complete independent compatibility acceptance is missing")
    if any(compatibility.get("capabilities", {}).get(name) is not True for name in REQUIRED_COMPATIBILITY):
        errors.append("required capability receipts are incomplete")
    return errors


def _provenance_errors(provenance):
    errors = []
    for revision in REVISIONS:
        item = provenance.get(revision, {})
        if item.get("source_revision") != revision or not re.fullmatch(r"[0-9a-f]{64}", item.get("binary_sha256", "")):
            errors.append(f"unverified baseline {revision}")
    if not re.fullmatch(r"[0-9a-f]{64}", provenance.get("rust", {}).get("binary_sha256", "")):
        errors.append("unverified Rust executable")
    if not re.fullmatch(r"[0-9a-f]{40}", provenance.get("rust", {}).get("source_revision", "")):
        errors.append("unverified Rust source revision")
    return errors


def _mount_budget_errors(config, run, key):
    errors = []
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
    return errors


def _run_errors(config, run, key):
    errors = _mount_budget_errors(config, run, key)
    if run.get("correctness") is not True or run.get("error"):
        errors.append(f"failed correctness/run {key}")
    if run.get("limits") != config:
        errors.append(f"changed budgets/workloads {key}")
    metrics = run.get("metrics", {})
    for metric in REQUIRED_METRICS:
        if not finite_nonnegative(metrics.get(metric)):
            errors.append(f"missing/invalid {metric} {key}")
    if metrics.get("throughput") == 0:
        errors.append(f"missing throughput {key}")
    if latency_samples(metrics) is None:
        errors.append(f"missing/invalid raw latency samples {key}")
    return errors


def _metric_ratio(left, right):
    if not finite_nonnegative(left) or not finite_nonnegative(right):
        return None
    if right > 0:
        return left / right
    return 1.0 if left == 0 else None


def _include_pair(ratios, pooled, go, rust):
    for metric in REQUIRED_METRICS:
        ratio = _metric_ratio(rust.get("metrics", {}).get(metric), go.get("metrics", {}).get(metric))
        if ratio is not None:
            ratios[metric].append(ratio)
    for implementation, record in [("go", go), ("rust", rust)]:
        values = latency_samples(record.get("metrics", {}))
        if values is not None:
            pooled[implementation].extend(values)


def _equivalent_pair(go, rust):
    return (bool(go.get("data_generation"))
            and go.get("data_generation") == rust.get("data_generation")
            and bool(go.get("machine")) and go.get("machine") == rust.get("machine"))


def _ratio_summary(ratios):
    return {metric: {"ratios": values, "mean": statistics.mean(values), "variance": statistics.pvariance(values)}
            for metric, values in ratios.items() if values}


def _group_summary(config, indexed, key):
    ratios = {metric: [] for metric in REQUIRED_METRICS}
    pooled = {"go": [], "rust": []}
    errors = []
    for repetition in range(config["repetitions"]):
        go = indexed.get((*key, repetition, "go"))
        rust = indexed.get((*key, repetition, "rust"))
        if not go or not rust:
            continue
        if not _equivalent_pair(go, rust):
            errors.append(f"non-equivalent data/machine {key}")
        _include_pair(ratios, pooled, go, rust)
    summary = _ratio_summary(ratios)
    summary["pooled_latency_ms"] = {implementation: latency_percentiles(values)
                                    for implementation, values in pooled.items() if values}
    return summary, errors


def _strict_gain(values, count, higher=False):
    return len(values) == count and all(value > 1 if higher else value < 1 for value in values)


def _mean_non_regression(values, count):
    return len(values) == count and statistics.mean(values) <= 1


def _tail_errors(key, summary, repetitions):
    errors = []
    tails = summary["pooled_latency_ms"]
    for metric in ["p50_ms", "p95_ms", "p99_ms"]:
        if not _mean_non_regression(summary.get(metric, {}).get("ratios", []), repetitions):
            errors.append(f"tail latency non-regression not established: {key} {metric}")
        if len(tails) != 2 or tails["rust"][metric] > tails["go"][metric]:
            errors.append(f"pooled tail latency non-regression not established: {key} {metric}")
    return errors


def _warm_group_errors(key, summary, repetitions):
    errors = _tail_errors(key, summary, repetitions)
    for metric in ["rss_peak_bytes", "rss_steady_bytes", "daemon_rss_peak_bytes", "daemon_rss_steady_bytes",
                   "cgroup_peak_bytes", "cgroup_steady_bytes", "allocator_peak_bytes", "allocator_steady_bytes"]:
        if not _strict_gain(summary.get(metric, {}).get("ratios", []), repetitions):
            errors.append(f"lower memory not established: {key} {metric}")
    if not _strict_gain(summary.get("throughput", {}).get("ratios", []), repetitions, higher=True):
        errors.append(f"faster operation not established: {key}")
    for message, metrics in [("memory component", ["file_cache_steady_bytes", "kernel_steady_bytes"]),
                             ("CPU", ["daemon_cpu_seconds", "cgroup_cpu_seconds"])]:
        for metric in metrics:
            if not _mean_non_regression(summary.get(metric, {}).get("ratios", []), repetitions):
                errors.append(f"{message} non-regression not established: {key} {metric}")
    return errors


def evaluate(config, runs, compatibility, provenance):
    """Independent evidence validation; absence is failure, never a zero/win."""
    try:
        validate(config)
    except (ValueError, TypeError) as error:
        return {"accepted": False, "errors": [str(error)], "summaries": {}}
    errors = _compatibility_errors(compatibility) + _provenance_errors(provenance)
    expected = set(product(REVISIONS, config["states"], config["workloads"],
                           range(config["repetitions"]), ["go", "rust"]))
    indexed, summaries = {}, {}
    for run in runs:
        key = tuple(run.get(k) for k in ["baseline", "state", "workload", "repetition", "implementation"])
        if key in indexed:
            errors.append(f"duplicate result {key}")
        indexed[key] = run
        errors.extend(_run_errors(config, run, key))
    if set(indexed) != expected:
        errors.append(f"incomplete matrix: expected {len(expected)}, received {len(indexed)}")
    for key in product(REVISIONS, config["states"], config["workloads"]):
        summary, group_errors = _group_summary(config, indexed, key)
        summaries["/".join(key)] = summary
        errors.extend(group_errors)
        # Cold work is still complete and correct; comparisons concern sustained
        # warm operation, not initial mount/listing speed.
        if key[1] == "warm":
            errors.extend(_warm_group_errors(key, summary, config["repetitions"]))
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
        self.failure = None
        self.thread = threading.Thread(target=self.run)
        self.cpu = CpuWindow({"daemon": pid, "client": client_pid}, group)

    def run(self):
        try:
            with open(self.trace, "w") as output:
                while not self.done.is_set():
                    daemon, client = proc_memory(self.pid), proc_memory(self.client_pid)
                    sample = {"monotonic_ns": time.monotonic_ns(), "daemon": daemon, "client": client}
                    if 'rss' in daemon and 'rss' in client:
                        sample['rss_bytes'] = daemon['rss'] + client['rss']
                    if self.group:
                        sample["cgroup_bytes"] = int((self.group / "memory.current").read_text())
                        stats = dict(line.split() for line in (self.group / "memory.stat").read_text().splitlines())
                        sample["file_cache_bytes"] = int(stats["file"])
                        sample["kernel_bytes"] = int(stats["kernel"])
                    output.write(json.dumps(sample) + "\n")
                    output.flush()
                    self.done.wait(0.02)
        except Exception as error:
            self.failure = str(error)

    def __enter__(self):
        self.started_ns = time.monotonic_ns()
        self.cpu.start()
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.finished_ns = time.monotonic_ns()
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
        # Stream the complete trace in constant memory; a bounded tail cannot
        # represent the temporal latter half of an arbitrarily long mount.
        fields = ['rss_bytes', 'daemon_rss_bytes', 'client_rss_bytes',
                  'cgroup_bytes', 'file_cache_bytes', 'kernel_bytes']
        series = {name: SampledBytes() for name in fields}
        midpoint = (self.started_ns + self.finished_ns) // 2
        with open(self.trace) as source:
            for line in source:
                sample = json.loads(line)
                at = sample['monotonic_ns']
                if not self.started_ns <= at <= self.finished_ns:
                    continue
                sample['daemon_rss_bytes'] = sample['daemon'].get('rss')
                sample['client_rss_bytes'] = sample['client'].get('rss')
                for name in fields:
                    series[name].record(sample.get(name), late=at >= midpoint)

        cgroup_peak = int((self.group / "memory.peak").read_text()) if self.group else None
        return {"rss_peak_bytes": series['rss_bytes'].peak,
                "rss_steady_bytes": series['rss_bytes'].late_mean,
                "rss_peak_scope": "20ms-sampled-active-workload",
                'rss_steady_scope': 'sample-mean-in-temporal-latter-half-of-active-window-not-a-plateau',
                'daemon_rss_peak_bytes': series['daemon_rss_bytes'].peak,
                'daemon_rss_steady_bytes': series['daemon_rss_bytes'].late_mean,
                'client_rss_peak_bytes': series['client_rss_bytes'].peak,
                'client_rss_steady_bytes': series['client_rss_bytes'].late_mean,
                "cgroup_peak_bytes": cgroup_peak, "cgroup_steady_bytes": series['cgroup_bytes'].late_mean,
                "cgroup_peak_scope": "owned-mount-lifetime-including-startup-and-warmup",
                "file_cache_steady_bytes": series['file_cache_bytes'].late_mean,
                "kernel_steady_bytes": series['kernel_bytes'].late_mean,
                **self.cpu.metrics()}


def _metadata(path, config):
    latencies = []
    with os.scandir(path / "implicit") as entries:
        while True:
            before = time.perf_counter_ns()
            try:
                entry = next(entries)
            except StopIteration:
                break
            number = int(entry.name)
            assert entry.stat().st_size == len(f"entry-{number}"), "metadata size"
            latencies.append((time.perf_counter_ns() - before) / 1e6)
    assert len(latencies) == config["metadata_entries"], "metadata traversal completeness"
    return latencies, 0


def _sequential(path, expected_sha):
    latencies, transferred = [], 0
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
    return latencies, transferred


def _random_reader(path, config, worker):
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


def _random(path, name, config):
    latencies = []
    workers = config["concurrency"] if name == "concurrent" else 1
    with ThreadPoolExecutor(max_workers=workers) as pool:
        for times in pool.map(lambda worker: _random_reader(path, config, worker), range(workers)):
            latencies.extend(times)
    return latencies, len(latencies) * config["random_block_bytes"]


def _sync_write(stream, block, failing):
    stream.write(block)
    try:
        os.fsync(stream.fileno())
    except OSError:
        if not failing:
            raise
    else:
        assert not failing, "failed fsync acknowledged durability"


def _write_batches(stream, size, failing):
    digest = hashlib.sha256()
    latencies, transferred = [], 0
    remaining = min(size, 1024 * 1024) if failing else size
    while remaining:
        block = integration.expected_range(transferred, min(remaining, 1024 * 1024))
        before = time.perf_counter_ns()
        _sync_write(stream, block, failing)
        latencies.append((time.perf_counter_ns() - before) / 1e6)
        transferred += len(block)
        remaining -= len(block)
        digest.update(block)
    return latencies, transferred, digest.digest()


def _check_remote(client, bucket, key, expected):
    remote = client.get_object(Bucket=bucket, Key=key)
    actual = hashlib.sha256()
    try:
        while block := remote["Body"].read(1024 * 1024):
            actual.update(block)
    finally:
        remote["Body"].close()
    assert actual.digest() == expected, "fresh writer/fsync backend checksum"


def _writer(path, name, config, client, bucket, prefix, endpoint, local):
    failing = name == "writer_failure"
    if failing and not local:
        raise RuntimeError("writer failure benchmark needs a controlled disposable fault injector")
    filename = "fault" if failing else "writer-" + uuid.uuid4().hex
    if failing:
        integration.fault(endpoint, "put")
    try:
        with open(path / filename, "w+b", buffering=0) as stream:
            latencies, transferred, digest = _write_batches(stream, config["writer_bytes"], failing)
            if failing:
                integration.fault(endpoint)
                integration.recover_fsync(stream.fileno())
        _check_remote(client, bucket, prefix + "/" + filename, digest)
        os.unlink(path / filename)
        return latencies, transferred
    finally:
        if failing:
            integration.fault(endpoint)


def workload(path, name, config, client, bucket, prefix, endpoint, local, expected_sha):
    started = time.perf_counter()
    if name == "metadata":
        latencies, transferred = _metadata(path, config)
    elif name in ["sequential", "eviction"]:
        latencies, transferred = _sequential(path, expected_sha)
    elif name in ["random", "concurrent"]:
        latencies, transferred = _random(path, name, config)
    else:
        latencies, transferred = _writer(path, name, config, client, bucket, prefix, endpoint, local)
    operations = len(latencies)
    elapsed = time.perf_counter() - started
    return {"throughput": (transferred or operations) / elapsed, "throughput_unit": "bytes/s" if transferred else "ops/s",
            "elapsed_seconds": elapsed, "bytes": transferred, "operations": operations,
            **latency_percentiles(latencies),
            "latency_variance_ms2": statistics.pvariance(latencies), "latencies_ms": latencies}


def allocator_rust(path, started_ns, finished_ns):
    series = SampledBytes()
    effective = None
    if path.exists():
        with open(path) as stream:
            for line in stream:
                item = json.loads(line)
                if not started_ns <= item.get("unix_time_ns", -1) <= finished_ns:
                    continue
                effective = item.get("effective_budgets")
                value = item.get('allocated_bytes')
                series.record(value if type(value) is int else None,
                              late=item['unix_time_ns'] >= (started_ns + finished_ns) // 2)
    return series.peak, series.late_mean if series.complete else None, effective


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
            if read_message(process, 600 if spec.get("warmup") else 30) != {"ready": True}:
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


def _arguments():
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
    return parser, parser.parse_args()


def _prepare_output(parser, args):
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
    return deadline


def _verified_source(args):
    if not args.go_reference:
        raise RuntimeError("verified --go-reference checkout is required")
    revision = integration.command(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    if integration.command(["git", "status", "--porcelain"], cwd=ROOT, capture_output=True, text=True).stdout:
        raise RuntimeError("commit the Rust source before comparison; a dirty tree has no exact source provenance")
    return revision


def _accounting_root(args):
    try:
        return cgroup_preflight(args.cgroup_root)
    except RuntimeError:
        if not args.allow_incomplete_accounting:
            raise
        return None


def _verify_binary(args):
    integration.command(["cargo", "build", "--release", "--locked", "-j", "2"], cwd=ROOT, timeout=900)
    built = ROOT / "target/release/geesefs-rs"
    if not args.rust_binary.is_file() or integration.sha_file(args.rust_binary) != integration.sha_file(built):
        raise RuntimeError("Rust executable does not match the current verified release build")


def _cases(config, selected, smoke):
    for revision, state, name, repetition in product(REVISIONS, config["states"], config["workloads"], range(config["repetitions"])):
        if selected and name not in selected:
            continue
        order = ["go", "rust"] if repetition % 2 == 0 else ["rust", "go"]
        for implementation in order:
            yield dict(baseline=revision, state=state, workload=name, repetition=repetition, implementation=implementation)
        if smoke:
            return


def _measure_run(args, group_root, binaries, work, fixture, record):
    config, implementation = record["limits"], record["implementation"]
    label = _label(record)
    binary = binaries[record["baseline"]] if implementation == "go" else args.rust_binary.resolve()
    stats = args.output / (label + "-allocator.jsonl")
    pprof = integration.free_port()
    extra = ["--pprof", f"127.0.0.1:{pprof}"] if implementation == "go" else ["--stats-file", str(stats.resolve())]
    extra += ["--entry-limit", str(config["entry_limit"])]
    environment = fixture["environment"]
    with cgroup(group_root, config) as group:
        with integration.mount(binary, fixture["bucket"], fixture["prefix"], fixture["endpoint"], environment,
                               work, label, config["memory_limit_mb"], extra, group, record) as (path, process):
            spec = {key: fixture[key] for key in ["bucket", "prefix", "endpoint", "local", "expected_sha"]}
            spec.update(path=str(path), name=record["workload"], config=config, warmup=record["state"] == "warm")
            with worker(spec, environment, group, args.output / (label + "-client.log")) as consumer:
                heap_observer = GoHeapSampler(pprof, args.output / (label + "-heap.jsonl")) if implementation == "go" else contextlib.nullcontext()
                with heap_observer as heap:
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
                    if heap is not None:
                        metrics.update(heap.metrics(started_ns, finished_ns))
                    consumer.stdin.write("stop\n")
                    consumer.stdin.flush()
            record["metrics"] = metrics
    if implementation == "rust":
        peak, steady, effective = allocator_rust(stats, started_ns, finished_ns)
        record["metrics"].update(allocator_peak_bytes=peak, allocator_steady_bytes=steady)
        record["effective_budgets"] = effective
    record["correctness"] = True


def _label(record):
    return f"{record['baseline'][:8]}-{record['state']}-{record['workload']}-{record['repetition']}-{record['implementation']}"


def _run_case(args, group_root, binaries, work, fixture, record):
    try:
        _measure_run(args, group_root, binaries, work, fixture, record)
    except BenchmarkDeadline:
        raise
    except Exception as error:
        record["error"] = str(error)
    finally:
        for log in work.glob("*.log"):
            if not (args.output / log.name).exists():
                shutil.copyfile(log, args.output / log.name)


def _run_matrix(args, group_root, binaries, work, fixture, common, runs, deadline):
    for case in _cases(common["limits"], args.diagnostic_workload, args.smoke):
        if time.monotonic() >= deadline:
            raise BenchmarkDeadline("paired benchmark deadline exceeded")
        record = dict(common, **case, correctness=False)
        _run_case(args, group_root, binaries, work, fixture, record)
        runs.append(record)
        (args.output / (_label(record) + ".json")).write_text(json.dumps(record, indent=2) + "\n")


def _comparisons(args, config, group_root, provenance, runs, revision, affinity, deadline):
    machine = {"kernel": os.uname().release, "machine": os.uname().machine, "cpu": min(affinity), "boot_id_sha256": hashlib.sha256(Path("/proc/sys/kernel/random/boot_id").read_bytes()).hexdigest()}
    os.sched_setaffinity(0, {min(affinity)})
    with tempfile.TemporaryDirectory(prefix="geesefs-bench-") as temporary:
        work = Path(temporary)
        binaries = {}
        for baseline in REVISIONS:
            binaries[baseline] = (args.output / ("go-" + baseline)).resolve()
            provenance[baseline] = build_baseline(args.go_reference, baseline, binaries[baseline], work)
        provenance["rust"] = {"source_revision": revision, "binary_sha256": integration.sha_file(args.rust_binary), "toolchain": integration.command(["rustc", "--version"], capture_output=True, text=True).stdout.strip()}
        with integration.service(args.endpoint, work) as (client, bucket, endpoint, environment, local):
            prefix = uuid.uuid4().hex
            expected_sha = integration.seed(client, bucket, prefix, config["dataset_bytes"], config["metadata_entries"])
            generation = {"sha256": expected_sha, "object_etag": client.head_object(Bucket=bucket, Key=prefix + "/payload")["ETag"], "bytes": config["dataset_bytes"]}
            fixture = dict(bucket=bucket, prefix=prefix, endpoint=endpoint, environment=environment, local=local, expected_sha=expected_sha)
            common = dict(limits=config, data_generation=generation, machine=machine)
            _run_matrix(args, group_root, binaries, work, fixture, common, runs, deadline)


def _report(args, config, runs, compatibility, provenance, errors):
    gate = evaluate(config, runs, compatibility, provenance)
    gate["errors"] = errors + gate["errors"]
    gate["accepted"] = gate["accepted"] and not errors and not args.smoke and not args.diagnostic_workload and not args.allow_incomplete_accounting
    report = {"gate": gate, "provenance": provenance, "limits": config, "runs": runs,
               "diagnostic_workloads": args.diagnostic_workload or [],
               "accounting_notes": ["RSS includes filesystem and client process", "raw memory traces retain daemon/client PSS separately", "RSS peak is sampled at 20ms; kernel cgroup memory.peak includes startup/warmup", "CPU is user/system counter delta over the active workload, with daemon/client separate and the owned cgroup including both; startup and warmup are outside the CPU window", "CPU seconds/GiB or operation and average occupied cores are distinct from wall time; cgroup throttling is recorded without treating it as useful CPU work", "Go HeapAlloc is sampled at 1s via bounded pprof debug=1 requests without forced GC; request durations and raw MemStats are retained, server observer overhead remains charged to Go; fewer than two fully in-window samples are insufficient", "allocator peaks are sampled; late-half means are not proof of a memory plateau", "cold means a new mount/process; warmup occurs in the owned client cgroup; never a global cache drop"]}
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"accepted": gate["accepted"], "runs": len(runs), "errors": gate["errors"][:10]}, indent=2))
    return 0 if gate["accepted"] else 1


def main():
    parser, args = _arguments()
    if args.self_test:
        suite = __import__("unittest").defaultTestLoader.discover(str(ROOT / "bench"), pattern="test_*.py")
        return 0 if __import__("unittest").TextTestRunner().run(suite).wasSuccessful() else 1
    deadline = _prepare_output(parser, args)
    affinity = os.sched_getaffinity(0)
    runs, provenance, errors = [], {}, []
    config, compatibility = {}, {}
    try:
        config = json.loads(args.workloads.read_text())
        validate(config)
        revision = _verified_source(args)
        group_root = _accounting_root(args)
        _verify_binary(args)
        if args.compatibility.exists():
            compatibility = json.loads(args.compatibility.read_text())
        _comparisons(args, config, group_root, provenance, runs, revision, affinity, deadline)
    except Exception as error:
        errors.append(str(error))
    finally:
        signal.alarm(0)
        os.sched_setaffinity(0, affinity)
    return _report(args, config, runs, compatibility, provenance, errors)


if __name__ == "__main__":
    raise SystemExit(main())
