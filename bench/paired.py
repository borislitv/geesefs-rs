"""Mount-paired screening; cost ratios below one favor the candidate.

Resample whole independent mounts, never individual calls or cycles. Control
ratios must come from the same executable, workload and resource placement. A
noise-relative non-inferiority screen is not a practical production loss budget.
"""
from functools import lru_cache
import math
import random
import statistics

MIN_PAIRS = 8
RESAMPLES = 4096


def _ratios(values):
    result = tuple(values)
    if any(type(value) not in (int, float) or not math.isfinite(value) or value <= 0
           for value in result):
        raise ValueError("paired ratios must be finite positive numbers")
    return result


@lru_cache(maxsize=4096)
def _interval(values):
    if not values:
        return None
    logs = tuple(math.log(value) for value in values)
    generator = random.Random(20261102)
    estimates = sorted(statistics.fmean(generator.choices(logs, k=len(logs)))
                       for _ in range(RESAMPLES))
    return [math.exp(estimates[math.ceil(fraction * RESAMPLES) - 1])
            for fraction in (0.025, 0.975)]


def _sign_probability(values):
    positive = sum(value > 1 for value in values)
    negative = sum(value < 1 for value in values)
    count = positive + negative
    return min(1.0, 2 * sum(math.comb(count, index)
                           for index in range(min(positive, negative) + 1)) / 2**count)


def assess(ratios, controls, *, margin=None):
    """Report a paired geometric-mean effect and descriptive 95% bootstrap CI.

    Require eight actual pairs and eight same-binary controls for decisions.
    The sign test is separately reported; it targets direction consistency, not
    the geometric-mean effect. Four favorable signs have two-sided p=0.125.
    """
    values, aa = _ratios(ratios), _ratios(controls)
    _check_margin(margin)
    interval, control_interval = _interval(values), _interval(aa)
    noise_ratio = _noise_ratio(aa)
    result = dict(pairs=len(values), controls=len(aa), ratios=list(values),
                  geometric_mean=math.exp(statistics.fmean(map(math.log, values))) if values else None,
                  ci95=interval, control_ci95=control_interval, noise_ratio=noise_ratio,
                  sign_p_two_sided=_sign_probability(values), margin=margin)
    result["status"] = _status(len(values), len(aa), interval, control_interval, noise_ratio, margin)
    return result


def _check_margin(margin):
    if margin is not None and (type(margin) not in (int, float) or not math.isfinite(margin) or margin < 1):
        raise ValueError("non-inferiority margin must be a finite cost factor >= 1")


def _noise_ratio(controls):
    noise = sorted(abs(math.log(value)) for value in controls)
    return math.exp(noise[math.ceil(0.95 * len(noise)) - 1]) if noise else None


def _status(pairs, controls, interval, control_interval, noise_ratio, margin):
    if pairs < MIN_PAIRS:
        return "insufficient_pairs"
    if controls < MIN_PAIRS:
        return "insufficient_controls"
    if not control_interval[0] <= 1 <= control_interval[1]:
        return "biased_controls"
    if interval[1] < 1:
        return "improved"
    # Without a predeclared practical margin, noise alone never proves non-inferiority.
    limit = noise_ratio if margin is None else min(noise_ratio, margin)
    if interval[1] <= limit:
        return "within_noise" if margin is None else "non_inferior"
    return "regressed" if interval[0] > limit else "inconclusive"
