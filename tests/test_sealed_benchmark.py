import hashlib
import io
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest
from unittest import mock

from botocore.exceptions import ClientError
from botocore.stub import Stubber

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
from accounting import CpuWindow
import read_dataset
import sealed_s3_fixture as fixture


class NativeFixtureOracle(unittest.TestCase):
    def entry(self):
        return {"key": "data/wide/file-0000", "path": "wide/file-0000", "size": 8,
                "sha256": hashlib.sha256(b"one\0two\n").hexdigest(), "etag": '"seed-generation"',
                "mode": 0o644, "mtime": 1700000000, "uid": 1000, "gid": 1000}

    def test_full_get_checks_generation_every_byte_and_metadata(self):
        for fault in (None, "payload", "size", "uid"):
            with self.subTest(fault=fault):
                entry = self.entry()
                payload = b"one\0two\n" if fault != "payload" else b"one\0bad\n"
                if fault == "size":
                    payload += b"x"
                metadata = {"mode": str(0o100644), "mtime": "1700000000", "uid": "1000", "gid": "1000"}
                if fault == "uid":
                    metadata["uid"] = "2000"
                s3 = fixture.client("http://127.0.0.1:1")
                with Stubber(s3) as service:
                    service.add_response("get_object", {"Body": io.BytesIO(payload), "Metadata": metadata},
                                         {"Bucket": "fixture", "Key": entry["key"], "IfMatch": '"seed-generation"'})
                    if fault is None:
                        fixture.verify(s3, [entry])
                    else:
                        with self.assertRaises(AssertionError):
                            fixture.verify(s3, [entry])
                    service.assert_no_pending_responses()

    def test_changed_generation_cannot_be_a_successful_native_preflight(self):
        s3 = fixture.client("http://127.0.0.1:1")
        with Stubber(s3) as service:
            service.add_client_error("get_object", service_error_code="PreconditionFailed", http_status_code=412,
                expected_params={"Bucket": "fixture", "Key": self.entry()["key"], "IfMatch": '"seed-generation"'})
            with self.assertRaises(ClientError) as caught:
                fixture.verify(s3, [self.entry()])
            self.assertEqual(caught.exception.response["ResponseMetadata"]["HTTPStatusCode"], 412)

    def test_listing_follows_all_pages_and_includes_common_prefixes(self):
        s3 = fixture.client("http://127.0.0.1:1")
        with Stubber(s3) as service:
            service.add_response("list_objects_v2", {"IsTruncated": True, "NextContinuationToken": "next",
                "Contents": [{"Key": "data/wide/a"}], "CommonPrefixes": [{"Prefix": "data/wide/sub/"}]},
                {"Bucket": "fixture", "Prefix": "data/wide/", "Delimiter": "/"})
            service.add_response("list_objects_v2", {"IsTruncated": False, "Contents": [{"Key": "data/wide/z"}]},
                {"Bucket": "fixture", "Prefix": "data/wide/", "Delimiter": "/", "ContinuationToken": "next"})
            self.assertEqual(fixture.list_names(s3, "wide"), ["a", "sub", "z"])
            service.assert_no_pending_responses()

    def test_expected_listing_comes_from_seed_not_a_possibly_incomplete_server(self):
        self.assertEqual(fixture.expected_listings([{"path": "z/b"}, {"path": "wide/c"}, {"path": "wide/a"}]),
                         [{"path": "", "names": ["wide", "z"]}, {"path": "wide", "names": ["a", "c"]},
                          {"path": "z", "names": ["b"]}])

    def test_permission_probe_rejects_success_and_does_not_accept_server_errors(self):
        fixture.denied(lambda: (_ for _ in ()).throw(ClientError({"Error": {"Code": "AccessDenied"},
            "ResponseMetadata": {"HTTPStatusCode": 403}}, "PutObject")))
        with self.assertRaisesRegex(AssertionError, "admitted a mutation"):
            fixture.denied(lambda: None)
        with self.assertRaises(AssertionError):
            fixture.denied(lambda: (_ for _ in ()).throw(ClientError({"Error": {"Code": "Unavailable"},
                "ResponseMetadata": {"HTTPStatusCode": 503}}, "PutObject")))


class MountedDatasetOracle(unittest.TestCase):
    def test_real_files_detect_equal_length_corruption_metadata_changes_and_missing_names(self):
        for fault in (None, "payload", "mtime", "listing"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                work = Path(temporary)
                shared, mount = work / "shared", work / "mount"
                shared.mkdir()
                mount.mkdir()
                path = mount / "payload"
                path.write_bytes(b"one\0two\n" if fault != "payload" else b"one\0bad\n")
                path.chmod(0o640)
                os.utime(path, (1700000000, 1700000000 if fault != "mtime" else 1700000001))
                entry = {"path": "payload", "size": 8, "mode": 0o640, "mtime": 1700000000,
                         "uid": os.getuid(), "gid": os.getgid(), "sha256": hashlib.sha256(b"one\0two\n").hexdigest()}
                (shared / "manifest.json").write_text(json.dumps([entry]))
                (shared / "listings.json").write_text(json.dumps([{"path": "", "names":
                    ["payload"] if fault != "listing" else ["missing", "payload"]}]))
                def counters(*_):
                    return (CpuWindow({"client": os.getpid()}).start(), CpuWindow({"backend": os.getpid()}).start())
                with mock.patch.object(read_dataset, "WORK", work), mock.patch.object(read_dataset, "SHARED", shared), \
                        mock.patch.object(read_dataset, "cpu_pair", side_effect=counters):
                    try:
                        read_dataset.client_phase(mount, "test", os.getpid(), os.getpid())
                    finally:
                        signal.alarm(0)
                result = json.loads((work / "test-client.json").read_text())
                self.assertEqual(result["correct_files"], 0 if fault in ("payload", "mtime") else 1)
                self.assertEqual(len(result["errors"]), 0 if fault is None else 1)
                if fault == "listing":
                    self.assertEqual(result["errors"][0]["missing"], ["missing"])

    def test_memory_peak_is_not_the_sum_of_unrelated_peaks_and_missing_stays_missing(self):
        samples = [{"daemon_rss": 100, "client_rss": 10, "daemon_client_rss": 110, "heap": None},
                   {"daemon_rss": 10, "client_rss": 100, "daemon_client_rss": 110, "heap": None}]
        result = read_dataset.summarize_memory(samples)
        self.assertEqual(result["peak"]["daemon_client_rss"], 110)
        self.assertEqual(result["steady"]["daemon_rss"], 10)
        self.assertEqual(result["steady"]["client_rss"], 100)
        self.assertIsNone(result["peak"]["heap"])
        self.assertIsNone(result["steady"]["heap"])
        with self.assertRaisesRegex(RuntimeError, "no observations"):
            read_dataset.summarize_memory([])

    def test_nearest_rank_tail_does_not_interpolate_or_hide_an_outlier(self):
        self.assertEqual(read_dataset.percentiles([1] * 98 + [5, 100]), {"0.5": 1, "0.95": 1, "0.99": 5})
        self.assertEqual(read_dataset.percentiles([1, 100])["0.99"], 100)
        self.assertIsNone(read_dataset.percentiles([])["0.99"])


if __name__ == "__main__":
    unittest.main()
