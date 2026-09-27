#!/usr/bin/env python3
"""Record, per task, that the searches a competent engineer would try do not find
the answer.

    python3 scripts/study/baseline.py                 # every D task
    python3 scripts/study/baseline.py D1 D8           # a subset
    python3 scripts/study/baseline.py --json out.json

A task that a literal search answers is a task where jevify has nothing to add,
so it does not belong in the set. This script is what decides that, and it runs
before any cell does. It does not use the gold answer to build a search: it uses
the question, the same words the agent under test is given.

Two kinds of probe, both run inside the pinned clone:

  derived   every content word of the question, one probe each, through
            `git log --grep`, `git log -S`, `git log --all --grep`,
            `git grep -il`, `git ls-files | grep` and, for a branch task, the
            subject of every remote ref. This is the mechanical part: nobody
            chose these words, the question did.
  guessed   a handful of identifiers a reader of the question might guess at,
            written down per task by hand and kept even when they succeed.
            `git log -S'math.MaxInt32'` is the sort of probe that turns a task
            down, so it is asked on purpose.

A probe VERDICT is one of:

  miss      the probe returned nothing, or nothing that names the gold
  buried    the gold is in the output, among `hits` candidates, too many for the
            probe to be called an answer (the threshold is 3)
  HIT       the gold is in the output and the probe returns at most 3 candidates

A task with any HIT is reported as `lexically reachable` and must be dropped.
For a branch task a commit-level probe is resolved the way an engineer would
resolve it: every commit the probe returns is mapped to the remote branches that
contain it, and the probe counts as finding the answer only if that set of
branches is small and holds the gold.
"""

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]


def _study_dir():
    p = pathlib.Path(os.environ.get("JEVSTUDY") or (pathlib.Path.home() / "jevify-study")).resolve()
    if not p.is_dir():
        raise SystemExit(f"JEVSTUDY is not a directory: {p}")
    return p


STUDY = _study_dir()
TASKS = {t["id"]: t for t in (json.loads(l) for l in (REPO / "scripts/study/tasks.jsonl").read_text().splitlines() if l.strip())}

# words the question spends on the harness's own framing, plus ordinary English.
# They are dropped before probing because probing them measures the question
# template, not the task.
STOP = set("""a an the and or of to in at for from with without on by as that which whose
this these those is are was were be been being it its not no nor but if then than so such
one two both each every all any some other another same single find search history checked
out repository repositories git source file files commit commits branch branches remote
work works working question answer value line lines part parts thing things must may can
cannot what when where who how why into onto over under about above below after before
during while until unless because although though also just only very much more most less
least few many several own use used uses using make makes made take takes taken give gives
given get gets got let lets letting does do did done has have had need needs needed
you your our their his her they them we us i me my mine
""".split())

# identifiers a reader of the question might guess at. Written down by hand and
# kept in the record whether they succeed or fail.
GUESSED = {
    "D1": [["git", "log", "--all", "--oneline", "-i", "--grep=nan"],
           ["git", "log", "--all", "--oneline", "-i", "--grep=inf"],
           ["git", "log", "--all", "--oneline", "-i", "--grep=fast"],
           ["git", "grep", "-il", "is_nan"]],
    "D2": [["git", "log", "--all", "--oneline", "-i", "--grep=iteration"],
           ["git", "log", "--all", "--oneline", "-i", "--grep=failure"],
           ["git", "grep", "-il", "iteration"]],
    "D3": [["git", "log", "--oneline", "-n", "400", "-S", "#[default]"],
           ["git", "log", "--oneline", "-n", "400", "-S", "impl Default"],
           ["git", "log", "--oneline", "-n", "400", "-i", "--grep=derive"],
           ["git", "log", "--oneline", "-n", "400", "--", "src/options.rs"]],
    "D4": [["git", "log", "--oneline", "-n", "400", "-S", "sort_order"],
           ["git", "log", "--oneline", "-n", "400", "-S", "SortOrder"],
           ["git", "log", "--oneline", "-n", "400", "--", "src/export/mod.rs"]],
    "D5": [["git", "log", "--oneline", "-n", "400", "-S", "map(|r|"],
           ["git", "log", "--oneline", "-n", "400", "-S", "replacement"],
           ["git", "log", "--oneline", "-n", "400", "-i", "--grep=simplify"]],
    "D6": [["git", "grep", "-il", "Replacer"],
           ["git", "grep", "-il", "replacement"],
           ["git", "grep", "-il", "amortiz"]],
    "D7": [["git", "grep", "-il", "ReadByLine"],
           ["git", "grep", "-lw", "Core"],
           ["git", "grep", "-il", "bookkeeping"]],
    "D8": [["git", "grep", "-il", "Slab"],
           ["git", "grep", "-il", "preallocat"],
           ["git", "grep", "-il", "scratch"]],
    "D9": [["git", "log", "--oneline", "-n", "400", "-S", "math.MaxInt32"],
           ["git", "log", "--oneline", "-n", "400", "-S", "MaxInt32"],
           ["git", "log", "--oneline", "-n", "400", "-i", "--grep=overflow"]],
    "D10": [["git", "log", "--all", "--oneline", "-i", "--grep=radix"],
            ["git", "log", "--all", "--oneline", "-i", "--grep=sort"],
            ["git", "grep", "-il", "radix"]],
    "E1": [["git", "log", "--oneline", "-n", "400", "-S", "<'_>"],
           ["git", "log", "--oneline", "-n", "400", "-i", "--grep=lifetime"],
           ["git", "log", "--oneline", "-n", "400", "--", "src/benchmark/relative_speed.rs"]],
    "E2": [["git", "log", "--oneline", "-n", "400", "-S", "into_iter"],
           ["git", "log", "--oneline", "-n", "400", "-i", "--grep=clippy"],
           ["git", "log", "--oneline", "-n", "400", "-S", "generate_results"]],
    "E3": [["git", "log", "--all", "--oneline", "-i", "--grep=actions"],
           ["git", "log", "--all", "--oneline", "-i", "--grep=travis"],
           ["git", "branch", "-r", "--list", "*ci*"]],
    "E4": [["git", "grep", "-il", "serializ"],
           ["git", "grep", "-il", "deserializ"],
           ["git", "grep", "-il", "json"]],
}

