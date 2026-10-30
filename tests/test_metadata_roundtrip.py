import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
from metadata_roundtrip import metadata_roundtrip


class MetadataRoundtrip(unittest.TestCase):
    def test_actual_xattrs_and_gnu_stat_preserve_failures_for_each_metadata_field(self):
        for fault in (None, "mode", "mtime", "uid", "gid", "cached-attribute"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                path = root / "payload"
                path.write_bytes(b"payload\n")
                path.chmod(0o640)
                os.utime(path, (1700000000, 1700000000))
                entry = {"path": "payload", "size": 8, "mode": 0o640, "mtime": 1700000000,
                         "uid": os.getuid(), "gid": os.getgid()}
                for field in ("mode", "mtime", "uid", "gid"):
                    value = entry[field] | 0o100000 if field == "mode" else entry[field]
                    os.setxattr(path, "user." + field, str(value + (1 if field == fault else 0)).encode())
                if fault == "cached-attribute":
                    os.utime(path, (1700000000, 1700000001))
                result = metadata_roundtrip(root, [entry])
                self.assertEqual(result["checked_files"], 1)
                self.assertEqual(result["xattr_checks"], 4)
                self.assertEqual(len(result["errors"]), 0 if fault is None else 1)
                if fault in ("mode", "mtime", "uid", "gid"):
                    self.assertEqual(result["errors"][0]["xattr"], fault)
                if fault == "cached-attribute":
                    self.assertEqual(result["errors"][0]["stat_observed"][2], 1700000001)


if __name__ == "__main__":
    unittest.main()
