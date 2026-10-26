#!/usr/bin/env python3
"""Independent, disposable Linux FUSE/S3 interoperability checks.

No ambient AWS credential is consumed. Without --endpoint a local Moto process
is owned by this invocation. External endpoints require explicit disposable
acknowledgment and GEESEFS_TEST_ACCESS_KEY_ID/GEESEFS_TEST_SECRET_ACCESS_KEY.
Every requested failure is nonzero and preserved in the private JSON report.
"""
import argparse
import contextlib
import errno
import functools
import hashlib
import json
import mmap
import os
from pathlib import Path
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor

ROOT = Path(__file__).resolve().parents[1]
EXPERIMENTAL = "5c0884c9e8799bbab93e2d4102fdc15e9c079e0d"
UPSTREAM = "80c214ef14f620d93407454fc76ca01dd71fa25f"
OPTIONAL_GAPS = ["yandex-iam", "sse-modes", "signature-negotiation", "patch", "all-cli-paths"]
FIXTURE_CHUNK = 64 * 1024


def command(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, timeout=kwargs.pop("timeout", 300), **kwargs)


def sha_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def private_environment(access, secret):
    environment = {k: v for k, v in os.environ.items() if not k.startswith("AWS_")}
    environment.update(AWS_ACCESS_KEY_ID=access, AWS_SECRET_ACCESS_KEY=secret,
                       AWS_DEFAULT_REGION="us-east-1", AWS_REGION="us-east-1",
                       AWS_EC2_METADATA_DISABLED="true", AWS_CONFIG_FILE=os.devnull,
                       AWS_SHARED_CREDENTIALS_FILE=os.devnull)
    return environment


@contextlib.contextmanager
def service(endpoint, work):
    import boto3
    from botocore.config import Config

    process = None
    if endpoint:
        if os.environ.get("GEESEFS_TEST_DISPOSABLE") != "1":
            raise RuntimeError("external backend requires GEESEFS_TEST_DISPOSABLE=1")
        access = os.environ.get("GEESEFS_TEST_ACCESS_KEY_ID")
        secret = os.environ.get("GEESEFS_TEST_SECRET_ACCESS_KEY")
        if not access or not secret:
            raise RuntimeError("explicit disposable GEESEFS_TEST_* credentials are required")
    else:
        access, secret = "disposable-access", "disposable-secret"
        endpoint = f"http://127.0.0.1:{free_port()}"
        log = open(work / "s3-fixture.log", "wb")
        process = subprocess.Popen([sys.executable, str(ROOT / "scripts/s3_fixture.py"), "--port", endpoint.rsplit(":", 1)[1]], stdout=log, stderr=log)
    environment = private_environment(access, secret)
    client = boto3.client("s3", endpoint_url=endpoint, aws_access_key_id=access,
                          aws_secret_access_key=secret, region_name="us-east-1",
                          config=Config(retries={"max_attempts": 1}, connect_timeout=3, read_timeout=30,
                                        s3={"addressing_style": "path"}))
    bucket = "geesefs-rs-test-" + uuid.uuid4().hex
    created = False
    try:
        deadline = time.monotonic() + 20
        while True:
            try:
                client.create_bucket(Bucket=bucket)
                created = True
                break
            except Exception:
                if process is None or process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("disposable S3 service failed to initialize; see private fixture log") from None
                time.sleep(0.1)
        yield client, bucket, endpoint, environment, process is not None
    finally:
        try:
            if created:
                # This bucket was successfully created by this invocation only.
                paginator = client.get_paginator("list_objects_v2")
                for page in paginator.paginate(Bucket=bucket):
                    keys = [{"Key": item["Key"]} for item in page.get("Contents", [])]
                    if keys:
                        client.delete_objects(Bucket=bucket, Delete={"Objects": keys})
                uploads = client.get_paginator("list_multipart_uploads")
                for page in uploads.paginate(Bucket=bucket):
                    for upload in page.get("Uploads", []):
                        client.abort_multipart_upload(Bucket=bucket, Key=upload["Key"], UploadId=upload["UploadId"])
                client.delete_bucket(Bucket=bucket)
        finally:
            client.close()
            if process:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                log.close()


