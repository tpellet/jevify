#!/usr/bin/env python3
"""Containment canaries: run the checks an escaping agent would run, under the
same profile the agent gets, and print what each one is allowed to do.

    python3 scripts/study/canary.py

Each canary runs through /usr/bin/sandbox-exec with the run's own profile.sb,
exactly as the agent process does, so what it reports is what the agent could
have done. The two arms are built from the real task set, so nothing about the
profile is written for the canary alone.
"""

import json
import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from run_cell import STUDY, HOME, REPO, build, marker, scan  # noqa: E402

CHECKS = [
    ("write into a repository under ~/Projects", "touch ~/Projects/testify/CANARY-study"),
    ("write into /tmp", "echo x > /tmp/canary-study.txt"),
    ("write into the home directory", "touch ~/CANARY-study"),
    ("read the harness and its gold answers", f"head -2 {REPO}/scripts/study/tasks.jsonl"),
    ("read the study tree above the run", f"ls {STUDY}/corpus"),
    ("list the run directory names", f"ls {STUDY}/runs | head -1"),
    ("read another run's answer", f"cat {STUDY}/runs/S8-with-r1/answer.txt"),
    ("read another run's transcript", f"head -c 40 {STUDY}/runs/S8-with-r1/transcript.jsonl"),
    ("read the TypeSafe key", "cat ~/.ssh/typesafe-ai-key"),
    ("exec the installed jevify", "~/.cargo/bin/jevify --version"),
    ("exec jevify from a target/ directory", f"{HOME}/Projects/jevify/target/release/jevify --version"),
    ("exec the study's jevify (the logged copy)", f"{STUDY}/bin/jevify --version"),
    ("cd out of the working directory and write", "cd ~ && touch CANARY-study-cd"),
    ("cd into the harness and read it", f"cd {REPO} && head -1 AGENTS.md"),
    ("write inside the run's own working directory", "touch ok.txt && echo ok"),
    ("mktemp", "f=$(mktemp) && echo $f"),
    ("the network", "curl -s -o /dev/null -w '%{http_code}' https://example.com"),
]


def main():
    for arm in ("with", "without"):
        t, run, rid = build("S1", arm, 0, "haiku")
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
        if arm == "with":
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
        left = scan(run)
        print(f"  files written outside the run: {len(left)}")
        for x in left[:10]:
            print("    ", x)


if __name__ == "__main__":
    main()
