#!/usr/bin/env python3
"""Join the blind scores to the arms and report each arm on its own.

    python3 scripts/study/unblind.py            # the whole report
    python3 scripts/study/unblind.py --tasks D1,D2

Runs after score.py. Three arms, reported separately and never averaged:

    control    jevify not installed; the floor
    available  jevify installed and mentioned once; ADOPTION is measured here
    required   the agent was told to use it; efficacy when used is measured here

Correctness is always k of n with a 95 % Wilson interval, never a bare count.
The adoption rate is the number this study exists to get: an agent that was
given the tool and reached for grep instead is the finding, whatever the
correctness columns say.

For the required arm two further figures: how often jevify was called at all,
and how often what jevify printed became the agent's answer. The second needs
the tool's own output, which the wrapper records, because a run can call the
tool, ignore it, and still be right.
"""

import argparse
import collections
import json
import math
import os
import pathlib
import re
import statistics
import subprocess

REPO = pathlib.Path(__file__).resolve().parents[2]
ARMS = ("control", "available", "required")
ABSTAIN = 3  # jevify's exit code for "nothing fits, or unsure"


def _study_dir():
    p = pathlib.Path(os.environ.get("JEVSTUDY") or (pathlib.Path.home() / "jevify-study")).resolve()
    if not p.is_dir():
        raise SystemExit(f"JEVSTUDY is not a directory: {p}")
    return p


STUDY = _study_dir()
TASKS = {t["id"]: t for t in (json.loads(l) for l in (REPO / "scripts/study/tasks.jsonl").read_text().splitlines() if l.strip())}


def wilson(k, n, z=1.96):
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    d = 1 + z * z / n
    c = p + z * z / (2 * n)
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return ((c - h) / d, (c + h) / d)