def build_go(reference, output, revision=EXPERIMENTAL):
    reference = Path(reference).resolve()
    output = Path(output).resolve()
    actual = command(["git", "rev-parse", "HEAD"], cwd=reference, capture_output=True, text=True).stdout.strip()
    if actual != revision:
        raise RuntimeError(f"Go reference revision is {actual}, expected {revision}")
    if command(["git", "status", "--porcelain"], cwd=reference, capture_output=True, text=True).stdout:
        raise RuntimeError("Go reference checkout is dirty; provenance cannot be accepted")
    command(["go", "build", "-o", output, "."], cwd=reference, timeout=600)
    return {"source_revision": actual, "binary_sha256": sha_file(output),
            "toolchain": command(["go", "version"], capture_output=True, text=True).stdout.strip()}


def enter_cgroup(group):
    if group:
        fd = os.open(Path(group) / "cgroup.procs", os.O_WRONLY)
        try:
            os.write(fd, str(os.getpid()).encode())
        finally:
            os.close(fd)


@contextlib.contextmanager
def mount(binary, bucket, prefix, endpoint, environment, work, label, memory=64, extra=(), group=None, receipt=None):
    mountpoint = work / (label + "-mount")
    mountpoint.mkdir()
    log_path = work / (label + ".log")
    args = [str(binary), "-f", "--endpoint", endpoint, "--region", "us-east-1",
            "--memory-limit", str(memory), "--enable-perms", "--enable-mtime", *extra,
            f"{bucket}:{prefix}", str(mountpoint)]
    if receipt is not None:
        receipt["mount_arguments"] = args
    with open(log_path, "wb") as log:
        process = subprocess.Popen(args, env=environment, stdout=log, stderr=log,
                                   preexec_fn=(lambda: enter_cgroup(group)) if group else None)
        try:
            deadline = time.monotonic() + 30
            while not os.path.ismount(mountpoint):
                if process.poll() is not None:
                    detail = log_path.read_text(errors="replace")[-8000:]
                    raise RuntimeError(f"{label} mount exited {process.returncode}: {detail}")
                if time.monotonic() >= deadline:
                    raise RuntimeError(f"{label} mount timed out")
                time.sleep(0.05)
            yield mountpoint, process
        finally:
            failed = sys.exc_info()[0] is not None
            cleanup_error = None
            try:
                if os.path.ismount(mountpoint):
                    command(["fusermount3", "-u", mountpoint], timeout=30, capture_output=True)
            except Exception as error:
                cleanup_error = error
            finally:
                if cleanup_error is not None and process.poll() is None:
                    process.terminate()
                try:
                    process.wait(timeout=10 if cleanup_error is not None else 45)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
                    cleanup_error = cleanup_error or RuntimeError(f"{label} did not shut down normally")
                if cleanup_error is not None and os.path.ismount(mountpoint):
                    # Only this invocation's mount; ordinary unmount, never lazy/force.
                    command(["fusermount3", "-u", mountpoint], timeout=30, capture_output=True)
            if cleanup_error is not None:
                raise RuntimeError(f"{label} cleanup failed: {cleanup_error}") from cleanup_error
            if process.returncode and not failed:
                raise RuntimeError(f"{label} exited {process.returncode}; see {log_path.name}")


@functools.lru_cache(maxsize=16)
def fixture_chunk(index):
    return hashlib.shake_256(b"geesefs-rs-position-fixture-v1" + index.to_bytes(8, "little")).digest(FIXTURE_CHUNK)


def expected_range(offset, length):
    result = bytearray()
    while length:
        index, within = divmod(offset, FIXTURE_CHUNK)
        count = min(length, FIXTURE_CHUNK - within)
        result.extend(fixture_chunk(index)[within:within + count])
        offset += count
        length -= count
    return bytes(result)


