"""Finite correctness-checked FUSE reads against an already sealed native fixture.

Run in an owned two-container test Pod. The S3 backend has its own CPU/memory
cgroup; no production credentials, global cache drops or S3 mutations are used.
"""
import array
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import resource
import signal
import statistics
import subprocess
import sys
import threading
import time
import urllib.request

import psutil

from accounting import CpuWindow, cpu_efficiency
from metadata_roundtrip import metadata_roundtrip
from sealed_s3_fixture import BUCKET, PREFIX, READER, client, list_names, verify

WORK = Path("/work")
SHARED = Path("/shared")
ENDPOINT = "http://127.0.0.1:8333"
CHUNK = 128 * 1024
BINARIES = {"go-experimental": "geesefs-go-experimental", "rust": "geesefs-rs",
            "go-packaged": "geesefs-go-packaged", "rust-pre-coherence": "geesefs-rs-pre-coherence"}
START = time.monotonic()
DEADLINE = START + 10500


def save(name, value):
    with (WORK / name).open("x") as output:
        json.dump(value, output, indent=2)
        output.write("\n")


def emit(value):
    print(json.dumps(value), flush=True)


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            result.update(block)
    return result.hexdigest()


def group(pid):
    relative = Path(f"/proc/{pid}/cgroup").read_text().strip().split("::", 1)[1].lstrip("/")
    result = Path("/sys/fs/cgroup") / relative
    assert (result / "cpu.stat").is_file() and (result / "memory.current").is_file(), relative
    return result


def percentile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


def percentiles(values):
    return {str(p): percentile(values, p) for p in (0.5, 0.95, 0.99)}


