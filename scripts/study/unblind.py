#!/usr/bin/env python3
"""Join the blind scores to the arms and report each arm on its own.

    python3 scripts/study/unblind.py                       # the whole report
    python3 scripts/study/unblind.py --tasks D1,D2 --model haiku --max-rep 4
    python3 scripts/study/unblind.py --json summary.json   # the per-arm figures, machine-readable

Runs after score.py. Four arms, reported separately per model and never averaged:

    control    jevify not installed; the floor
    thin       the thin baseline `jev` installed and mentioned once: one keyless
               Jev call ranking the options the agent passes, nothing else
    available  jevify installed and mentioned once; ADOPTION is measured here
    required   the agent was told to use it; efficacy when used is measured here

Correctness is always k of n with a 95 % Wilson interval, never a bare count.

Every tool call is put in one exit class. jevify's own contract: 0 ok, 1 no,
2 usage, 3 abstain, 4 unavailable, 5 auth, 6 input. A `fill` call runs a child
command after resolving its markers, and the child owns the exit code from then
on: when the call's --dry-run shadow resolved (exit 0) and the real call exited
non-zero, or the exit is outside the contract (git's 128), the class is `child`,
not jevify's. `usage`, `input` and `child` together are shape failures -- the
agent asked in a form that could not work -- and are counted apart from
abstentions, which are the tool saying nothing fits.

For each tool arm: how often the tool's top answer named the gold, and how often
it named the gold and the agent answered it or answered otherwise. For `jev` the
top answer is its first line; for jevify it is what the call printed.
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
ARMS = ("control", "thin", "available", "required")
TOOL_ARMS = ("thin", "available", "required")
JEVIFY_EXIT = {0: "ok", 1: "no", 2: "usage", 3: "abstain", 4: "unavailable", 5: "auth", 6: "input",
               130: "declined"}
SHAPE = ("usage", "input", "child")


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


_SHA = {}


def full_sha(repo, ref):
    if not ref or not all(c in "0123456789abcdefABCDEF" for c in ref) or len(ref) < 7:
        return None
    if (repo, ref) not in _SHA:
        p = subprocess.run(["git", "-C", str(STUDY / "corpus" / repo), "rev-parse", "--verify", f"{ref}^{{commit}}"],
                           capture_output=True, text=True)
        _SHA[(repo, ref)] = p.stdout.strip() if p.returncode == 0 else None
    return _SHA[(repo, ref)]


def matches_gold(task, text):
    """Does this blob of text name the gold answer? Used on what a tool printed,
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
    """What the tool showed as its answer. jev ranks every option it was given,
    so only its first line is an answer; jevify prints what it chose, plus the
    shadow envelope when the agent did not ask for one."""
    if call.get("tool") == "jev":
        first = (call.get("stdout_head") or "").split("\n", 1)[0]
        return first.split("\t", 1)[1] if "\t" in first else ""
    sh = call.get("shadow") or {}
    return "\n".join([call.get("stdout_head") or "", sh.get("stdout_head") or ""])


def exit_class(call):
    ex = call.get("exit")
    if call.get("tool") == "jev":
        return {0: "ok", 2: "usage", 4: "unavailable"}.get(ex, f"exit-{ex}")
    sh = call.get("shadow") or {}
    if call.get("verb") == "fill" and ex not in (0, None) and sh.get("exit") == 0:
        return "child"
    return JEVIFY_EXIT.get(ex, "child")


def med(xs):
    return statistics.median(xs) if xs else 0


