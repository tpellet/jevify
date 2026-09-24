#!/usr/bin/env python3
"""Score the blinded records. This file never learns which arm produced a run.

    python3 scripts/study/score.py

It opens exactly three kinds of path: $JEVSTUDY/blind/*.json, the task file,
and the pinned corpus repositories (to resolve an abbreviated commit hash to
its full one). It does not open $JEVSTUDY/unblind_map.json, any run directory,
any meta.json or any transcript. The word "arm" does not appear below except in
this sentence and in the check at the bottom, which fails the run if a blinded
record ever carries one.

Normalisation, so that a third party scoring by hand reaches the same verdict:

  commit  both sides are resolved with `git rev-parse` in the pinned clone and
          the full 40-character hashes are compared, so 7, 8 or 40 characters
          all score the same and a hash that does not exist scores wrong.
  path    leading "./", a leading repository name and surrounding quotes are
          dropped; the comparison is then exact and case-sensitive.
  branch  a leading "origin/" or "refs/remotes/origin/" is dropped.

A record whose leak check fired is refused, not scored.
"""

import json
import os
import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
def _study_dir():
    """$JEVSTUDY decides which binary and which repositories this touches, so it is
    resolved and required to be an existing directory before anything uses it."""
    p = pathlib.Path(os.environ.get("JEVSTUDY") or (pathlib.Path.home() / "jevify-study")).resolve()
    if not p.is_dir():
        raise SystemExit(f"JEVSTUDY is not a directory: {p}")
    return p


STUDY = _study_dir()
TASKS = {t["id"]: t for t in (json.loads(l) for l in (REPO / "scripts/study/tasks.jsonl").read_text().splitlines() if l.strip())}


def full_sha(repo, ref):
    if not ref or not all(c in "0123456789abcdefABCDEF" for c in ref) or len(ref) < 4:
        return None
    p = subprocess.run(["git", "-C", str(STUDY / "corpus" / repo), "rev-parse", "--verify", f"{ref}^{{commit}}"],
                       capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else None


def norm(kind, repo, value):
    v = value.strip().strip("`'\"").strip()
    if kind == "commit":
        return full_sha(repo, v.split()[0] if v else "")
    if kind == "path":
        v = v.split()[0] if v else ""
        v = v.lstrip("./")
        if v.startswith(repo + "/"):
            v = v[len(repo) + 1:]
        return v or None
    if kind == "branch":
        for p in ("refs/remotes/origin/", "remotes/origin/", "origin/"):
            if v.startswith(p):
                v = v[len(p):]
        return v or None
    raise ValueError(kind)


def main():
    rows = []
    for f in sorted((STUDY / "blind").glob("*.json")):
        rec = json.loads(f.read_text())
        assert "arm" not in rec and "rid" not in rec, f"{f} is not blind"
        t = TASKS[rec["task"]]
        if rec["leaked"]:
            rows.append({"bid": rec["bid"], "task": rec["task"], "status": "refused_leak",
                         "correct": None, "why": "; ".join(rec["leak"])})
            continue
        if not rec["answer"] or rec["answer"].lower() == "unknown":
            rows.append({"bid": rec["bid"], "task": rec["task"], "status": "no_answer", "correct": 0})
            continue
        got = norm(t["kind"], t["repo"], rec["answer"])
        want = norm(t["kind"], t["repo"], t["gold"])
        rows.append({"bid": rec["bid"], "task": rec["task"],
                     "status": "scored", "correct": int(got is not None and got == want),
                     "normalised": got})
    out = STUDY / "scored.jsonl"
    out.write_text("".join(json.dumps(r) + "\n" for r in rows))
    n = len(rows)
    print(f"scored {n} records -> {out}")
    print(f"  correct {sum(1 for r in rows if r['correct'] == 1)}"
          f" · wrong {sum(1 for r in rows if r['correct'] == 0)}"
          f" · refused {sum(1 for r in rows if r['status'] == 'refused_leak')}")


if __name__ == "__main__":
    main()
