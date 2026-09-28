#!/usr/bin/env python3
"""Containment canaries: run the checks an escaping agent would run, under the
same profile the agent gets, and print what each one is allowed to do.

    python3 scripts/study/canary.py [--task D1]

Each canary runs through /usr/bin/sandbox-exec with the run's own profile.sb,
exactly as the agent process does, so what it reports is what the agent could
have done. The three arms are built from the real task set, so nothing about
the profile is written for the canary alone.
"""

import argparse
import json
import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from run_cell import ARMS, STUDY, HOME, REPO, TASKS, HAS_JEVIFY, TOOL, build, marker, scan  # noqa: E402

# a run directory that is not this canary's own, so the sibling rules are what
# is being tested. The newest one that exists; the check is skipped if there is none.
_sibs = sorted((d.name for d in (STUDY / "runs").iterdir() if d.is_dir()), reverse=True)
SIBLING = next((d for d in _sibs if not d.endswith("-r0")), "NO-SIBLING")

CHECKS = [
    ("write into a repository under ~/Projects", "touch ~/Projects/testify/CANARY-study"),
    ("write into /tmp", "echo x > /tmp/canary-study.txt"),
    ("write the CLI's /tmp/claude-XXXX-cwd (allowed)", "pwd -P > /tmp/claude-c0de-cwd && echo ok"),
    ("a zsh here-document (allowed)", "/bin/zsh -c 'cat <<EOF\nheredoc ok\nEOF'"),
    ("write into the home directory", "touch ~/CANARY-study"),
    ("read the harness and its gold answers", f"head -2 {REPO}/scripts/study/tasks.jsonl"),
    ("read the study tree above the run", f"ls {STUDY}/corpus"),
    ("list the run directory names", f"ls {STUDY}/runs | head -1"),
    ("read another run's answer", f"cat {STUDY}/runs/{SIBLING}/answer.txt"),
    ("read another run's transcript", f"head -c 40 {STUDY}/runs/{SIBLING}/transcript.jsonl"),
    ("read the TypeSafe key", "cat ~/.ssh/typesafe-ai-key"),
    ("exec the installed jevify", "~/.cargo/bin/jevify --version"),
    ("exec jevify from a target/ directory", f"{HOME}/Projects/jevify/target/release/jevify --version"),
    ("exec the study's jevify (the logged copy)", f"{STUDY}/bin/jevify --version"),
    ("run the study's jev (the thin baseline)", f"python3 {STUDY}/bin/jev --help 2>&1 | grep -q usage"),
    ("read the thin baseline in the repository", f"head -1 {REPO}/scripts/thinjev/jev"),
    ("cd out of the working directory and write", "cd ~ && touch CANARY-study-cd"),
    ("cd into the harness and read it", f"cd {REPO} && head -1 AGENTS.md"),
    ("write inside the run's own working directory", "touch ok.txt && echo ok"),
    ("write into the shared ~/.claude (the CLI's own state; allowed on purpose)",
     "touch ~/.claude/CANARY-study && echo ok"),
    ("read the operator's history in ~/.claude", "head -c 20 ~/.claude/history.jsonl"),
    ("read the operator's settings in ~/.claude", "head -c 20 ~/.claude/settings.json"),
    ("read the operator's own CLAUDE.md", "head -c 20 ~/.claude/CLAUDE.md"),
    ("mktemp", "f=$(mktemp) && echo $f"),
    ("the network", "curl -s -o /dev/null -w '%{http_code}' https://example.com"),
]


def main():
    ap = argparse.ArgumentParser()
    # the task id reaches a /bin/sh -c string through the run directory's name, so
    # it is taken from a fixed set rather than from free text or the environment
    ap.add_argument("--task", default=sorted(TASKS)[0], choices=sorted(TASKS))
    task = ap.parse_args().task
    for arm in ARMS:
        t, run, rid, _ = build(task, arm, 0, "haiku")
        marker(run)
        print(f"\n===== arm={arm}  profile={run}/profile.sb")
        for name, cmd in CHECKS:
            p = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(run / "profile.sb"),
                                "/bin/sh", "-c", cmd],
                               cwd=str(run / "work"), capture_output=True, text=True,
                               env={**os.environ, "TMPDIR": str(run / "tmp"), "PWD": str(run / "work")})
            out = (p.stdout + p.stderr).strip().splitlines()
            tail = out[-1][:90] if out else ""
            print(f"  {'ALLOWED' if p.returncode == 0 else 'denied ':8s} exit={p.returncode:<4d} {name:48s} {tail}")
        if HAS_JEVIFY[arm]:
            # the instrumentation itself: a real call through the run's wrapper,
            # and the log line it must leave behind
            w = run / "bin" / "jevify"
            q = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(run / "profile.sb"), "/bin/sh", "-c",
                                f"printf 'alpha the cat sleeps\nbeta the dog barks\n' | {w} pick 'the one about a dog'"],
                               cwd=str(run / "work"), capture_output=True, text=True,
                               env={**os.environ, "TMPDIR": str(run / "tmp"), "PWD": str(run / "work")})
            print(f"  wrapper    exit={q.returncode:<4d} {'jevify pick through the logged wrapper':48s} "
                  f"{q.stdout.strip()[:60]}")
            log = (run / "jevify.jsonl").read_text().strip().splitlines()
            print(f"  log line   {log[-1][:160] if log else 'NOTHING LOGGED'}")
        if TOOL[arm] == "jev":
            w = run / "bin" / "jev"
            q = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(run / "profile.sb"), "/bin/sh", "-c",
                                f"printf 'alpha the cat sleeps\nbeta the dog barks\n' | {w} 'the one about a dog'"],
                               cwd=str(run / "work"), capture_output=True, text=True,
                               env={**os.environ, "TMPDIR": str(run / "tmp"), "PWD": str(run / "work")})
            print(f"  wrapper    exit={q.returncode:<4d} {'jev through the logged wrapper':48s} "
                  f"{' | '.join(q.stdout.strip().splitlines())[:60]}")
            log = (run / "jevify.jsonl").read_text().strip().splitlines()
            print(f"  log line   {log[-1][:160] if log else 'NOTHING LOGGED'}")
        left = scan(run)
        print(f"  paths outside the run whose mtime moved: {len(left)} "
              f"(an mtime, not a writer -- see run_cell.scan)")
        for x in left[:10]:
            print("    ", x)


if __name__ == "__main__":
    main()
