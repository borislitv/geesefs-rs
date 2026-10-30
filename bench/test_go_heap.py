import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest import mock

import go_heap


PROFILE = (b"heap profile: 1: 64 [2: 128] @ heap/1048576\n"
           b"# runtime.MemStats\n# HeapAlloc = 123456\n# HeapSys = 262144\n"
           b"# HeapIdle = 65536\n# HeapInuse = 196608\n# HeapReleased = 32768\n"
           b"# NumGC = 7\n# NumForcedGC = 0\n")


@contextlib.contextmanager
def server(body, *, received=None, gate=None):
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            if received is not None and len(requests) >= 3:
                received.set()
            if gate is not None:
                gate.wait(timeout=5)
            self.send_response(200)
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def log_message(self, *_):
            pass

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as http:
        thread = threading.Thread(target=http.serve_forever, kwargs={"poll_interval": 0.01})
        thread.start()
        try:
            yield http.server_port, requests
        finally:
            if gate is not None:
                gate.set()
            http.shutdown()
            thread.join(timeout=2)
            assert not thread.is_alive()


class HeapHttpTests(unittest.TestCase):
    def test_live_memstats_are_not_pprof_sampled_inuse_or_forced_gc(self):
        with server(PROFILE) as (port, requests):
            result = go_heap.read_go_heap(port)
        self.assertEqual(requests, ["/debug/pprof/heap?debug=1"])
        self.assertEqual(result["memstats"]["HeapAlloc"], 123456)
        self.assertEqual(result["memstats"]["HeapReleased"], 32768)
        self.assertEqual(result["memstats"]["NumForcedGC"], 0)
        self.assertGreaterEqual(result["finished_ns"], result["started_ns"])
        self.assertGreaterEqual(result["request_seconds"], 0)
        self.assertEqual(result["response_bytes"], len(PROFILE))

    def test_missing_or_invalid_stats_cannot_be_zero_heap(self):
        for body in (b"heap profile: 0: 0\n", PROFILE.replace(b"HeapAlloc = 123456", b"HeapAlloc = -1"),
                     PROFILE + b"# HeapAlloc = 42\n"):
            with self.subTest(body=body), server(body) as (port, _):
                with self.assertRaisesRegex(ValueError, "MemStats"):
                    go_heap.read_go_heap(port)

    def test_profile_response_is_bounded(self):
        with server(b"x" * (go_heap.MAX_PROFILE_BYTES + 1)) as (port, _):
            with self.assertRaisesRegex(ValueError, "profile.*limit"):
                go_heap.read_go_heap(port)

    def test_unresponsive_endpoint_has_a_finite_request_timeout(self):
        with server(PROFILE, gate=threading.Event()) as (port, _):
            with self.assertRaises(TimeoutError):
                go_heap.read_go_heap(port)


class HeapSeriesTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.trace = Path(self.directory.name) / "heap.jsonl"

    def samples(self, rows):
        self.trace.write_text("".join(json.dumps({"started_ns": start, "finished_ns": end,
                                                 "request_seconds": 0.002,
                                                 "memstats": {"HeapAlloc": allocated,
                                                              "NumForcedGC": forced}}) + "\n"
                                      for start, end, allocated, forced in rows))
        return go_heap.GoHeapSampler(12345, self.trace).metrics(10, 100)

    def test_peak_and_late_mean_come_from_active_series_not_end_snapshot(self):
        result = self.samples([(0, 9, 999, 0), (10, 20, 30, 0), (30, 40, 90, 0),
                               (70, 80, 50, 0), (90, 110, 1000, 0)])
        self.assertEqual(result["allocator_peak_bytes"], 90)
        self.assertEqual(result["allocator_steady_bytes"], 70)
        self.assertEqual(result["allocator_samples"], 3)
        self.assertAlmostEqual(result["allocator_sampling_wall_seconds"], 0.006)

    def test_single_snapshot_or_no_active_sample_is_insufficient(self):
        for rows in ([], [(10, 20, 30, 0)], [(0, 11, 999, 0), (90, 110, 999, 0)]):
            with self.subTest(rows=rows):
                result = self.samples(rows)
                self.assertIsNone(result["allocator_peak_bytes"])
                self.assertIsNone(result["allocator_steady_bytes"])

    def test_baseline_owned_gc_is_reported_without_changing_its_policy(self):
        result = self.samples([(10, 20, 30, 0), (30, 40, 90, 1), (70, 80, 50, 1)])
        self.assertEqual(result["allocator_peak_bytes"], 90)
        self.assertEqual(result["allocator_forced_gc_count"], 1)

    def test_decreasing_gc_counter_invalidates_series(self):
        with self.assertRaisesRegex(RuntimeError, "counter decreased"):
            self.samples([(10, 20, 30, 1), (30, 40, 90, 0), (70, 80, 50, 0)])

    def test_background_observations_survive_until_normal_shutdown(self):
        received = threading.Event()
        with server(PROFILE, received=received) as (port, _):
            with go_heap.GoHeapSampler(port, self.trace, interval=0.01) as observer:
                self.assertTrue(received.wait(timeout=2))
        result = observer.metrics(0, 10 ** 30)
        self.assertGreaterEqual(result["allocator_samples"], 3)
        self.assertEqual(result["allocator_peak_bytes"], 123456)
        self.assertEqual(result["allocator_steady_bytes"], 123456)

    def test_background_failure_is_retained_and_propagated(self):
        attempted = threading.Event()

        def unavailable(_port):
            attempted.set()
            raise OSError("fixture pprof unavailable")

        with mock.patch.object(go_heap, "read_go_heap", side_effect=unavailable):
            with self.assertRaisesRegex(RuntimeError, "fixture pprof unavailable"):
                with go_heap.GoHeapSampler(12345, self.trace):
                    self.assertTrue(attempted.wait(timeout=2))
        rows = [json.loads(line) for line in self.trace.read_text().splitlines()]
        self.assertEqual(len(rows), 1)
        self.assertIn("fixture pprof unavailable", rows[0]["error"])
        self.assertNotIn("memstats", rows[0])
