"""Bounded Go MemStats observations; never request pprof's forced-GC option."""
import hashlib
import json
import re
import statistics
import threading
import time
import urllib.request

MAX_PROFILE_BYTES = 4 * 1024 * 1024
MEMSTATS = {"HeapAlloc", "HeapSys", "HeapIdle", "HeapInuse", "HeapReleased", "NumGC", "NumForcedGC"}


def read_go_heap(port):
    started_ns, before = time.time_ns(), time.monotonic()
    # Avoid ambient proxies even when the benchmark environment defines one.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(f"http://127.0.0.1:{port}/debug/pprof/heap?debug=1", timeout=2) as response:
        body = response.read(MAX_PROFILE_BYTES + 1)
    if len(body) > MAX_PROFILE_BYTES:
        raise ValueError("Go heap profile exceeds response limit")
    fields = re.findall(rb"^# (HeapAlloc|HeapSys|HeapIdle|HeapInuse|HeapReleased|NumGC|NumForcedGC) = ([0-9]+)$",
                        body, re.MULTILINE)
    stats = {name.decode("ascii"): int(value) for name, value in fields}
    if len(fields) != len(MEMSTATS) or set(stats) != MEMSTATS:
        raise ValueError("Go heap profile has missing, duplicate or invalid runtime.MemStats")
    return {"started_ns": started_ns, "finished_ns": time.time_ns(),
            "request_seconds": time.monotonic() - before, "response_bytes": len(body),
            "response_sha256": hashlib.sha256(body).hexdigest(), "memstats": stats}


class GoHeapSampler:
    def __init__(self, port, trace, *, interval=1):
        if interval <= 0:
            raise ValueError("positive Go heap sampling interval required")
        self.port, self.trace, self.interval = port, trace, interval
        self.done = threading.Event()
        self.failure = None
        self.thread = threading.Thread(target=self.run)

    def run(self):
        try:
            with self.trace.open("x") as output:
                while not self.done.is_set():
                    try:
                        sample = read_go_heap(self.port)
                    except Exception as error:
                        output.write(json.dumps({"error": str(error), "finished_ns": time.time_ns()}) + "\n")
                        raise
                    output.write(json.dumps(sample) + "\n")
                    output.flush()
                    self.done.wait(self.interval)
        except Exception as error:
            self.failure = str(error)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.done.set()
        self.thread.join(timeout=5)
        if self.thread.is_alive():
            raise RuntimeError("Go heap sampler did not stop")
        if self.failure:
            raise RuntimeError("Go heap accounting failed: " + self.failure)

    def metrics(self, started_ns, finished_ns):
        samples = []
        with self.trace.open() as source:
            for line in source:
                item = json.loads(line)
                if "error" in item:
                    raise RuntimeError("Go heap accounting failed: " + item["error"])
                if started_ns <= item["started_ns"] <= item["finished_ns"] <= finished_ns:
                    samples.append(item)
        forced = [item["memstats"]["NumForcedGC"] for item in samples]
        if any(right < left for left, right in zip(forced, forced[1:])):
            raise RuntimeError("Go forced GC counter decreased during measurement")
        values = [item["memstats"]["HeapAlloc"] for item in samples]
        enough = len(values) >= 2
        return {"allocator_peak_bytes": max(values) if enough else None,
                "allocator_steady_bytes": statistics.mean(values[len(values) // 2:]) if enough else None,
                "allocator_samples": len(samples), "allocator_sampling_interval_seconds": self.interval,
                "allocator_forced_gc_count": forced[-1] - forced[0] if enough else None,
                "allocator_sampling_wall_seconds": sum(item["request_seconds"] for item in samples),
                "allocator_peak_scope": "sampled-HeapAlloc-active-workload-not-an-exact-peak",
                "allocator_steady_scope": "mean-of-last-half-of-active-samples-not-a-plateau"}
