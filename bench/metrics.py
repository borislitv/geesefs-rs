"""Shared measurement validation and nearest-rank latency accounting."""
import math
from dataclasses import dataclass


def finite_nonnegative(value):
    return (isinstance(value, (int, float)) and not isinstance(value, bool)
            and math.isfinite(value) and value >= 0)


@dataclass
class SampledBytes:
    """Constant-memory peak/late gauges with explicit missing observations."""
    minimum_count: int = 1
    count: int = 0
    maximum: float = 0
    late_count: int = 0
    late_total: float = 0
    complete: bool = True
    late_complete: bool = True

    def record(self, value, *, late):
        self.count += 1
        if not finite_nonnegative(value):
            self.complete = False
            self.late_complete &= not late
            return
        self.maximum = max(self.maximum, value)
        if late:
            self.late_count += 1
            self.late_total += value

    @property
    def enough(self):
        return self.count >= self.minimum_count

    @property
    def peak(self):
        return self.maximum if self.complete and self.enough else None

    @property
    def late_mean(self):
        if not self.enough or not self.late_complete or not self.late_count:
            return None
        return self.late_total / self.late_count


def latency_samples(metrics):
    values = metrics.get("latencies_ms")
    operations = metrics.get("operations")
    if (type(operations) is not int or not isinstance(values, list) or not values
            or len(values) != operations
            or any(not finite_nonnegative(value) for value in values)):
        return None
    return values


def latency_percentiles(values):
    ordered = sorted(values)
    return {metric: ordered[math.ceil(len(ordered) * fraction) - 1]
            for metric, fraction in [("p50_ms", .5), ("p95_ms", .95), ("p99_ms", .99)]} if ordered else {}
