#!/usr/bin/env python3
"""Explicit disposable real-binary FUSE/S3 metrics contract in private namespaces."""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import urllib.request
import urllib.parse

import integration as fixture

BODY = b"independent metrics fixture bytes"


def sample(text, name):
    return float(next(line.split()[1] for line in text.splitlines()
                      if line.startswith(name + " ")))


def check(binary, client, bucket, endpoint, environment, work):
    client.put_object(Bucket=bucket, Key="metrics/private-fixture-object", Body=BODY)
    stats = work / "stats.jsonl"
    extra = ["-o", "ro", "--read-only-direct-io", "--metrics-listen", "127.0.0.1:0",
             "--stats-file", str(stats)]
    receipt = {}
    with fixture.mount(binary, bucket, "metrics/", endpoint, environment, work,
                       "metrics", extra=extra, receipt=receipt) as (path, daemon):
        log = (work / "metrics.log").read_text()
        url = next(line.split("metrics listening on ", 1)[1] for line in log.splitlines()
                   if line.startswith("metrics listening on "))
        assert (path / "private-fixture-object").read_bytes() == BODY
        assert (path / "private-fixture-object").read_bytes() == BODY
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(url, timeout=5) as response:
            assert response.headers["Content-Type"].startswith("application/openmetrics-text;")
            text = response.read(65536).decode()
        assert text.endswith("# EOF\n")
        assert sample(text, "geesefs_read_cache_misses_total") == 1
        assert sample(text, "geesefs_read_cache_hits_total") == 1
        assert sample(text, "geesefs_read_remote_bytes_total") == len(BODY)
        assert sample(text, "geesefs_read_cache_bytes") == len(BODY)
        assert "private-fixture-object" not in text
        allocation = json.loads(stats.read_text().splitlines()[0])
        assert allocation["allocator"] in ("jemalloc", "system")
        assert allocation["allocated_bytes"] > 0
        assert "effective_budgets" in allocation
        (work / "scrape.txt").write_text(text)
        receipt.update(pid=daemon.pid, metrics_url=url, allocator=allocation["allocator"])
    assert daemon.returncode == 0
    address = urllib.parse.urlsplit(url)
    with socket.socket() as connection:
        connection.settimeout(2)
        assert connection.connect_ex((address.hostname, address.port)) != 0, "listener survived daemon exit"
    receipt["daemon_exit"] = daemon.returncode
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    assert os.geteuid() == 0, "explicit root fixture required"
    for namespace in ("mnt", "net"):
        assert os.readlink(f"/proc/self/ns/{namespace}") != os.readlink(f"/proc/1/ns/{namespace}")
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("metrics fixture deadline")))
    signal.alarm(120)
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=5)
    args.output.mkdir(mode=0o700)
    binary = args.binary.resolve()
    report = {"completed": False, "binary_sha256": fixture.sha_file(binary)}
    try:
        with fixture.service(None, args.output) as (client, bucket, endpoint, environment, _):
            environment.update(TOKIO_WORKER_THREADS="2")
            report["mount"] = check(binary, client, bucket, endpoint, environment, args.output)
        report["completed"] = True
    finally:
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        signal.alarm(0)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
