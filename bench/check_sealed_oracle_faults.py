"""Prove failure sensitivity of sealed-fixture/measurement checks in copied sources.

This targeted check does not mutate the checkout, mount anything or access S3.
Run it in the owned verification container with requirements-fixture installed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


FAULTS = [
    ("native-checksum", "sealed_s3_fixture.py",
     'count == entry["size"] and digest.hexdigest() == entry["sha256"]', 'count == entry["size"]'),
    ("native-generation", "sealed_s3_fixture.py", ', IfMatch=entry["etag"]', ''),
    ("native-uid", "sealed_s3_fixture.py", '("mtime", "uid", "gid")', '("mtime", "gid")'),
    ("listing-first-page-only", "sealed_s3_fixture.py", '    return sorted(names)', '        break\n    return sorted(names)'),
    ("permission-success", "sealed_s3_fixture.py",
     'raise AssertionError("sealed fixture unexpectedly admitted a mutation")', 'return'),
    ("mounted-checksum", "read_dataset.py",
     'count != entry["size"] or checksum.hexdigest() != entry["sha256"]', 'count != entry["size"]'),
    ("mounted-mtime", "read_dataset.py", 'int(info.st_mtime)', 'entry["mtime"]'),
    ("steady-whole-window", "read_dataset.py", 'samples[-max(1, len(samples) // 5):]', 'samples'),
    ("missing-memory-zero", "read_dataset.py",
     'max(v) if (v := values(samples, key)) else None', 'max(v) if (v := values(samples, key)) else 0'),
    ("tail-minimum", "read_dataset.py", 'math.ceil(len(ordered) * fraction)', '1'),
]


def check(bench, tests, output):
    output.mkdir()
    sources = {name: (bench / name).read_text() for name in ("read_dataset.py", "sealed_s3_fixture.py", "accounting.py", "metadata_roundtrip.py")}
    hashes = {name: hashlib.sha256(value.encode()).hexdigest() for name, value in sources.items()}
    receipts = []
    for label, name, old, new in [("baseline", None, None, None), *FAULTS, ("restored", None, None, None)]:
        with tempfile.TemporaryDirectory(prefix="sealed-oracle-") as temporary:
            root = Path(temporary)
            (root / "bench").mkdir()
            (root / "tests").mkdir()
            for source, value in sources.items():
                if source == name:
                    assert value.count(old) == 1, (label, "mutation no longer identifies one guard")
                    value = value.replace(old, new)
                (root / "bench" / source).write_text(value)
            shutil.copyfile(tests / "test_sealed_benchmark.py", root / "tests/test_sealed_benchmark.py")
            path = output / (label + ".log")
            with path.open("xb") as log:
                result = subprocess.run([sys.executable, "-m", "unittest", "discover", "-v", "-s", str(root / "tests")],
                                        env=os.environ.copy(), stdout=log, stderr=subprocess.STDOUT, timeout=45)
            text = path.read_text()
            if name is None:
                assert result.returncode == 0 and "Ran 8 tests" in text and "\nOK\n" in text, label
            else:
                assert result.returncode == 1 and "FAIL:" in text and "ERROR:" not in text, (label, text)
            receipts.append({"fault": label, "exitcode": result.returncode,
                             "assertion_failures": text.count("FAIL:"), "setup_errors": text.count("ERROR:"),
                             "log_sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    assert all(hashlib.sha256((bench / name).read_bytes()).hexdigest() == value for name, value in hashes.items())
    receipt = {"source_hashes": hashes, "tests_sha256": hashlib.sha256((tests / "test_sealed_benchmark.py").read_bytes()).hexdigest(),
               "targeted_faults": len(FAULTS), "killed": len(FAULTS), "survived": 0,
               "original_sources_unchanged": True, "runs": receipts}
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bench", type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument("--tests", type=Path, default=Path(__file__).resolve().parents[1] / "tests")
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    check(arguments.bench, arguments.tests, arguments.output)