MAX_IDENTIFYING = 3   # a probe that returns more candidates than this has not found the answer


def words(question):
    raw = re.findall(r"[A-Za-z][A-Za-z0-9_'-]*", question)
    seen, out = set(), []
    for w in raw:
        lw = w.lower()
        if len(lw) < 4 or lw in STOP or lw in seen:
            continue
        seen.add(lw)
        out.append(lw)
    return out


def run(clone, argv):
    p = subprocess.run(argv, cwd=str(clone), capture_output=True, text=True)
    return p.stdout


def branches_of(clone, shas):
    """Every remote branch containing any of these commits, the way an engineer
    would get from a commit a --grep found to the branch it belongs to."""
    out = set()
    for sha in shas:
        txt = run(clone, ["git", "branch", "-r", "--contains", sha])
        for line in txt.splitlines():
            name = line.strip().split(" ")[0]
            if not name or "->" in line:
                continue
            out.add(name[len("origin/"):] if name.startswith("origin/") else name)
    out.discard("master")
    out.discard("main")
    out.discard("HEAD")
    return out


def judge(task, clone, argv, out):
    """What this probe returned, and whether it is an answer."""
    gold = task["gold"]
    kind = task["kind"]
    is_log = argv[:2] == ["git", "log"]
    if kind == "commit" and is_log:
        shas = [l.split()[0] for l in out.splitlines() if l.split()]
        full = {run(clone, ["git", "rev-parse", s]).strip() for s in shas}
        want = run(clone, ["git", "rev-parse", gold]).strip()
        cands, present = len(full), want in full
    elif kind == "branch" and is_log:
        shas = [l.split()[0] for l in out.splitlines() if l.split()]
        bs = branches_of(clone, shas[:200])
        cands, present = len(bs), gold in bs
    elif kind == "branch":
        names = set()
        for line in out.splitlines():
            tok = line.strip().split()
            if tok:
                n = tok[0].strip("*").strip()
                names.add(n[len("origin/"):] if n.startswith("origin/") else n)
        names.discard("HEAD")
        cands, present = len(names), gold in names
    else:
        paths = {l.strip() for l in out.splitlines() if l.strip()}
        cands, present = len(paths), gold in paths
    if not present:
        v = "miss"
    elif cands <= MAX_IDENTIFYING:
        v = "HIT"
    else:
        v = "buried"
    return {"cmd": argv, "candidates": cands, "gold_present": present, "verdict": v}


def probes(task):
    kind = task["kind"]
    q = task["question"].replace("{repo}", "")
    out = []
    for w in words(q):
        if kind == "commit":
            out.append(["git", "log", "--oneline", "-n", "400", "-i", f"--grep={w}"])
            out.append(["git", "log", "--oneline", "-n", "400", "-i", "-S", w])
        elif kind == "branch":
            out.append(["git", "log", "--all", "--oneline", "-i", f"--grep={w}"])
            out.append(["git", "for-each-ref", "--format=%(refname:short) %(subject)", "refs/remotes"])
        out.append(["git", "grep", "-il", w])
        out.append(["git", "ls-files", f"*{w}*"])
    seen, uniq = set(), []
    for a in out:
        k = tuple(a)
        if k not in seen:
            seen.add(k)
            uniq.append(a)
    return uniq


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("tasks", nargs="*", help="task ids; default every D task")
    ap.add_argument("--json", help="write the full record here")
    ap.add_argument("--verbose", action="store_true", help="print every probe, not only the hits")
    a = ap.parse_args()
    ids = a.tasks or [t for t in TASKS if t.startswith("D")]

    record = {}
    bad = []
    for tid in ids:
        t = TASKS[tid]
        clone = STUDY / "corpus" / t["repo"]
        rows = []
        for argv in probes(t):
            rows.append({"kind": "derived", **judge(t, clone, argv, run(clone, argv))})
        for argv in GUESSED.get(tid, []):
            rows.append({"kind": "guessed", **judge(t, clone, argv, run(clone, argv))})
        hits = [r for r in rows if r["verdict"] == "HIT"]
        buried = [r for r in rows if r["verdict"] == "buried"]
        record[tid] = {"question": t["question"], "gold": t["gold"], "kind": t["kind"],
                       "repo": t["repo"], "probes": rows,
                       "n_probes": len(rows), "n_hit": len(hits), "n_buried": len(buried),
                       "lexically_reachable": bool(hits)}
        if hits:
            bad.append(tid)
        print(f"{tid:4s} {t['kind']:7s} {t['repo']:10s} probes {len(rows):3d}  "
              f"miss {len(rows) - len(hits) - len(buried):3d}  buried {len(buried):3d}  "
              f"HIT {len(hits):2d}   -> {'LEXICALLY REACHABLE, drop it' if hits else 'keeps'}")
        for r in (rows if a.verbose else hits):
            print(f"       {r['verdict']:7s} {r['candidates']:5d} cand  {' '.join(r['cmd'])}")

    print(f"\n{len(ids) - len(bad)} of {len(ids)} tasks survive; dropped: {bad or 'none'}")
    if a.json:
        pathlib.Path(a.json).write_text(json.dumps(record, indent=1))
        print(f"record -> {a.json}")


if __name__ == "__main__":
    main()
