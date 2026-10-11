#!/usr/bin/env python3
"""Interleaved A/B benchmark driver with bootstrap CIs and a rank test.

Runs two or more harness binaries (Rust `ring_throughput`, C++ `bench_ring`,
or knob builds of either) in alternating order, round after round, so slow
drift of the machine affects every variant equally, then compares every
variant against the first one with distribution-free statistics.

    python3 scripts/bench_compare.py --pin 2,4 --rounds 10 --iters 4 \
        rust="target/release/examples/ring_throughput" \
        cpp="cpp/bench_ring"

Each variant command is run as `<cmd> <iters> <count> <capacity> --raw
--workload <w> [--pin P,C]` and must print one `raw <workload> <value>` line
per trial. Lower values are better for every workload.

Statistics (pure Python, no numpy/scipy, so the environment is reproducible):
  * median with a 95% bootstrap percentile CI (10,000 resamples, fixed seed)
  * ratio of medians vs the baseline with a bootstrap CI
  * Mann-Whitney U, two-sided, normal approximation with tie correction
  * Cliff's delta (effect size; |d| < 0.147 is conventionally "negligible")
"""
import argparse
import json
import math
import platform
import random
import shlex
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone

B = 10000  # bootstrap resamples


def bootstrap_median_ci(xs, rng, b=B, alpha=0.05):
    n = len(xs)
    meds = []
    for _ in range(b):
        sample = [xs[rng.randrange(n)] for _ in range(n)]
        meds.append(statistics.median(sample))
    meds.sort()
    lo = meds[int(math.floor(alpha / 2 * b))]
    hi = meds[min(b - 1, int(math.ceil((1 - alpha / 2) * b)) - 1)]
    return lo, hi


def bootstrap_ratio_ci(xs, ys, rng, b=B, alpha=0.05):
    """CI for median(ys) / median(xs), independent resampling of both."""
    nx, ny = len(xs), len(ys)
    ratios = []
    for _ in range(b):
        mx = statistics.median([xs[rng.randrange(nx)] for _ in range(nx)])
        my = statistics.median([ys[rng.randrange(ny)] for _ in range(ny)])
        ratios.append(my / mx)
    ratios.sort()
    lo = ratios[int(math.floor(alpha / 2 * b))]
    hi = ratios[min(b - 1, int(math.ceil((1 - alpha / 2) * b)) - 1)]
    return lo, hi


def rankdata(values):
    """Average ranks (1-based) with ties sharing the mean rank."""
    order = sorted(range(len(values)), key=lambda i: values[i])
    ranks = [0.0] * len(values)
    i = 0
    while i < len(values):
        j = i
        while j + 1 < len(values) and values[order[j + 1]] == values[order[i]]:
            j += 1
        avg = (i + j) / 2 + 1
        for k in range(i, j + 1):
            ranks[order[k]] = avg
        i = j + 1
    return ranks


def mann_whitney(xs, ys):
    """Two-sided Mann-Whitney U (normal approximation, tie-corrected)."""
    n1, n2 = len(xs), len(ys)
    allv = list(xs) + list(ys)
    ranks = rankdata(allv)
    r1 = sum(ranks[:n1])
    u1 = r1 - n1 * (n1 + 1) / 2
    u2 = n1 * n2 - u1
    u = min(u1, u2)
    mu = n1 * n2 / 2
    n = n1 + n2
    # tie correction
    counts = {}
    for v in allv:
        counts[v] = counts.get(v, 0) + 1
    tie_term = sum(c ** 3 - c for c in counts.values())
    sigma = math.sqrt(n1 * n2 / 12 * ((n + 1) - tie_term / (n * (n - 1))))
    if sigma == 0:
        return u, 1.0
    z = (u - mu + 0.5) / sigma  # continuity correction
    p = 2 * (1 - 0.5 * (1 + math.erf(abs(z) / math.sqrt(2))))
    return u, max(min(p, 1.0), 0.0)


def cliffs_delta(xs, ys):
    """P(y > x) - P(y < x). Positive means ys tend to be larger (slower)."""
    more = less = 0
    for x in xs:
        for y in ys:
            if y > x:
                more += 1
            elif y < x:
                less += 1
    return (more - less) / (len(xs) * len(ys))


