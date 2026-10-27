#!/usr/bin/env python3
"""Summarize every successful and failed run without exposing mount arguments."""
import argparse
from collections import defaultdict
import json
import math
from pathlib import Path
import statistics

METRICS = ["throughput", "elapsed_seconds", "rss_peak_bytes", "rss_steady_bytes",
           "cgroup_peak_bytes", "cgroup_steady_bytes", "file_cache_steady_bytes", "kernel_steady_bytes",
           "allocator_peak_bytes", "allocator_steady_bytes", "p50_ms", "p95_ms", "p99_ms"]


def summarize(report):
    grouped = defaultdict(list)
    for run in report.get("runs", []):
        grouped[(run["baseline"], run["state"], run["workload"], run["implementation"])].append(run)
    groups = []
    for key, runs in sorted(grouped.items()):
        item = dict(zip(["baseline", "state", "workload", "implementation"], key))
        item.update(runs=len(runs), correct=sum(run.get("correctness") is True for run in runs),
                    errors=[{"repetition": run["repetition"], "error": run["error"]} for run in runs if run.get("error")])
        item["throughput_units"] = sorted({run.get("metrics", {}).get("throughput_unit", "unspecified") for run in runs})
        item["metrics"] = {}
        for metric in METRICS:
            values = [run["metrics"][metric] for run in runs
                      if isinstance(run.get("metrics", {}).get(metric), (int, float))
                      and not isinstance(run["metrics"][metric], bool)
                      and math.isfinite(run["metrics"][metric])]
            if values:
                item["metrics"][metric] = {"n": len(values), "missing": len(runs) - len(values),
                                           "mean": statistics.mean(values), "stdev": statistics.pstdev(values),
                                           "min": min(values), "max": max(values)}
            else:
                item["metrics"][metric] = {"n": 0, "missing": len(runs)}
        groups.append(item)
    return {"accepted": report.get("gate", {}).get("accepted", False),
            "runs": len(report.get("runs", [])),
            "gate_error_count": len(report.get("gate", {}).get("errors", [])),
            "provenance": {name: {key: item[key] for key in ["source_revision", "binary_sha256", "toolchain"] if key in item}
                           for name, item in report.get("provenance", {}).items()}, "groups": groups}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--table", action="store_true", help="compact throughput/elapsed/RSS table; full accounting remains in JSON")
    args = parser.parse_args()
    summary = summarize(json.loads(args.report.read_text()))
    if args.table:
        print(f"accepted={summary['accepted']} runs={summary['runs']} gate_errors={summary['gate_error_count']}")
        print("baseline state workload implementation correct/runs throughput_mean throughput_stdev elapsed_mean peak_rss_mean")
        for group in summary["groups"]:
            metrics = group["metrics"]
            print(group["baseline"][:8], group["state"], group["workload"], group["implementation"],
                  f"{group['correct']}/{group['runs']}",
                  *[round(metrics[name][stat], 3) if stat in metrics[name] else "missing"
                    for name, stat in [("throughput", "mean"), ("throughput", "stdev"),
                                       ("elapsed_seconds", "mean"), ("rss_peak_bytes", "mean")]])
    else:
        print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