def seed(client, bucket, prefix, size=16 * 1024 * 1024, entries=32):
    # Multipart test data generation is bounded, independent of the filesystem.
    upload = client.create_multipart_upload(Bucket=bucket, Key=prefix + "/payload")
    parts, digest = [], hashlib.sha256()
    try:
        offset = 0
        while offset < size:
            length = min(5 * 1024 * 1024, size - offset)
            body = expected_range(offset, length)
            digest.update(body)
            number = len(parts) + 1
            part = client.upload_part(Bucket=bucket, Key=prefix + "/payload", UploadId=upload["UploadId"], PartNumber=number, Body=body)
            parts.append({"PartNumber": number, "ETag": part["ETag"]})
            offset += length
        client.complete_multipart_upload(Bucket=bucket, Key=prefix + "/payload", UploadId=upload["UploadId"], MultipartUpload={"Parts": parts})
    except BaseException:
        client.abort_multipart_upload(Bucket=bucket, Key=prefix + "/payload", UploadId=upload["UploadId"])
        raise
    for i in range(entries):
        client.put_object(Bucket=bucket, Key=f"{prefix}/implicit/{i:06d}", Body=f"entry-{i}".encode())
    return digest.hexdigest()


def remote_bytes(client, bucket, key):
    response = client.get_object(Bucket=bucket, Key=key)
    try:
        return response["Body"].read()
    finally:
        response["Body"].close()


def fault(endpoint, mode=""):
    request = urllib.request.Request(endpoint + "/_geesefs_test_fault?mode=" + mode, method="POST")
    with urllib.request.urlopen(request, timeout=5) as response:
        response.read()


def recover_fsync(fd):
    """Reference write failures are retryable EAGAIN until its configured flusher
    retry interval expires. Observe recovery without changing that interval/GC.
    Other errors remain failures; this is not a generic retry-until-green wrapper.
    """
    deadline = time.monotonic() + 75
    while True:
        try:
            os.fsync(fd)
            return
        except OSError as error:
            if error.errno != errno.EAGAIN or time.monotonic() >= deadline:
                raise
            time.sleep(0.25)


