"""Counter-based process/cgroup v2 CPU windows, independent of wall throughput."""
import math
from pathlib import Path
import time

import psutil


class CpuWindow:
    def __init__(self, processes, group=None, *, clock_ns=time.monotonic_ns):
        self.processes = dict(processes)
        self.group = Path(group) if group is not None else None
        self.clock_ns = clock_ns
        self.before = self.after = None

    def snapshot(self):
        processes = {}
        for role, pid in self.processes.items():
            process = psutil.Process(pid)
            times = process.cpu_times()
            if not process.is_running():
                raise RuntimeError("CPU process exited during counter collection")
            # children_user/system are intentionally excluded: owned cgroup
            # totals include all processes without double-counting their parents.
            processes[role] = {"pid": pid, "created": process.create_time(),
                               "user": times.user, "system": times.system}
        group = None
        if self.group is not None:
            group = {name: int(value) for name, value in
                     (line.split() for line in (self.group / "cpu.stat").read_text().splitlines())}
            for key in ("usage_usec", "user_usec", "system_usec"):
                if key not in group:
                    raise RuntimeError(f"CPU accounting missing cgroup {key}")
        return {"monotonic_ns": self.clock_ns(), "processes": processes, "cgroup": group}

    def start(self):
        self.before = self.snapshot()
        return self

    def finish(self):
        self.after = self.snapshot()
        return self

    def metrics(self):
        if self.before is None or self.after is None:
            raise RuntimeError("CPU accounting needs both window boundaries")
        wall = (self.after["monotonic_ns"] - self.before["monotonic_ns"]) / 1e9
        if not math.isfinite(wall) or wall <= 0:
            raise RuntimeError("CPU accounting needs a positive monotonic window")
        result = {"cpu_wall_seconds": wall}
        for role in self.processes:
            before, after = self.before["processes"][role], self.after["processes"][role]
            if before["pid"] != after["pid"] or before["created"] != after["created"]:
                raise RuntimeError("CPU process identity changed during measurement")
            user, system = after["user"] - before["user"], after["system"] - before["system"]
            if any(not math.isfinite(value) or value < 0 for value in (user, system)):
                raise RuntimeError("CPU process counters decreased or are invalid")
            result.update({role + "_cpu_user_seconds": user, role + "_cpu_system_seconds": system,
                           role + "_cpu_seconds": user + system, role + "_average_cores": (user + system) / wall})
        fields = {"usage_usec": ("cgroup_cpu_seconds", 1e6),
                  "user_usec": ("cgroup_cpu_user_seconds", 1e6),
                  "system_usec": ("cgroup_cpu_system_seconds", 1e6),
                  "throttled_usec": ("cgroup_throttled_seconds", 1e6),
                  "nr_throttled": ("cgroup_nr_throttled", 1), "nr_periods": ("cgroup_nr_periods", 1)}
        for key, (name, divisor) in fields.items():
            before, after = self.before["cgroup"], self.after["cgroup"]
            if before is None or after is None or key not in before or key not in after:
                result[name] = None
                continue
            delta = after[key] - before[key]
            if delta < 0:
                raise RuntimeError("CPU cgroup counters decreased during measurement")
            result[name] = delta / divisor
        total = result["cgroup_cpu_seconds"]
        result["cgroup_average_cores"] = total / wall if total is not None else None
        return result


def cpu_efficiency(metrics, *, transferred, operations):
    result = dict(metrics)
    for role in ("daemon", "client", "cgroup"):
        total = metrics.get(role + "_cpu_seconds")
        result[role + "_cpu_seconds_per_gib"] = total * (1024 ** 3) / transferred if total is not None and transferred > 0 else None
        result[role + "_cpu_seconds_per_operation"] = total / operations if total is not None and operations > 0 else None
    return result
