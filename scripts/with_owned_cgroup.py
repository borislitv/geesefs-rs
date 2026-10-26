#!/usr/bin/env python3
"""Prepare only this dedicated systemd user unit's delegated benchmark subtree.

Invoke through systemd-run --user with Delegate=cpu memory and RuntimeMaxSec.
The hosting service and its ancestors are never changed by this script.
"""
import os
from pathlib import Path
import sys


def main():
    if len(sys.argv) < 2:
        raise RuntimeError("a benchmark command is required")
    current = Path("/proc/self/cgroup").read_text().strip()
    if not current.startswith("0::/") or "\n" in current:
        raise RuntimeError("a unified cgroup v2 hierarchy is required")
    root = Path("/sys/fs/cgroup") / current.removeprefix("0::/")
    if not root.name.startswith("geesefs-rs-bench-") or not root.name.endswith(".service"):
        raise RuntimeError("refusing to configure anything except a dedicated geesefs-rs-bench-*.service")
    processes = (root / "cgroup.procs").read_text().split()
    if processes != [str(os.getpid())]:
        raise RuntimeError("dedicated unit must contain only this setup process")
    controllers = set((root / "cgroup.controllers").read_text().split())
    if not {"cpu", "memory"} <= controllers:
        raise RuntimeError("unit owner has not delegated both cpu and memory")
    runner = root / "runner"
    runner.mkdir()
    (runner / "cgroup.procs").write_text(str(os.getpid()))
    (root / "cgroup.subtree_control").write_text("+cpu +memory")
    environment = os.environ.copy()
    environment["GEESEFS_BENCH_CGROUP_ROOT"] = str(root)
    os.execvpe(sys.argv[1], sys.argv[1:], environment)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError) as error:
        print(f"owned benchmark cgroup setup failed: {error}", file=sys.stderr)
        sys.exit(1)
