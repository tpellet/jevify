#!/usr/bin/env python3
"""Turn finished runs into blinded records the scorer can read, and write the
unblinding map somewhere the scorer does not look.

    python3 scripts/study/blind.py

For every run directory it writes

    $JEVSTUDY/blind/<bid>.json   {"bid", "task", "answer", "leaked", "leak"}

and nothing else. The record names the task and the answer text. It does not
name the arm, the repetition, the model, the run directory or the wall time,
and `bid` is sha256(rid) truncated, which carries no arm: the two arms of a
pair hash to unrelated strings and the directory listing is in hash order.

    $JEVSTUDY/unblind_map.json   bid -> {rid, task, arm, rep, model}

is written outside blind/ so that score.py, which opens only blind/*.json and
the task file, cannot reach it.

The leak check runs here rather than in the scorer because it needs the
transcript, and a transcript names the arm. Its verdict is a single boolean,
which is the same kind of fact for either arm. A run is leaked when:

  1. the gold answer appears in the prompt the agent was given;
  2. the sentinel string that lives only in the gold file appears anywhere in
     the transcript, which means the agent read the gold file;
  3. any tool result in the transcript names the harness directory or the task
     file, which means the agent reached the harness.
"""

import hashlib
import json
import os
import pathlib
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
HARNESS = str(REPO / "scripts/study")


def leak_check(run, task):
    reasons = []
    gold = task["gold"].lower()
    prompt = (run / "prompt.txt").read_text()
    if gold in prompt.lower():
        reasons.append("gold in prompt")
    tr = (run / "transcript.jsonl")
    text = tr.read_text() if tr.exists() else ""
    if task["sentinel"] in text:
        reasons.append("gold-file sentinel in transcript")
    for line in text.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("type") != "user":
            continue
        for c in (m.get("message") or {}).get("content") or []:
            if c.get("type") != "tool_result":
                continue
            body = c.get("content")
            if isinstance(body, list):
                body = " ".join(x.get("text", "") for x in body if isinstance(x, dict))
            body = str(body)
            if HARNESS in body or "tasks.jsonl" in body:
                reasons.append("harness reached from a tool result")
                return reasons
    return reasons


def main():
    blind = STUDY / "blind"
    blind.mkdir(parents=True, exist_ok=True)
    mapping = {}
    n = 0
    for run in sorted((STUDY / "runs").iterdir()):
        meta_f = run / "meta.json"
        if not meta_f.exists():
            continue
        meta = json.loads(meta_f.read_text())
        task = TASKS[meta["task"]]
        bid = hashlib.sha256(meta["rid"].encode()).hexdigest()[:12]
        reasons = leak_check(run, task)
        rec = {"bid": bid, "task": meta["task"],
               "answer": (run / "answer.txt").read_text().strip(),
               "leaked": bool(reasons), "leak": reasons}
        (blind / f"{bid}.json").write_text(json.dumps(rec, indent=1))
        mapping[bid] = {"rid": meta["rid"], "task": meta["task"], "arm": meta["arm"], "rep": meta["rep"],
                        "model": meta.get("model", "haiku")}
        n += 1
    (STUDY / "unblind_map.json").write_text(json.dumps(mapping, indent=1))
    print(f"blinded {n} runs into {blind}; map in {STUDY / 'unblind_map.json'}")


if __name__ == "__main__":
    main()
