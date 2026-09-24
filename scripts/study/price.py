#!/usr/bin/env python3
"""Project the cost of the full study from the validation runs and the probe.

    python3 scripts/study/price.py --tasks 8 --reps 5 [--probe probe.txt]

Agent-side figures come from the `meta.json` of every completed cell: the
median per run per arm, not the mean, because with six runs one long run moves
a mean by a third. Backend figures cannot come from those cells when the with
arm did not call jevify, so the keyless side is priced from `probe.py`, which
measures what one adopted call costs per task, and is reported as a ceiling:
every with-arm run makes one call. Adoption below that scales it down linearly.
"""

import argparse
import json
import os
import pathlib
import statistics
import subprocess
import sys

def _study_dir():
    """$JEVSTUDY decides which binary and which repositories this touches, so it is
    resolved and required to be an existing directory before anything uses it."""
    p = pathlib.Path(os.environ.get("JEVSTUDY") or (pathlib.Path.home() / "jevify-study")).resolve()
    if not p.is_dir():
        raise SystemExit(f"JEVSTUDY is not a directory: {p}")
    return p


STUDY = _study_dir()


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tasks", type=int, default=8)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--probe-questions", type=int, default=220,
                    help="classifications for one adopted call on each of the 8 tasks, summed")
    ap.add_argument("--probe-requests", type=int, default=110)
    ap.add_argument("--allowance", type=int, default=20000)
    ap.add_argument("--scan-s", type=float, default=9.0)
    a = ap.parse_args()

    metas = []
    for run in sorted((STUDY / "runs").iterdir()):
        f = run / "meta.json"
        if f.exists():
            m = json.loads(f.read_text())
            if m.get("turns") and m.get("rep", 0) > 0:
                metas.append(m)

    cells = a.tasks * a.reps
    print(f"validation cells priced from: {len(metas)}")
    print(f"projection: {a.tasks} tasks x {a.reps} reps x 2 arms = {cells * 2} runs\n")
    tot_w = tot_i = tot_c = 0.0
    for arm in ("with", "without"):
        ms = [m for m in metas if m["arm"] == arm]
        if not ms:
            continue
        w, t, i, c = (med([m["wall_s"] for m in ms]), med([m["turns"] for m in ms]),
                      med([m["input_tokens"] for m in ms]), med([m["agent_cost_usd"] for m in ms]))
        tot_w += w * cells
        tot_i += i * cells
        tot_c += c * cells
        print(f"{arm:8s} n={len(ms)}  median run: wall {w:5.1f}s  turns {t:3.0f}  "
              f"input {i:8,.0f} tok  ${c:.4f}")
        print(f"{'':8s}       x {cells} runs: wall {w * cells / 60:6.1f} min  "
              f"input {i * cells / 1e6:6.2f} Mtok  ${c * cells:6.2f}")

    scan = a.scan_s * cells * 2
    print(f"\nagent side, sequential: {(tot_w + scan) / 60:.0f} min wall "
          f"({tot_w / 60:.0f} min of runs plus {scan / 60:.0f} min of post-run scans)")
    print(f"                        {tot_i / 1e6:.1f} M input tokens, ${tot_c:.2f} of agent inference")

    q = a.probe_questions * a.reps
    r = a.probe_requests * a.reps
    print(f"\nkeyless ceiling (every with-arm run makes one call, caches are per run):")
    print(f"  {q} classifications and {r} requests over the whole study")
    print(f"  = {q / a.allowance * 100:.1f} % of the {a.allowance:,}-a-day allowance as classifications,"
          f" {r / a.allowance * 100:.1f} % as requests")
    print(f"  at the adoption the last pilot saw (5 of 8 runs): "
          f"{q * 5 // 8} classifications, {r * 5 // 8} requests")
    print(f"  at the adoption this validation saw (0 of 6 runs): 0")


if __name__ == "__main__":
    main()
