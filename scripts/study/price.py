#!/usr/bin/env python3
"""Project the cost of a block of cells from the cells already run.

    python3 scripts/study/price.py --from ~/jevify-study-d --tasks 17 \
        --block haiku:4:control,thin,available,required --block sonnet:2:control,thin,available,required

Agent-side figures come from the `meta.json` of every completed cell under
--from: per arm, the MEAN run (a sum is what a budget pays, and the tail is what
breaks one) and the median beside it. An arm with no cells yet is priced as the
arm named by --stand-in (thin as available by default). A model with no cells is
priced as haiku times --model-factor (sonnet's per-token price is 3x haiku's).

The keyless side is priced from the same cells' classification counts, mean per
run, against the per-IP daily allowance.
"""

import argparse
import json
import pathlib
import statistics

LIST_PRICE_FACTOR = {"haiku": 1.0, "sonnet": 3.0, "opus": 5.0}


def load(study):
    out = []
    for f in sorted(pathlib.Path(study).expanduser().glob("runs/*/meta.json")):
        m = json.loads(f.read_text())
        if m.get("turns") and m.get("rep", 0) > 0:
            m.setdefault("model", "haiku")
            out.append(m)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="src", action="append", required=True,
                    help="study tree(s) whose finished cells set the per-run price")
    ap.add_argument("--tasks", type=int, required=True)
    ap.add_argument("--block", action="append", required=True, help="model:reps:arm,arm,...")
    ap.add_argument("--stand-in", default="thin=available")
    ap.add_argument("--allowance", type=int, default=20000, help="keyless classifications per day")
    ap.add_argument("--exclude", default="", help="comma-separated task ids not in the block (D9)")
    a = ap.parse_args()

    skip = set(a.exclude.split(",")) - {""}
    metas = [m for s in a.src for m in load(s) if m["task"] not in skip]
    stand = dict(p.split("=") for p in a.stand_in.split(",") if p)
    print(f"priced from {len(metas)} finished cells")
    usd = cls = 0.0
    for spec in a.block:
        model, reps, arms = spec.split(":")
        for arm in arms.split(","):
            src_arm = arm if any(m["arm"] == arm for m in metas) else stand.get(arm, arm)
            ms = [m for m in metas if m["arm"] == src_arm and m["model"].startswith(model)]
            factor = 1.0
            if not ms:
                ms = [m for m in metas if m["arm"] == src_arm and m["model"].startswith("haiku")]
                factor = LIST_PRICE_FACTOR.get(model, 3.0)
            if not ms:
                print(f"  {model:7s} {arm:10s} no cells to price from")
                continue
            c = [m.get("agent_cost_usd") or 0 for m in ms]
            # a stand-in prices the agent, not the tool: jev spends one
            # classification per call and no envelope counts it, so it is not priced
            q = [0 if src_arm != arm else (m.get("jevify_questions") or 0) for m in ms]
            n = a.tasks * int(reps)
            usd += statistics.mean(c) * factor * n
            cls += statistics.mean(q) * n
            print(f"  {model:7s} {arm:10s} {n:4d} runs  priced as {src_arm}"
                  f"{'' if factor == 1 else f' x{factor:g}'} (n={len(ms)}): mean ${statistics.mean(c) * factor:.4f}"
                  f" median ${statistics.median(c) * factor:.4f} -> ${statistics.mean(c) * factor * n:6.2f};"
                  f" keyless {statistics.mean(q) * n:6.0f} classifications")
    print(f"\nprojected agent inference ${usd:.2f}")
    print(f"projected keyless {cls:.0f} classifications = {cls / a.allowance:.1%} of a {a.allowance:,}-a-day allowance")


if __name__ == "__main__":
    main()