def summarize_memory(samples):
    if not samples:
        raise RuntimeError("memory sampler produced no observations")
    steady = samples[-max(1, len(samples) // 5):]
    keys = sorted({key for sample in samples for key in sample if key not in ("monotonic_ns", "unix_ns")})
    def values(rows, key):
        return [row[key] for row in rows if row.get(key) is not None]
    return {"samples": len(samples), "interval_seconds": 0.05,
            "peak": {key: max(v) if (v := values(samples, key)) else None for key in keys},
            "steady": {key: statistics.mean(v) if (v := values(steady, key)) else None for key in keys},
            "steady_scope": "mean of available samples in final fifth; not proof of a stationary plateau",
            "rss_scope": "daemon/client individually and pointwise combined; controller/backend separately; shared RSS pages can be counted twice"}


def metrics_snapshot(label, boundary):
    try:
        with urllib.request.urlopen("http://127.0.0.1:9328/metrics", timeout=10) as response:
            body = response.read(8 * 1024 * 1024)
        (WORK / f"{label}-{boundary}-backend-metrics.txt").write_bytes(body)
    except Exception as error:
        save(f"{label}-{boundary}-backend-metrics-error.json", {"error": str(error)})


def cpu_pair(daemon, backend):
    own, remote = group(os.getpid()), group(backend)
    assert own != remote, "backend must not contaminate the client cgroup"
    return (CpuWindow({"daemon": daemon, "client": os.getpid()}, own).start(),
            CpuWindow({"backend": backend}, remote).start())


def finish_pair(windows, transferred, operations):
    for window in windows:
        window.finish()
    return {"filesystem": cpu_efficiency(windows[0].metrics(), transferred=transferred, operations=operations),
            "backend": windows[1].metrics()}


def client_phase(mount, label, daemon, backend, *, roundtrip=False):
    signal.alarm(1700)
    manifest = json.loads((SHARED / "manifest.json").read_text())
    listings = json.loads((SHARED / "listings.json").read_text())
    root = Path(mount)
    workload = cpu_pair(daemon, backend)
    listing = cpu_pair(daemon, backend)
    errors, list_times, stat_times, file_times = [], [], [], []
    read_times = array.array("d")
    begun = time.monotonic()
    for entry in listings:
        before = time.monotonic()
        try:
            names = sorted(item.name for item in os.scandir(root / entry["path"]))
            if names != entry["names"]:
                errors.append({"listing": entry["path"], "missing": sorted(set(entry["names"]) - set(names)),
                               "extra": sorted(set(names) - set(entry["names"]))})
        except OSError as error:
            errors.append({"listing": entry["path"], "error": str(error)})
        list_times.append((time.monotonic() - before) * 1000)
    listing_seconds = time.monotonic() - begun
    listing_cpu = finish_pair(listing, 0, len(listings))
    metadata = None
    if roundtrip:
        windows = cpu_pair(daemon, backend)
        metadata = metadata_roundtrip(root, manifest)
        metadata["cpu"] = finish_pair(windows, 0, len(manifest))
        (WORK / (label + "-forced-stat.txt")).write_text(metadata.pop("stat_stdout"))
        errors.extend(metadata["errors"])

    def read(entry):
        before = time.monotonic()
        count, checksum = 0, hashlib.sha256()
        latencies = array.array("d")
        result = {"path": entry["path"], "expected_size": entry["size"]}
        try:
            stat_at = time.monotonic()
            info = (root / entry["path"]).stat()
            result["stat_ms"] = (time.monotonic() - stat_at) * 1000
            observed = (info.st_size, info.st_mode & 0o7777, int(info.st_mtime), info.st_uid, info.st_gid)
            expected = tuple(entry[key] for key in ("size", "mode", "mtime", "uid", "gid"))
            if observed != expected:
                raise RuntimeError(f"metadata {observed!r} differs from independent fixture {expected!r}")
            with (root / entry["path"]).open("rb", buffering=0) as source:
                while True:
                    at = time.monotonic()
                    block = source.read(CHUNK)
                    latencies.append((time.monotonic() - at) * 1000)
                    if not block:
                        break
                    checksum.update(block)
                    count += len(block)
            if count != entry["size"] or checksum.hexdigest() != entry["sha256"]:
                raise RuntimeError("full payload does not match independently generated SHA-256")
        except Exception as error:
            result["error"] = str(error)
        result.update(bytes=count, sha256=checksum.hexdigest(), elapsed_ms=(time.monotonic() - before) * 1000)
        return result, latencies

    reads = cpu_pair(daemon, backend)
    begun = time.monotonic()
    total, good = 0, 0
    with (WORK / f"{label}-files.jsonl").open("x") as journal, ThreadPoolExecutor(max_workers=4) as pool:
        futures = [pool.submit(read, entry) for entry in manifest]
        for number, future in enumerate(as_completed(futures), 1):
            result, latencies = future.result()
            journal.write(json.dumps(result) + "\n")
            read_times.extend(latencies)
            file_times.append(result["elapsed_ms"])
            if "stat_ms" in result:
                stat_times.append(result["stat_ms"])
            total += result["bytes"]
            if "error" in result:
                errors.append(result)
            else:
                good += 1
            if number % 250 == 0:
                journal.flush()
                emit({"label": label, "files": number, "correct": good, "bytes": total})
    elapsed = time.monotonic() - begun
    read_cpu = finish_pair(reads, total, good)
    workload_cpu = finish_pair(workload, total, good)
    result = {"label": label, "files": len(manifest), "correct_files": good, "bytes": total,
              "errors": errors, "elapsed_seconds": elapsed, "throughput_bytes_second": total / elapsed,
              "listing_seconds": listing_seconds, "listings": len(listings),
              "read_syscall_ms": percentiles(read_times), "file_total_ms": percentiles(file_times),
              "stat_ms": percentiles(stat_times), "listing_ms": percentiles(list_times),
              "read_cpu": read_cpu, "listing_cpu": listing_cpu, "metadata_roundtrip": metadata,
              "workload_cpu": workload_cpu}
    save(label + "-cpu-counters.json", {name: {role: {"before": window.before, "after": window.after}
         for role, window in zip(("filesystem", "backend"), pair)}
         for name, pair in (("read", reads), ("listing", listing), ("workload", workload))})
    save(label + "-client.json", result)


class Sampler:
    def __init__(self, label, daemon, backend):
        self.label = label
        self.processes = {"daemon": psutil.Process(daemon), "controller": psutil.Process(os.getpid()),
                          "backend": psutil.Process(backend)}
        self.client = None
        self.groups = {"cgroup": group(os.getpid()), "backend_cgroup": group(backend)}
        self.stop = threading.Event()
        self.samples = []
        self.error = None
        self.thread = threading.Thread(target=self.run)

    def run(self):
        try:
            self.sample()
        except Exception as error:
            self.error = error

    def sample(self):
        with (WORK / (self.label + "-memory.jsonl")).open("x") as journal:
            while not self.stop.is_set():
                sample = {"monotonic_ns": time.monotonic_ns(), "unix_ns": time.time_ns()}
                processes = dict(self.processes)
                if self.client is not None:
                    processes["client"] = self.client
                for role, process in processes.items():
                    try:
                        sample[role + "_rss"] = process.memory_info().rss
                    except psutil.NoSuchProcess:
                        sample[role + "_rss"] = None
                    if len(self.samples) % 20 == 0:
                        try:
                            sample[role + "_pss"] = process.memory_full_info().pss
                        except psutil.NoSuchProcess:
                            sample[role + "_pss"] = None
                for role, path in self.groups.items():
                    stat = dict(line.split() for line in (path / "memory.stat").read_text().splitlines())
                    sample[role + "_current"] = int((path / "memory.current").read_text())
                    sample[role + "_lifetime_peak"] = int((path / "memory.peak").read_text())
                    for name in ("anon", "file", "kernel"):
                        sample[role + "_" + name] = int(stat[name])
                if sample.get("client_rss") is not None and sample.get("daemon_rss") is not None:
                    sample["daemon_client_rss"] = sample["daemon_rss"] + sample["client_rss"]
                self.samples.append(sample)
                journal.write(json.dumps(sample) + "\n")
                self.stop.wait(0.05)

    def finish(self):
        self.stop.set()
        self.thread.join(timeout=5)
        assert not self.thread.is_alive()
        if self.error is not None:
            raise RuntimeError("memory sampler failed; phase cannot be accepted") from self.error
        return summarize_memory(self.samples)


def environment():
    result = {key: value for key, value in os.environ.items()
              if not key.startswith("AWS_") and key not in ("GOMEMLIMIT", "GOGC")}
    result.update(AWS_ACCESS_KEY_ID=READER[0], AWS_SECRET_ACCESS_KEY=READER[1],
                  AWS_REGION="us-east-1", AWS_DEFAULT_REGION="us-east-1", AWS_EC2_METADATA_DISABLED="true",
                  AWS_CONFIG_FILE="/dev/null", AWS_SHARED_CREDENTIALS_FILE="/dev/null",
                  GOMAXPROCS="2", GOGC="100", TOKIO_WORKER_THREADS="2")
    return result


def mount_run(implementation, repetition, backend):
    label = f"{repetition}-{implementation}"
    mount = WORK / (label + "-mount")
    mount.mkdir()
    args = [str(WORK / BINARIES[implementation]), "-f", "-o", "ro", "--no-expire-multipart", "--no-detect",
            "--endpoint", ENDPOINT, "--region", "us-east-1", "--list-type", "2", "--memory-limit", "512",
            "--entry-limit", "100000", "--enable-mtime", "--enable-perms", "--http-timeout", "30s"]
    if implementation.startswith("rust"):
        args += ["--request-timeout", "60s", "--stats-file", str(WORK / (label + "-allocator.jsonl"))]
    else:
        args += ["--gc-interval", "0", "--pprof", "127.0.0.1:6060"]
    args += [BUCKET + ":" + PREFIX.rstrip("/"), str(mount)]
    results = []
    with (WORK / (label + "-mount.log")).open("xb") as log:
        process = subprocess.Popen(args, env=environment(), stdout=log, stderr=log)
        try:
            until = time.monotonic() + 45
            while not os.path.ismount(mount):
                if process.poll() is not None or time.monotonic() >= until:
                    raise RuntimeError(f"mount {label} not ready; exit={process.poll()}")
                time.sleep(0.1)
            for state in ("fresh", "warm-revisit"):
                phase = label + "-" + state
                metrics_snapshot(phase, "before")
                sampler = Sampler(phase, process.pid, backend)
                cpu = CpuWindow({"daemon": process.pid}, group(os.getpid())).start()
                child_before = resource.getrusage(resource.RUSAGE_CHILDREN)
                sampler.thread.start()
                try:
                    with (WORK / (phase + "-client.log")).open("xb") as output:
                        child = subprocess.Popen([sys.executable, __file__, "--client", str(mount), phase,
                                                  str(process.pid), str(backend), "--metadata-roundtrip"],
                                                  stdout=output, stderr=subprocess.STDOUT)
                        sampler.client = psutil.Process(child.pid)
                        try:
                            code = child.wait(timeout=min(1800, max(1, DEADLINE - time.monotonic())))
                        except subprocess.TimeoutExpired:
                            child.kill()
                            child.wait(timeout=15)
                            code = 124
                    cpu.finish()
                    waited = resource.getrusage(resource.RUSAGE_CHILDREN)
                finally:
                    memory = sampler.finish()
                metrics_snapshot(phase, "after")
                save(phase + "-accounting.json", memory)
                path = WORK / (phase + "-client.json")
                result = json.loads(path.read_text()) if path.exists() else {"error": "client incomplete"}
                counters = cpu.metrics()
                counters["client_cpu_user_seconds"] = waited.ru_utime - child_before.ru_utime
                counters["client_cpu_system_seconds"] = waited.ru_stime - child_before.ru_stime
                counters["client_cpu_seconds"] = counters["client_cpu_user_seconds"] + counters["client_cpu_system_seconds"]
                save(phase + "-whole-cpu-counters.json", {"before": cpu.before, "after": cpu.after,
                     "waited_user_before": child_before.ru_utime, "waited_user_after": waited.ru_utime,
                     "waited_system_before": child_before.ru_stime, "waited_system_after": waited.ru_stime})
                result.update(implementation=implementation, repetition=repetition, state=state, client_exitcode=code,
                              memory=memory, cpu=cpu_efficiency(counters, transferred=result.get("bytes", 0),
                                                              operations=result.get("correct_files", 0)))
                save(phase + "-result.json", result)
                results.append(result)
                emit({"result": phase, "correct": result.get("correct_files"), "errors": len(result.get("errors", [])),
                      "MB_second": result.get("throughput_bytes_second", 0) / 1e6,
                      "daemon_rss_peak_MB": memory["peak"]["daemon_rss"] / 1e6,
                      "client_rss_peak_MB": memory["peak"].get("client_rss", 0) / 1e6})
        finally:
            try:
                if os.path.ismount(mount):
                    subprocess.run(["umount", str(mount)], check=True, timeout=45)
            finally:
                if process.poll() is None:
                    process.terminate()
                try:
                    process.wait(timeout=45)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=15)
            if os.path.ismount(mount):
                raise RuntimeError(f"owned mount did not cleanly unmount: {mount}")
    return results


def controller():
    signal.alarm(10700)
    until = time.monotonic() + 1800
    while not (SHARED / "sealed.json").exists():
        if time.monotonic() >= until:
            raise TimeoutError("sealed fixture was not ready within the setup deadline")
        time.sleep(1)
    seal = json.loads((SHARED / "sealed.json").read_text())
    assert seal["files"] == 3000 and seal["read_only_mutations_rejected"] == 4
    assert digest(SHARED / "manifest.json") == seal["manifest_sha256"]
    backend = int((SHARED / "backend.pid").read_text())
    own, remote = group(os.getpid()), group(backend)
    assert own != remote and int((own / "memory.max").read_text()) == 4 * 1024**3
    versions = json.loads((WORK / "provenance.json").read_text())
    for implementation, binary in BINARIES.items():
        assert digest(WORK / binary) == versions[implementation]["binary_sha256"], implementation
    if not Path("/usr/bin/fusermount").exists():
        Path("/usr/bin/fusermount").symlink_to("/work/fusermount3")
    save("environment.json", {"kernel": platform.platform(), "python": sys.version, "binaries": versions,
         "fixture": seal, "cpu_max": (own / "cpu.max").read_text(), "memory_max": (own / "memory.max").read_text(),
         "backend_cpu_max": (remote / "cpu.max").read_text(), "backend_memory_max": (remote / "memory.max").read_text(),
         "cache_memory_mb": 512, "entry_limit": 100000, "reader_threads": 4, "read_chunk_bytes": CHUNK,
         "fresh": "new mount/process; host and backend cache are not controlled or globally dropped",
         "warm_revisit": "same complete dataset revisited; larger than daemon payload cache",
         "backend_scope": "same test Pod/network namespace but a separate CPU/memory cgroup; not actual-service S3",
         "metadata_workload": "all stored xattrs read and checked, GNU stat --cached=never, then ordinary stat and full payload hashes; additional to the retained failing plain-list/stat workload",
         "go_heap_peak_steady_series": None, "production_credentials": False})
    manifest = json.loads((SHARED / "manifest.json").read_text())
    listings = json.loads((SHARED / "listings.json").read_text())
    verify(client(ENDPOINT), manifest)
    save("before-native.json", {"full_hashes_verified": len(manifest), "generation_errors": []})
    base = list(BINARIES)
    orders = [base[offset:] + base[:offset] for offset in range(4)] + [list(reversed(base))]
    save("orders.json", orders)
    report = {"accepted": False, "comparison_valid": False, "runs": [], "errors": []}
    try:
        for repetition, order in enumerate(orders):
            for implementation in order:
                if time.monotonic() > DEADLINE - 120:
                    raise TimeoutError("whole immutable experiment deadline reached")
                try:
                    report["runs"].extend(mount_run(implementation, repetition, backend))
                except Exception as error:
                    report["errors"].append({"implementation": implementation, "repetition": repetition, "error": str(error)})
                    emit(report["errors"][-1])
                    if any(os.path.ismount(path) for path in WORK.glob("*-mount")):
                        raise RuntimeError("abort after mount cleanup failure to avoid contaminating later phases") from error
        verify(client(ENDPOINT), manifest)
        assert digest(SHARED / "manifest.json") == seal["manifest_sha256"]
        for entry in listings:
            assert list_names(client(ENDPOINT), entry["path"]) == entry["names"], entry["path"]
        save("after-native.json", {"full_hashes_verified": len(manifest), "listing_checks": len(listings), "generation_errors": []})
        report["comparison_valid"] = (len(report["runs"]) == 40 and not report["errors"] and all(
            result.get("correct_files") == 3000 and result.get("bytes") == seal["bytes"] and
            not result.get("errors") and result.get("client_exitcode") == 0 for result in report["runs"]))
    finally:
        report["elapsed_seconds"] = time.monotonic() - START
        save("report.json", report)
        emit({"finished": True, "runs": len(report["runs"]), "comparison_valid": report["comparison_valid"],
              "errors": report["errors"], "accepted": False})


if __name__ == "__main__":
    if len(sys.argv) > 1:
        assert sys.argv[1] == "--client"
        assert sys.argv[6:] in ([], ["--metadata-roundtrip"])
        client_phase(sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5]), roundtrip=bool(sys.argv[6:]))
    else:
        controller()
