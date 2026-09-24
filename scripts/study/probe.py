#!/usr/bin/env python3
"""Measure what one jevify call costs on each task, so the full study can be
priced against the keyless allowance even when the with arm does not call it.

    python3 scripts/study/probe.py

The validation block recorded zero jevify calls, so it says nothing about the
backend cost of the study. This runs, once per task, the call a with-arm agent
would plausibly make (`pick --from commit`, `pick --from file`, `pick --from
branch`, with the task's own description), reads `requests` and
`semantic_questions` out of the envelope, and prints the per-task ceiling: what
one adopted call costs. A run that calls jevify twice costs twice this; a run
that does not call it costs nothing.

It also scores the probe against the gold, which is a second, separate fact:
whether a single jevify call answers the task at all, measured outside any
agent. That is not the study's result and does not enter it.
"""

import json
import os
import pathlib
import subprocess

REPO = pathlib.Path(__file__).resolve().parents[2]
def _study_dir():
    """$JEVSTUDY decides which binary and which repositories this touches, so it is
    resolved and required to be an existing directory before anything uses it."""
    p = pathlib.Path(os.environ.get("JEVSTUDY") or (pathlib.Path.home() / "jevify-study")).resolve()
    if not p.is_dir():
        raise SystemExit(f"JEVSTUDY is not a directory: {p}")
    return p


STUDY = _study_dir()
TASKS = [json.loads(l) for l in (REPO / "scripts/study/tasks.jsonl").read_text().splitlines() if l.strip()]
KIND = {"commit": "commit", "path": "file", "branch": "branch"}


def main():
    env = dict(os.environ)
    env.pop("TYPESAFE_API_KEY_FILE", None)
    env.pop("TYPESAFE_API_KEY", None)
    env["JEVIFY_CACHE_DIR"] = str(STUDY / "probe-cache")
    tot_r = tot_q = 0
    print(f"{'task':5s} {'kind':7s} {'exit':>4s} {'req':>4s} {'quest':>6s} {'ms':>7s}  hit  answer")
    for t in TASKS:
        repo = STUDY / "corpus" / t["repo"]
        desc = t["question"].split(", find ", 1)[1].rstrip(".")
        p = subprocess.run([str(STUDY / "bin" / "jevify"), "pick", "--json",
                            "--from", KIND[t["kind"]], desc],
                           cwd=str(repo), capture_output=True, text=True, env=env)
        try:
            j = json.loads(p.stdout)
        except ValueError:
            print(f"{t['id']:5s} {KIND[t['kind']]:7s} {p.returncode:>4d}  no envelope: {p.stderr.strip()[:60]}")
            continue
        m = j.get("meta") or {}
        r = m.get("requests") or 0
        q = ((m.get("telemetry") or {}).get("semantic_questions")) or 0
        tot_r += r
        tot_q += q
        matches = (j.get("data") or {}).get("matches") or []
        got = matches[0].get("text", "") if matches else ""
        want = t["gold"]
        hit = "yes" if (got.startswith(want) or want.startswith(got) or got == want) and got else "no "
        print(f"{t['id']:5s} {KIND[t['kind']]:7s} {p.returncode:>4d} {r:>4d} {q:>6d} "
              f"{m.get('elapsed_ms') or 0:>7d}  {hit}  {got[:48]!r}")
    print(f"\ntotal over {len(TASKS)} tasks: {tot_r} requests, {tot_q} classifications "
          f"({tot_q / len(TASKS):.0f} per adopted call)")


if __name__ == "__main__":
    main()