def run_variant(cmd, iters, count, capacity, workload, pin):
    args = shlex.split(cmd) + [str(iters), str(count), str(capacity), "--raw", "--workload", workload]
    if pin:
        args += ["--pin", pin]
    proc = subprocess.run(args, capture_output=True, text=True, check=True)
    out = []
    for line in proc.stdout.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[0] == "raw" and parts[1] == workload:
            out.append(float(parts[2]))
    if len(out) != iters:
        sys.exit(f"{cmd}: expected {iters} raw lines for {workload}, got {len(out)}\n{proc.stdout}\n{proc.stderr}")
    return out


def fmt(x, unit):
    return f"{x:.3f} {unit}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("variants", nargs="+", help="name=command (first one is the baseline)")
    ap.add_argument("--pin", default=None, help="P,C passed to every harness run")
    ap.add_argument("--rounds", type=int, default=10)
    ap.add_argument("--iters", type=int, default=4, help="trials per harness invocation")
    ap.add_argument("--count", type=int, default=1_000_000)
    ap.add_argument("--capacity", type=int, default=8192)
    ap.add_argument("--workloads", default="u64,order,latency")
    ap.add_argument("--json", default=None, help="write all samples and stats here")
    ap.add_argument("--seed", type=int, default=20261011)
    a = ap.parse_args()

    variants = []
    for v in a.variants:
        if "=" not in v:
            sys.exit(f"variant must be name=command: {v}")
        name, cmd = v.split("=", 1)
        variants.append((name, cmd))
    workloads = a.workloads.split(",")
    units = {"u64": "ms", "order": "ms", "latency": "ns"}

    samples = {w: {name: [] for name, _ in variants} for w in workloads}
    t_start = time.time()
    for r in range(a.rounds):
        order = variants if r % 2 == 0 else list(reversed(variants))  # ABBA
        for w in workloads:
            for name, cmd in order:
                samples[w][name].extend(run_variant(cmd, a.iters, a.count, a.capacity, w, a.pin))
        print(f"round {r + 1}/{a.rounds} done ({time.time() - t_start:.0f}s)", file=sys.stderr)

    rng = random.Random(a.seed)
    result = {
        "date_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "pin": a.pin,
        "rounds": a.rounds,
        "iters_per_round": a.iters,
        "count": a.count,
        "capacity": a.capacity,
        "variants": dict(variants),
        "workloads": {},
    }

    base_name = variants[0][0]
    lines = []
    lines.append(f"Interleaved runs: {a.rounds} rounds x {a.iters} trials per variant and workload "
                 f"(n = {a.rounds * a.iters}), pin = {a.pin or 'none'}, count = {a.count}, capacity = {a.capacity}.")
    lines.append("")
    for w in workloads:
        unit = units.get(w, "")
        lines.append(f"### {w}")
        lines.append("")
        lines.append(f"| variant | n | median | 95% CI (bootstrap) | min | mean | vs {base_name}: ratio [CI] | Mann-Whitney p | Cliff's delta |")
        lines.append("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
        wres = {}
        xs = samples[w][base_name]
        for name, _ in variants:
            ys = samples[w][name]
            med = statistics.median(ys)
            lo, hi = bootstrap_median_ci(ys, rng)
            row = {
                "n": len(ys),
                "median": med,
                "ci95": [lo, hi],
                "min": min(ys),
                "mean": statistics.fmean(ys),
                "samples": ys,
            }
            if name != base_name:
                rlo, rhi = bootstrap_ratio_ci(xs, ys, rng)
                u, p = mann_whitney(xs, ys)
                d = cliffs_delta(xs, ys)
                row.update({"ratio_vs_base": med / statistics.median(xs), "ratio_ci95": [rlo, rhi],
                            "mann_whitney_u": u, "p_value": p, "cliffs_delta": d})
                cmp_cell = f"{med / statistics.median(xs):.3f} [{rlo:.3f}, {rhi:.3f}]"
                p_cell = f"{p:.4f}" if p >= 0.0001 else f"{p:.1e}"
                d_cell = f"{d:+.3f}"
            else:
                cmp_cell, p_cell, d_cell = "baseline", "", ""
            wres[name] = row
            lines.append(f"| {name} | {len(ys)} | {fmt(med, unit)} | [{lo:.3f}, {hi:.3f}] | {fmt(min(ys), unit)} | "
                         f"{fmt(statistics.fmean(ys), unit)} | {cmp_cell} | {p_cell} | {d_cell} |")
        lines.append("")
        result["workloads"][w] = wres

    print("\n".join(lines))
    if a.json:
        with open(a.json, "w", encoding="utf-8") as f:
            json.dump(result, f, indent=1)
        print(f"wrote {a.json}", file=sys.stderr)


if __name__ == "__main__":
    main()