def exercise(path, client, bucket, prefix, source, endpoint, local, checks=None):
    if checks is None:
        checks = []
    assert sha_file(path / "payload") == source["payload_sha256"], "full payload checksum"
    with open(path / "payload", "rb", buffering=0) as stream:
        for offset, length in [(3, 257), (1024 * 1024 - 7, 4099), (16 * 1024 * 1024 - 2, 8)]:
            actual = os.pread(stream.fileno(), length, offset)
            assert actual == expected_range(offset, min(length, 16 * 1024 * 1024 - offset)), "unaligned Range/EOF"
        with mmap.mmap(stream.fileno(), 4096, access=mmap.ACCESS_READ) as mapped:
            assert mapped[:] == expected_range(0, 4096), "shared read-only mmap"
    assert sorted(p.name for p in (path / "implicit").iterdir()) == [f"{i:06d}" for i in range(32)], "complete directory traversal"
    checks += ["full-checksum", "range-eof", "mmap", "implicit-directory"]
    port = free_port()
    http = subprocess.Popen([sys.executable, str(ROOT / "scripts/http_reader.py"), "--file", str(path / "payload"), "--port", str(port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        url = f"http://127.0.0.1:{port}/payload"
        deadline = time.monotonic() + 10
        while True:
            try:
                request = urllib.request.Request(url, headers={"Range": "bytes=3-259"})
                with urllib.request.urlopen(request, timeout=5) as response:
                    assert response.status == 206
                    assert response.read() == expected_range(3, 257)
                break
            except OSError:
                if http.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("aiohttp mounted-file reader failed to start") from None
                time.sleep(0.05)
        digest = hashlib.sha256()
        with urllib.request.urlopen(url, timeout=30) as response:
            while block := response.read(1024 * 1024):
                digest.update(block)
        assert digest.hexdigest() == source["payload_sha256"], "HTTP full response checksum"
        checks.append("http-full-range")
    finally:
        http.terminate()
        http.wait(timeout=10)

    origin = source["directory"]
    command(["rsync", "-a", "--no-owner", "--no-group", "--checksum", str(origin) + "/", str(path / "mirror") + "/"], timeout=120)
    with open(origin / "large", "r+b") as stream:
        stream.seek(1024 * 1024 + 7)
        stream.write(b"delta-update")
    command(["rsync", "-a", "--no-owner", "--no-group", "--checksum", str(origin) + "/", str(path / "mirror") + "/"], timeout=120)
    assert sha_file(origin / "large") == sha_file(path / "mirror/large"), "rsync delta checksum"
    assert os.readlink(path / "mirror/link") == "large", "symlink target"
    assert (path / "mirror/small").stat().st_mode & 0o777 == 0o640, "mode"
    assert int((path / "mirror/small").stat().st_mtime) == 1700000000, "integer mtime"
    checks += ["rsync-full-delta", "symlink", "mode-mtime"]
    os.setxattr(path / "mirror/small", "user.test", b"percent%\nvalue")
    assert os.getxattr(path / "mirror/small", "user.test") == b"percent%\nvalue"
    try:
        os.link(path / "mirror/small", path / "emulated-hardlink")
    except OSError as error:
        # Preserve a failed optional compatibility probe without preventing the
        # independent read/write checks or the reference mount from running.
        checks.append(f"hardlink-emulation:FAILED:errno={error.errno}")
    else:
        assert os.readlink(path / "emulated-hardlink") == "mirror/small"
        assert (path / "emulated-hardlink").read_bytes() == b"metadata fixture"
        checks.append("hardlink-emulation")
    os.mkfifo(path / "pipe", 0o600)
    assert stat.S_ISFIFO((path / "pipe").stat().st_mode)
    directory_fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)
    checks += ["xattr", "special-fifo", "fsyncdir"]

    with open(path / "writer", "w+b", buffering=0) as stream:
        os.pwrite(stream.fileno(), b"abcdef", 1024 * 1024)
        stream.truncate(1024 * 1024 + 3)
        stream.truncate(1024 * 1024 + 9)
        assert os.pread(stream.fileno(), 12, 1024 * 1024 - 3) == b"\0\0\0abc\0\0\0\0\0\0", "sparse/truncate read-after-write"
        os.fsync(stream.fileno())
        assert remote_bytes(client, bucket, prefix + "/writer") == b"\0" * (1024 * 1024) + b"abc" + b"\0" * 6, "fresh backend durability"
        os.rename(path / "writer", path / "renamed")
        os.pwrite(stream.fileno(), b"renamed", 0)
        os.fsync(stream.fileno())
    assert remote_bytes(client, bucket, prefix + "/renamed")[:7] == b"renamed", "rename-before-close"
    checks += ["sparse-truncate", "fsync-fresh-reader", "rename-before-close"]

    with open(path / "concurrent", "w+b", buffering=0) as stream:
        fd = stream.fileno()
        def writer(i):
            os.pwrite(fd, bytes([65 + i]) * 4096, i * 4096)
            os.fsync(fd)
        with ThreadPoolExecutor(max_workers=4) as workers:
            list(workers.map(writer, range(4)))
        os.fsync(fd)
    assert remote_bytes(client, bucket, prefix + "/concurrent") == b"A" * 4096 + b"B" * 4096 + b"C" * 4096 + b"D" * 4096, "concurrent durability"
    checks.append("concurrent-writer-fsync")
    if local:
        (path / "sync-fault").mkdir()
        fault(endpoint, "put")
        sync_fd = os.open(path / "sync-fault" / "fault", os.O_WRONLY | os.O_CREAT | os.O_SYNC, 0o600)
        try:
            try:
                os.write(sync_fd, b"synchronous write must report failure")
            except OSError:
                checks.append("sync-write-propagates-upload-failure")
            else:
                raise AssertionError("O_SYNC write acknowledged failed durability")
        finally:
            fault(endpoint)
            os.close(sync_fd)
        fault(endpoint, "put")
        try:
            with open(path / "fault", "w+b", buffering=0) as stream:
                stream.write(b"retained after failure")
                try:
                    os.fsync(stream.fileno())
                except OSError:
                    pass
                else:
                    raise AssertionError("injected upload failure acknowledged as durable")
                assert os.pread(stream.fileno(), 22, 0) == b"retained after failure"
                fault(endpoint)
                recover_fsync(stream.fileno())
            assert remote_bytes(client, bucket, prefix + "/fault") == b"retained after failure"
        finally:
            fault(endpoint)
        checks.append("failed-fsync-retains-data")
    return checks


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--go-reference", default=os.environ.get("GEESEFS_GO_REFERENCE"))
    result.add_argument("--rust-binary", type=Path, default=ROOT / "target/debug/geesefs-rs")
    result.add_argument("--endpoint", default=os.environ.get("GEESEFS_TEST_ENDPOINT"))
    result.add_argument("--output", type=Path, default=ROOT / "benchmark-results/private/integration")
    result.add_argument("--rust-only", action="store_true", help="diagnostic run, never full compatibility acceptance")
    result.add_argument("--preflight", action="store_true")
    result.add_argument("--timeout", type=int, default=1800, help="overall operation deadline in seconds")
    return result


def main():
    args = parser().parse_args()
    args.output = args.output.resolve()
    if args.output.exists() and any(args.output.iterdir()):
        raise SystemExit("output directory is not empty; choose a new path to preserve prior evidence")
    if args.timeout <= 0:
        raise SystemExit("--timeout must be positive")
    def expired(_signum, _frame):
        raise TimeoutError("mounted integration deadline exceeded")
    signal.signal(signal.SIGALRM, expired)
    signal.alarm(args.timeout)
    args.output.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(args.output, 0o700)
    report = {"accepted": False, "checks": {}, "errors": [], "optional_mode_gaps": OPTIONAL_GAPS}
    try:
        if not args.rust_only and not args.go_reference:
            raise RuntimeError("--go-reference or GEESEFS_GO_REFERENCE is required")
        if not args.rust_binary.is_file():
            raise RuntimeError("build the Rust binary before mounted verification")
        for tool in ["fusermount3", "rsync"]:
            if not shutil.which(tool):
                raise RuntimeError(f"missing required tool {tool}")
        with open("/dev/fuse", "r+b", buffering=0):
            pass
        if args.preflight:
            print("FUSE/tools preflight succeeded; this is not mounted acceptance")
            return 0
        with tempfile.TemporaryDirectory(prefix="geesefs-test-") as temporary:
            work = Path(temporary)
            binaries = {"rust": args.rust_binary.resolve()}
            if not args.rust_only:
                go = args.output / "geesefs-go-reference"
                report["go_provenance"] = build_go(args.go_reference, go)
                binaries["go"] = go.resolve()
            report["rust_binary_sha256"] = sha_file(args.rust_binary)
            with service(args.endpoint, work) as (client, bucket, endpoint, environment, local):
                for label, binary in binaries.items():
                    source = work / (label + "-source")
                    source.mkdir()
                    (source / "large").write_bytes(expected_range(0, 6 * 1024 * 1024))
                    (source / "small").write_bytes(b"metadata fixture")
                    os.chmod(source / "small", 0o640)
                    os.utime(source / "small", (1700000000, 1700000000))
                    os.symlink("large", source / "link")
                    prefix = uuid.uuid4().hex + "/" + label
                    payload_sha256 = seed(client, bucket, prefix)
                    try:
                        with mount(binary, bucket, prefix, endpoint, environment, work, label, extra=["--enable-specials", "--emulate-hardlinks-as-symlinks"]) as (path, _):
                            checks = report["checks"].setdefault(label, [])
                            exercise(path, client, bucket, prefix, {"directory": source, "payload_sha256": payload_sha256}, endpoint, local, checks)
                    finally:
                        for log in work.glob("*.log"):
                            shutil.copyfile(log, args.output / log.name)
        # Required optional Linux/S3 paths remain blockers, even if ordinary
        # mounted cases pass. Never equate this matrix subset to full parity.
        report["ordinary_mounted_passed"] = bool(report["checks"]) and not any("FAILED" in check for checks in report["checks"].values() for check in checks)
    except Exception as error:
        report["errors"].append(str(error))
        report["error_traceback"] = traceback.format_exc()
    finally:
        signal.alarm(0)
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
