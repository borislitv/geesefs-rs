#!/usr/bin/env python3
"""Explicit local FUSE access regression; requires root and private namespaces."""
import argparse
import errno
import json
import os
from pathlib import Path
import signal
import subprocess
import sys

import integration as fixture

BODY = b"independent cross-UID fixture bytes"
PROBE = """
import json, os, sys
try:
    fd = os.open('readable', os.O_RDONLY, dir_fd=int(sys.argv[1]))
    try:
        result = {'uid': os.getuid(), 'bytes': os.read(fd, 256).hex()}
    finally:
        os.close(fd)
except OSError as error:
    result = {'uid': os.getuid(), 'errno': error.errno}
print(json.dumps(result))
"""


def other_uid_read(path):
    # The descriptor bypasses only private fixture ancestors, not FUSE access
    # policy or the directory/file permission checks on the mounted filesystem.
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        interpreter = str(Path(sys.executable).resolve())
        result = subprocess.run([interpreter, "-I", "-c", PROBE, str(fd)], cwd="/",
                                pass_fds=(fd,), user=65534, group=65534, extra_groups=[],
                                capture_output=True, text=True, check=True, timeout=5)
        return json.loads(result.stdout)
    finally:
        os.close(fd)


def check_mount(binary, client, bucket, endpoint, environment, work, label, options, mode):
    prefix = label + "/"
    client.put_object(Bucket=bucket, Key=prefix + "readable", Body=BODY)
    receipt = {"label": label}
    extra = ["-o", options, "--file-mode", mode]
    with fixture.mount(binary, bucket, prefix, endpoint, environment, work, label,
                       memory=64, extra=extra, receipt=receipt) as (path, daemon):
        receipt["pid"] = daemon.pid
        assert (path / "readable").read_bytes() == BODY, "owner byte check"
        assert (path.stat().st_mode & 0o777) == 0o755, "mount-root permissions"
        assert ((path / "readable").stat().st_mode & 0o777) == int(mode, 8)
        observed = other_uid_read(path)
        expected = ({"uid": 65534, "bytes": BODY.hex()} if label == "allow-other"
                    else {"uid": 65534, "errno": errno.EACCES})
        assert observed == expected, (label, observed, expected)
        receipt["other_uid"] = observed
    assert daemon.returncode == 0, "normal daemon shutdown"
    receipt["daemon_exit"] = daemon.returncode
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert os.geteuid() == 0, "explicit root fixture required"
    for namespace in ("mnt", "net"):
        assert os.readlink(f"/proc/self/ns/{namespace}") != os.readlink(f"/proc/1/ns/{namespace}"), namespace
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("access fixture deadline")))
    signal.alarm(120)
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=5)
    args.output.mkdir(mode=0o700)
    binary = args.binary.resolve()
    result = {"completed": False, "accepted": False, "binary_sha256": fixture.sha_file(binary), "cases": []}
    try:
        with fixture.service(None, args.output) as (client, bucket, endpoint, environment, _):
            environment.update(TOKIO_WORKER_THREADS="2")
            for label, options, mode in [("owner-only", "rw", "0644"),
                                         ("allow-other", "ro,allow_other", "0644"),
                                         ("private-file", "ro,allow_other,nodev,nosuid,noexec,default_permissions", "0600")]:
                result["cases"].append(check_mount(binary, client, bucket, endpoint,
                    environment, args.output, label, options, mode))
        result["completed"] = True
    finally:
        with (args.output / "report.json").open("x") as stream:
            json.dump(result, stream, indent=2)
            stream.write("\n")
        signal.alarm(0)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
