#!/usr/bin/env python3
"""Fresh benchmark client: exec occurs after entering the owned cgroup.

Arguments/spec contain no credentials. Only explicit disposable credentials are
received through the sanitized environment. The worker stays alive until sampled.
"""
import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
import compare
import boto3
from botocore.config import Config

spec = json.loads(sys.stdin.readline())
client = boto3.client("s3", endpoint_url=spec["endpoint"], region_name="us-east-1",
                      aws_access_key_id=os.environ["AWS_ACCESS_KEY_ID"],
                      aws_secret_access_key=os.environ["AWS_SECRET_ACCESS_KEY"],
                       config=Config(connect_timeout=3, read_timeout=30, retries={"max_attempts": 1}, s3={"addressing_style": "path"}))


def run_workload(name):
    return compare.workload(Path(spec["path"]), name, spec["config"], client,
                            spec["bucket"], spec["prefix"], spec["endpoint"], spec["local"], spec["expected_sha"])


try:
    if spec.get("warmup"):
        # Warm the actual operation in the measured subtree, not an unrelated
        # payload scan. Prime a healthy writer before injecting its measured fault.
        run_workload("writer_fsync" if spec["name"] == "writer_failure" else spec["name"])
    print(json.dumps({"ready": True}), flush=True)
    sys.stdin.readline()
    try:
        metrics = run_workload(spec["name"])
        print(json.dumps({"metrics": metrics}), flush=True)
    except Exception as error:
        print(json.dumps({"error": str(error)}), flush=True)
    finally:
        sys.stdin.readline()
finally:
    client.close()
