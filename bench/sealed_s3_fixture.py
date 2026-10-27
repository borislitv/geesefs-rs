"""Disposable native S3 fixture: deterministic bytes, seed once, restart read-only.

SeaweedFS owns the S3 protocol and storage. Credentials below are public fixture
identities, never production credentials. Run only in an isolated test namespace.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import contextlib
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

import boto3
from botocore.config import Config
from botocore.exceptions import ClientError

BUCKET = "fixture"
PREFIX = "data/"
READER = ("fixture-reader", "fixture-reader-secret")
SEEDER = ("fixture-seeder", "fixture-seeder-secret")
CHUNK = 1024 * 1024


def client(endpoint, identity=READER):
    return boto3.client(
        "s3", endpoint_url=endpoint, aws_access_key_id=identity[0],
        aws_secret_access_key=identity[1], region_name="us-east-1",
        config=Config(connect_timeout=5, read_timeout=30, max_pool_connections=8,
                      retries={"total_max_attempts": 2}, s3={"addressing_style": "path"},
                      request_checksum_calculation="when_required",
                      response_checksum_validation="when_required"),
    )


def identity(name, credentials, actions):
    return {"name": name, "credentials": [{"accessKey": credentials[0], "secretKey": credentials[1]}],
            "actions": actions}


def save(path, value):
    with path.open("x") as output:
        json.dump(value, output, indent=2)
        output.write("\n")


def blueprint(sizes):
    if len(sizes) != 3000 or any(type(size) is not int or not 0 < size <= 128 * 1024**2 for size in sizes):
        raise ValueError("fixture requires exactly 3000 positive sizes no larger than 128 MiB")
    if sum(sizes) > 16 * 1024**3:
        raise ValueError("fixture exceeds its 16 GiB byte budget")
    result = []
    for number, size in enumerate(sizes):
        directory = "wide" if number < 1400 else f"dir-{number % 95:03d}"
        path = f"{directory}/file-{number:04d}"
        result.append({"number": number, "key": PREFIX + path, "path": path, "size": size,
                       "mode": (0o644, 0o755, 0o600, 0o640)[number % 4],
                       "mtime": 1_700_000_000 + number, "uid": 1000, "gid": 1000})
    return result


def blocks(number, size):
    for offset in range(0, size, CHUNK):
        seed = f"geesefs-immutable-v1:{number}:{offset}".encode()
        yield hashlib.shake_256(seed).digest(min(CHUNK, size - offset))


def seed(s3, entries, temporary):
    def upload(entry):
        digest = hashlib.sha256()
        with tempfile.SpooledTemporaryFile(max_size=8 * CHUNK, dir=temporary) as body:
            for block in blocks(entry["number"], entry["size"]):
                digest.update(block)
                body.write(block)
            body.seek(0)
            metadata = {"mode": str(0o100000 | entry["mode"]), "mtime": str(entry["mtime"]),
                        "uid": str(entry["uid"]), "gid": str(entry["gid"])}
            response = s3.put_object(Bucket=BUCKET, Key=entry["key"], Body=body,
                                     ContentLength=entry["size"], Metadata=metadata)
        return dict(entry, sha256=digest.hexdigest(), etag=response["ETag"])
    result = []
    with ThreadPoolExecutor(max_workers=4) as pool:
        for entry in pool.map(upload, entries):
            result.append(entry)
            if len(result) % 250 == 0:
                print(json.dumps({"seeded_files": len(result)}), flush=True)
    return result


def verify(s3, manifest):
    def read(entry):
        response = s3.get_object(Bucket=BUCKET, Key=entry["key"], IfMatch=entry["etag"])
        digest, count = hashlib.sha256(), 0
        with contextlib.closing(response["Body"]) as body:
            while block := body.read(CHUNK):
                digest.update(block)
                count += len(block)
        assert count == entry["size"] and digest.hexdigest() == entry["sha256"], entry["path"]
        metadata = response["Metadata"]
        for field in ("mtime", "uid", "gid"):
            assert metadata[field] == str(entry[field]), (entry["path"], field)
        assert metadata["mode"] == str(0o100000 | entry["mode"]), entry["path"]
    with ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(read, manifest))


def list_names(s3, path):
    prefix = PREFIX + (path + "/" if path else "")
    names = set()
    for page in s3.get_paginator("list_objects_v2").paginate(Bucket=BUCKET, Prefix=prefix, Delimiter="/"):
        names.update(item["Key"][len(prefix):].rstrip("/") for item in page.get("Contents", []) if item["Key"] != prefix)
        names.update(item["Prefix"][len(prefix):].rstrip("/") for item in page.get("CommonPrefixes", []))
    return sorted(names)


def expected_listings(entries):
    directories = {}
    for entry in entries:
        directory, name = entry["path"].split("/")
        directories.setdefault(directory, []).append(name)
    return [{"path": "", "names": sorted(directories)}] + [
        {"path": path, "names": sorted(names)} for path, names in sorted(directories.items())]


def denied(call):
    try:
        call()
    except ClientError as error:
        assert error.response["ResponseMetadata"]["HTTPStatusCode"] == 403, error.response["Error"]
        return
    raise AssertionError("sealed fixture unexpectedly admitted a mutation")


def serve(args):
    signal.alarm(args.timeout)
    work, shared = Path(args.work), Path(args.shared)
    work.mkdir(exist_ok=True)
    shared.mkdir(exist_ok=True)
    temporary = work / "temporary"
    temporary.mkdir(exist_ok=True)
    data = work / "data"
    data.mkdir(exist_ok=True)
    reader = identity("fixture-reader", READER, ["Read", "List"])
    config = work / "s3.json"
    config.write_text(json.dumps({"identities": [reader, identity("fixture-seeder", SEEDER,
                           ["Admin", "Read", "List", "Write", "Tagging"])]}))
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith("AWS_") and key not in ("S3_BUCKET", "S3_TABLE_BUCKET")}
    environment.update(HOME=str(work), TMPDIR=str(temporary), GOMAXPROCS="4")
    command = [args.weed, "mini", f"-dir={data}", "-ip=127.0.0.1", "-ip.bind=127.0.0.1",
               f"-s3.config={config}", "-master.telemetry=false", "-webdav=false", "-admin.ui=false",
               "-s3.iam=false", "-s3.autoCreateBucket=false", "-s3.port.iceberg=0", "-s3.port.lance=0",
               "-master.volumeSizeLimitMB=1024", "-volume.max=16", "-s3.metricsPort=9328"]
    endpoint = "http://127.0.0.1:8333"
    process = None
    logs = []

    def start(label, seeding):
        log = (work / (label + "-server.log")).open("xb")
        logs.append(log)
        process = subprocess.Popen(command + ([f"-bucket={BUCKET}"] if seeding else []),
                                   cwd=work, env=environment, stdout=log, stderr=subprocess.STDOUT)
        until = time.monotonic() + 150
        while True:
            if process.poll() is not None:
                raise RuntimeError(f"native fixture {label} exited {process.returncode}; inspect server log")
            try:
                client(endpoint).list_objects_v2(Bucket=BUCKET, MaxKeys=1)
                return process
            except Exception:
                if time.monotonic() >= until:
                    process.terminate()
                    process.wait(timeout=45)
                    raise
                time.sleep(0.5)

    try:
        process = start("seed", True)
        entries = blueprint(json.loads(Path(args.sizes).read_text()))
        manifest = seed(client(endpoint, SEEDER), entries, temporary)
        process.terminate()
        process.wait(timeout=45)
        config.write_text(json.dumps({"identities": [reader]}))
        process = start("sealed", False)
        (shared / "backend.pid").write_text(str(process.pid) + "\n")
        s3 = client(endpoint)
        denied(lambda: s3.put_object(Bucket=BUCKET, Key="_permission-probe", Body=b"forbidden"))
        denied(lambda: s3.delete_object(Bucket=BUCKET, Key=manifest[0]["key"]))
        denied(lambda: s3.create_multipart_upload(Bucket=BUCKET, Key="_permission-probe"))
        denied(lambda: client(endpoint, SEEDER).put_object(Bucket=BUCKET, Key="_permission-probe", Body=b"forbidden"))
        verify(s3, manifest)
        # A wrong generation must reject, not serve bytes from another version.
        try:
            response = s3.get_object(Bucket=BUCKET, Key=manifest[0]["key"], IfMatch='"wrong-fixture-generation"')
            response["Body"].close()
            raise AssertionError("native fixture ignored If-Match")
        except ClientError as error:
            assert error.response["ResponseMetadata"]["HTTPStatusCode"] == 412
        listings = expected_listings(manifest)
        for entry in listings:
            assert list_names(s3, entry["path"]) == entry["names"], entry["path"]
        assert len(listings) == 97 and len(next(row["names"] for row in listings if row["path"] == "wide")) == 1400
        save(shared / "manifest.json", manifest)
        save(shared / "listings.json", listings)
        save(shared / "sealed.json", {"files": len(manifest), "bytes": sum(entry["size"] for entry in manifest),
             "listings": len(listings), "standard_listing": "v2", "read_only_mutations_rejected": 4,
             "if_match_rejected": True, "independent_full_hashes_verified": len(manifest),
             "manifest_sha256": hashlib.sha256((shared / "manifest.json").read_bytes()).hexdigest(),
             "server": "SeaweedFS 4.51", "seed_identity_removed": True})
        print(json.dumps({"sealed": True, "files": len(manifest), "bytes": sum(entry["size"] for entry in manifest)}), flush=True)
        while not (shared / "finished").exists():
            if process.poll() is not None:
                raise RuntimeError("sealed backend exited before benchmark completed")
            time.sleep(1)
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=45)
        for log in logs:
            log.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--weed", required=True)
    parser.add_argument("--sizes", required=True)
    parser.add_argument("--work", default="/work")
    parser.add_argument("--shared", default="/shared")
    parser.add_argument("--timeout", type=int, default=12800)
    serve(parser.parse_args())