def full_sha(repo, ref):
    if not ref or not all(c in "0123456789abcdefABCDEF" for c in ref) or len(ref) < 7:
        return None
    p = subprocess.run(["git", "-C", str(STUDY / "corpus" / repo), "rev-parse", "--verify", f"{ref}^{{commit}}"],
                       capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else None


def matches_gold(task, text):
    """Does this blob of text name the gold answer? Used on what jevify printed,
    never on what the agent answered: the agent's answer is scored blind."""
    if not text:
        return False
    kind, gold, repo = task["kind"], task["gold"], task["repo"]
    if kind == "commit":
        want = full_sha(repo, gold)
        return any(full_sha(repo, m) == want for m in set(re.findall(r"\b[0-9a-f]{7,40}\b", text)))
    if kind == "path":
        return gold in text
    return re.search(rf"(^|[^\w/-]){re.escape(gold)}($|[^\w/-])", text) is not None


def calls_of(run):
    f = run / "jevify.jsonl"
    if not f.exists():
        return []
    return [json.loads(l) for l in f.read_text().splitlines() if l.strip()]


def printed(call):
    """Everything jevify showed the agent for this call: the plain output it
    replayed, plus the matches its envelope named."""
    parts = [call.get("stdout_head") or ""]
    sh = call.get("shadow") or {}
    parts.append(sh.get("stdout_head") or "")
    return "\n".join(parts)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tasks", help="comma-separated task ids to report; default all present")
    ap.add_argument("--max-rep", type=int, default=None,
                    help="ignore repetitions above this. Pilot and canary cells live in the same "
                         "runs/ directory and are numbered above the study's, and nothing may be "
                         "deleted from a finished study tree, so they are excluded by number.")
    a = ap.parse_args()
    keep = set(a.tasks.split(",")) if a.tasks else None

    m = json.loads((STUDY / "unblind_map.json").read_text())
    scored = {r["bid"]: r for r in (json.loads(l) for l in (STUDY / "scored.jsonl").read_text().splitlines() if l.strip())}
    runs = {}
    for d in sorted((STUDY / "runs").iterdir()):
        f = d / "meta.json"
        if f.exists():
            meta = json.loads(f.read_text())
            runs[meta["rid"]] = (d, meta)

    rows = []
    for bid, info in m.items():
        if bid not in scored or info["rid"] not in runs:
            continue
        if keep and info["task"] not in keep:
            continue
        if info["arm"] not in ARMS:
            continue
        if a.max_rep is not None and info["rep"] > a.max_rep:
            continue
        d, meta = runs[info["rid"]]
        rows.append({**info, "score": scored[bid], "dir": d, "meta": meta,
                     "calls": calls_of(d), "answer": (d / "answer.txt").read_text().strip()})
    rows.sort(key=lambda r: (r["task"], ARMS.index(r["arm"]), r["rep"]))
    if not rows:
        raise SystemExit("no runs in the three arms; run run_study.sh first")

    print(f"{'rid':26s} {'status':13s} {'ok':3s} {'turns':>5s} {'tools':>5s} {'in_tok':>8s} "
          f"{'wall_s':>7s} {'agent$':>8s} {'jev':>4s} {'req':>4s} {'used':>5s} {'mtime':>6s}")
    for r in rows:
        meta, sc = r["meta"], r["score"]
        used = "-" if not r["calls"] else ("yes" if any(matches_gold(TASKS[r["task"]], printed(c)) and
                                                       matches_gold(TASKS[r["task"]], r["answer"])
                                                       for c in r["calls"]) else "no")
        print(f"{r['rid']:26s} {sc['status']:13s} {str(sc['correct']):3s} "
              f"{str(meta.get('turns')):>5s} {str(meta.get('tool_uses')):>5s} "
              f"{str(meta.get('input_tokens')):>8s} {str(meta.get('wall_s')):>7s} "
              f"{meta.get('agent_cost_usd') or 0:8.4f} {str(meta.get('jevify_calls')):>4s} "
              f"{str(meta.get('jevify_requests')):>4s} {used:>5s} "
              f"{len(meta.get('mtime_moved_outside_run') or []):>6d}")

    # ---- correctness per cell, then per arm
    cells = collections.defaultdict(list)
    for r in rows:
        cells[(r["task"], r["arm"])].append(r)
    print(f"\n{'cell':26s} {'k/n':>7s}  95% Wilson       refused")
    for (task, arm), rs in sorted(cells.items(), key=lambda kv: (kv[0][0], ARMS.index(kv[0][1]))):
        ok = [r for r in rs if r["score"]["status"] != "refused_leak"]
        k, n = sum(r["score"]["correct"] or 0 for r in ok), len(ok)
        lo, hi = wilson(k, n)
        print(f"{task + ' ' + arm:26s} {k:3d}/{n:<3d}  [{lo:.2f}, {hi:.2f}]   "
              f"{len(rs) - len(ok)}")

    print(f"\n{'arm':10s} {'k/n':>8s}  95% Wilson      {'turns':>6s} {'in_tok':>10s} "
          f"{'wall_s':>8s} {'agent$':>8s}   (sums; median in brackets)")
    for arm in ARMS:
        rs = [r for r in rows if r["arm"] == arm]
        if not rs:
            continue
        ok = [r for r in rs if r["score"]["status"] != "refused_leak"]
        k, n = sum(r["score"]["correct"] or 0 for r in ok), len(ok)
        lo, hi = wilson(k, n)
        g = lambda key: [r["meta"].get(key) or 0 for r in rs]  # noqa: E731
        print(f"{arm:10s} {k:3d}/{n:<4d}  [{lo:.2f}, {hi:.2f}]   "
              f"{sum(g('turns')):6d} {sum(g('input_tokens')):10,d} {sum(g('wall_s')):8.1f} "
              f"{sum(g('agent_cost_usd')):8.4f}")
        print(f"{'':10s} {'':8s}  {'':16s}   "
              f"[{statistics.median(g('turns')):.0f}] [{statistics.median(g('input_tokens')):,.0f}] "
              f"[{statistics.median(g('wall_s')):.1f}] [{statistics.median(g('agent_cost_usd')):.4f}]")

    # ---- the number this study exists to get
    av = [r for r in rows if r["arm"] == "available"]
    if av:
        used = [r for r in av if r["calls"]]
        lo, hi = wilson(len(used), len(av))
        print(f"\nADOPTION, available arm: {len(used)}/{len(av)} runs called jevify at least once "
              f"= {len(used) / len(av):.0%}  95% Wilson [{lo:.2f}, {hi:.2f}]")
        by = collections.Counter(r["task"] for r in used)
        print("  per task: " + "  ".join(
            f"{t}:{by.get(t, 0)}/{sum(1 for r in av if r['task'] == t)}"
            for t in sorted({r['task'] for r in av})))

    rq = [r for r in rows if r["arm"] == "required"]
    if rq:
        called = [r for r in rq if r["calls"]]
        became = [r for r in called
                  if any(matches_gold(TASKS[r["task"]], printed(c)) for c in r["calls"])
                  and matches_gold(TASKS[r["task"]], r["answer"])]
        ignored = [r for r in called
                   if any(matches_gold(TASKS[r["task"]], printed(c)) for c in r["calls"])
                   and not matches_gold(TASKS[r["task"]], r["answer"])]
        print(f"\nREQUIRED arm: called jevify {len(called)}/{len(rq)}; "
              f"jevify named the gold and the agent answered it {len(became)}/{len(rq)}; "
              f"jevify named the gold and the agent answered otherwise {len(ignored)}/{len(rq)}")

    # ---- every jevify call that abstained or named something other than the gold
    print("\njevify calls that abstained or did not name the gold")
    bad = 0
    for r in rows:
        for i, c in enumerate(r["calls"], 1):
            ex = c.get("exit")
            names = matches_gold(TASKS[r["task"]], printed(c))
            if ex == ABSTAIN or not names:
                bad += 1
                why = "abstained" if ex == ABSTAIN else f"exit {ex}, did not name the gold"
                out = " ".join((c.get("stdout_head") or "").split())[:110]
                print(f"  {r['rid']} call {i}: {why}\n"
                      f"      argv   {' '.join(c.get('argv') or [])[:150]}\n"
                      f"      printed {out or '(nothing)'}\n"
                      f"      gold   {TASKS[r['task']]['gold']}   agent answered {r['answer'] or '(nothing)'}")
    if not bad:
        print("  none")

    moved = [(r["rid"], r["meta"].get("mtime_moved_outside_run") or []) for r in rows
             if r["meta"].get("mtime_moved_outside_run")]
    print(f"\nruns with an mtime moving outside their directory: {len(moved)} of {len(rows)}"
          " (an mtime, not a writer: see run_cell.scan; canary.py is the containment evidence)")
    for rid, files in moved[:10]:
        print(f"  {rid}: {files[:4]}")


if __name__ == "__main__":
    main()