def summarise(rs):
    ok = [r for r in rs if r["score"]["status"] != "refused_leak"]
    k, n = sum(r["score"]["correct"] or 0 for r in ok), len(ok)
    lo, hi = wilson(k, n)
    g = lambda key: [r["meta"].get(key) or 0 for r in rs]  # noqa: E731
    calls = [c for r in rs for c in r["calls"]]
    classes = collections.Counter(exit_class(c) for c in calls)
    adopted = [r for r in rs if r["calls"]]
    named = [r for r in adopted if any(matches_gold(TASKS[r["task"]], printed(c)) for c in r["calls"])]
    became = [r for r in named if r["score"]["correct"] == 1]
    s = {
        "runs": len(rs), "correct": k, "scored": n, "refused": len(rs) - n,
        "wilson": [round(lo, 3), round(hi, 3)],
        "adopted": len(adopted), "adoption_wilson": [round(x, 3) for x in wilson(len(adopted), len(rs))],
        "calls": len(calls), "calls_per_adopting_run": round(len(calls) / len(adopted), 2) if adopted else 0,
        "exit_classes": dict(classes),
        "abstentions": classes.get("abstain", 0),
        "shape_failures": sum(classes.get(c, 0) for c in SHAPE),
        "tool_named_gold": len(named), "tool_named_gold_agent_right": len(became),
        "tool_named_gold_agent_wrong": len(named) - len(became),
        "keyless_requests": sum(g("jevify_requests")), "keyless_classifications": sum(g("jevify_questions")),
        "jev_calls": sum(1 for c in calls if c.get("tool") == "jev"),
    }
    for key in ("turns", "input_tokens", "wall_s", "agent_cost_usd"):
        s[key] = {"sum": round(sum(g(key)), 4), "median": round(med(g(key)), 4)}
    s["stops"] = dict(collections.Counter(str(r["meta"].get("stop")) for r in rs))
    return s


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tasks", help="comma-separated task ids to report; default all present")
    ap.add_argument("--model", help="report one model only (haiku, sonnet)")
    ap.add_argument("--arms", default=",".join(ARMS))
    ap.add_argument("--max-rep", type=int, default=None,
                    help="ignore repetitions above this. Pilot and canary cells live in the same "
                         "runs/ directory and are numbered above the study's, and nothing may be "
                         "deleted from a finished study tree, so they are excluded by number.")
    ap.add_argument("--json", help="write the per-(model, arm) summary here")
    ap.add_argument("--quiet", action="store_true", help="no per-run table and no call list")
    a = ap.parse_args()
    keep = set(a.tasks.split(",")) if a.tasks else None
    arms = [x for x in ARMS if x in a.arms.split(",")]

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
        info = {**info, "model": info.get("model") or runs[info["rid"]][1].get("model", "haiku")}
        if keep and info["task"] not in keep:
            continue
        if info["arm"] not in arms or (a.model and info["model"] != a.model):
            continue
        if info["rep"] < 1 or (a.max_rep is not None and info["rep"] > a.max_rep):
            continue
        d, meta = runs[info["rid"]]
        rows.append({**info, "score": scored[bid], "dir": d, "meta": meta,
                     "calls": calls_of(d), "answer": (d / "answer.txt").read_text().strip()})
    rows.sort(key=lambda r: (r["model"], r["task"], ARMS.index(r["arm"]), r["rep"]))
    if not rows:
        raise SystemExit("no runs match; run run_parallel.sh, blind.py and score.py first")
    models = sorted({r["model"] for r in rows})

    if not a.quiet:
        print(f"{'rid':30s} {'status':13s} {'ok':3s} {'turns':>5s} {'in_tok':>9s} "
              f"{'wall_s':>7s} {'agent$':>8s} {'calls':>5s} {'exits':14s}")
        for r in rows:
            meta, sc = r["meta"], r["score"]
            print(f"{r['rid']:30s} {sc['status']:13s} {str(sc['correct']):3s} "
                  f"{str(meta.get('turns')):>5s} {str(meta.get('input_tokens')):>9s} {str(meta.get('wall_s')):>7s} "
                  f"{meta.get('agent_cost_usd') or 0:8.4f} {len(r['calls']):5d} "
                  f"{','.join(exit_class(c) for c in r['calls'])[:40]}")

    out = {}
    for model in models:
        print(f"\n===== model {model}")
        mrows = [r for r in rows if r["model"] == model]
        print(f"{'cell':22s} " + " ".join(f"{x:>11s}" for x in arms))
        for task in sorted({r["task"] for r in mrows}, key=lambda t: (t[0], int(t[1:]))):
            cells = []
            for arm in arms:
                rs = [r for r in mrows if r["task"] == task and r["arm"] == arm]
                k = sum(r["score"]["correct"] or 0 for r in rs)
                cells.append(f"{k}/{len(rs)}" if rs else "-")
            print(f"{task:22s} " + " ".join(f"{c:>11s}" for c in cells))
        print(f"\n{'arm':10s} {'k/n':>8s}  95% Wilson    adopt  calls  shape  abst  named/right/wrong"
              f"   turns[med]   in_tok[med]      wall[med]   usd[med]")
        for arm in arms:
            rs = [r for r in mrows if r["arm"] == arm]
            if not rs:
                continue
            s = summarise(rs)
            out[f"{model}/{arm}"] = s
            print(f"{arm:10s} {s['correct']:3d}/{s['scored']:<4d} [{s['wilson'][0]:.2f}, {s['wilson'][1]:.2f}]  "
                  f"{s['adopted']:2d}/{s['runs']:<3d} {s['calls']:5d} {s['shape_failures']:6d} {s['abstentions']:5d}  "
                  f"{s['tool_named_gold']:3d}/{s['tool_named_gold_agent_right']}/{s['tool_named_gold_agent_wrong']}"
                  f"      {s['turns']['sum']:5.0f}[{s['turns']['median']:.0f}] "
                  f"{s['input_tokens']['sum']:11,.0f}[{s['input_tokens']['median']:,.0f}] "
                  f"{s['wall_s']['sum']:7.0f}[{s['wall_s']['median']:.0f}] "
                  f"{s['agent_cost_usd']['sum']:7.2f}[{s['agent_cost_usd']['median']:.3f}]")
            if s["exit_classes"]:
                print(f"{'':10s} exits {s['exit_classes']}  stops {s['stops']}")

    if not a.quiet:
        print("\ntool calls that abstained, failed on shape, or did not name the gold")
        for r in rows:
            for i, c in enumerate(r["calls"], 1):
                cls = exit_class(c)
                if cls == "ok" and matches_gold(TASKS[r["task"]], printed(c)):
                    continue
                err = " ".join((c.get("stderr_head") or "").split())[:110]
                outp = " ".join((c.get("stdout_head") or "").split())[:110]
                print(f"  {r['rid']} call {i}: {cls}\n"
                      f"      argv    {' '.join(c.get('argv') or [])[:150]}\n"
                      f"      stdout  {outp or '(nothing)'}\n"
                      f"      stderr  {err or '(nothing)'}\n"
                      f"      gold    {TASKS[r['task']]['gold']}   agent answered {r['answer'] or '(nothing)'}")
    if a.json:
        pathlib.Path(a.json).write_text(json.dumps(out, indent=1))
        print(f"\nsummary -> {a.json}")


if __name__ == "__main__":
    main()
