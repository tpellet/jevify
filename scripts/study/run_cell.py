#!/usr/bin/env python3
"""Run one cell of the paired study: one task, one arm, one repetition.

    python3 scripts/study/run_cell.py --task D1 --arm available --rep 1

Four arms, never averaged together:

    control    jevify is not installed and the prompt says so
    thin       jevify is not installed; the thin baseline scripts/thinjev/jev (one
               keyless Jev call ranking options, nothing else) is, mentioned once
               the way `available` mentions jevify
    available  jevify is installed, the prompt mentions it once and hands over its
               own documentation; whether the agent reaches for it is the
               measurement
    required   the same, plus one sentence telling the agent to use it for the
               step that chooses among candidates

A cell is a headless Claude Code agent answering one task inside a Seatbelt
sandbox. The sandbox wraps the agent process itself, so every child process and
every in-process file read of the built-in tools inherits it; there is no
wrapper for the agent to step around and `cd` changes nothing about what it may
touch.

Layout, all outside the repository, under $JEVSTUDY (default ~/jevify-study):

    corpus/<repo>/          the pinned clone, never touched by a run
    bin/jevify              the binary under study, the only copy a run can exec
    bin/jev                 the thin baseline, the only copy the thin arm can run
    runs/<task>-<arm>-<model>-r<n>/
        work/<repo>/        an APFS clone of the pinned repo, the run's own
        bin/jevify, bin/jev the logging wrapper (jevify arms, thin arm)
        cache/ tmp/         the run's own answer cache and temp dir
        profile.sb          the Seatbelt profile
        prompt.txt          the exact prompt, byte for byte
        transcript.jsonl    the agent's stream-json transcript
        jevify.jsonl        one line per jevify call: argv, exit, envelope meta
        meta.json           cost and adoption figures for this cell
        answer.txt          the ANSWER: line the agent printed
        claude-config/      this cell's own CLAUDE_CONFIG_DIR, with --per-run-config
        mtime_moved.txt     paths outside the run whose mtime moved (a smoke test,
                            not write attribution: see scan())

The gold answers live in the repository, which the profile denies reading, so a
run cannot read the answer it is being scored against.
"""

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

REPO = pathlib.Path(__file__).resolve().parents[2]
def _study_dir():
    """$JEVSTUDY decides which binary and which repositories this touches, so it is
    resolved and required to be an existing directory before anything uses it."""
    p = pathlib.Path(os.environ.get("JEVSTUDY") or (pathlib.Path.home() / "jevify-study")).resolve()
    if not p.is_dir():
        raise SystemExit(f"JEVSTUDY is not a directory: {p}")
    return p


STUDY = _study_dir()
HOME = pathlib.Path.home()
TASKS = {t["id"]: t for t in (json.loads(l) for l in (REPO / "scripts/study/tasks.jsonl").read_text().splitlines() if l.strip())}

PROFILE = """(version 1)
(allow default)

; ---- writes: nothing outside this run, the agent's own config and its scratch
(deny file-write*)
(allow file-write* (subpath "{run}"))
(allow file-write* (subpath "{claude_home}"))
{claude_read_denies}
(allow file-write* (subpath "{scratch}"))
(allow file-write* (subpath "{tmpdir}"))
(allow file-write* (subpath "/dev"))

; ---- reads: not the harness, not the gold, not the study tree, not the keys.
; file-read-data, not file-read*: stat stays allowed, so git and jevify can walk
; the parent chain of the working directory. Denying metadata too makes every
; tool fail on the way up and silently changes what the task is.
(deny file-read-data (subpath "{repo}"))
(deny file-read-data (subpath "{study}/blind"))
(deny file-read-data (literal "{study}/unblind_map.json"))
(deny file-read-data (literal "{study}/scored.jsonl"))
(deny file-read-data (subpath "{home}/.ssh"))
; every other run, one rule each. A single deny on the runs/ subtree would cover
; this run's own ancestor and the agent process then fails to start, whatever is
; re-allowed underneath, so the siblings are named instead.
{siblings}
(allow file-read* (subpath "{run}"))
(allow file-read* (subpath "{study}/bin"))

; ---- jevify: the only executable copy is the one this harness logs
(deny file-read* (literal "{home}/.cargo/bin/jevify"))
(deny process-exec* (literal "{home}/.cargo/bin/jevify"))
(deny file-read* (regex #"/target/[^/]+/jevify$"))
(deny process-exec* (regex #"/target/[^/]+/jevify$"))
{arm}
"""

# The arms differ in the profile by exactly these lines, and in the prompt by
# exactly one paragraph. `available` and `required` get the same profile; what
# separates them is what the prompt says, which is the whole point of having
# both. `thin` swaps jevify for the thin baseline scripts/thinjev/jev: the same
# model behind one keyless call, without listers, evidence, rounds or threshold.
ALLOW = """(allow file-read* (literal "{study}/bin/{tool}"))
(allow process-exec* (literal "{study}/bin/{tool}"))
"""
DENY = """(deny file-read* (literal "{study}/bin/{tool}"))
(deny process-exec* (literal "{study}/bin/{tool}"))
"""
ARMS = ("control", "thin", "available", "required")
HAS_JEVIFY = {"control": False, "thin": False, "available": True, "required": True}
TOOL = {"control": None, "thin": "jev", "available": "jevify", "required": "jevify"}


def arm_rules(arm, study):
    return "".join((ALLOW if TOOL[arm] == tool else DENY).format(study=study, tool=tool)
                   for tool in ("jevify", "jev"))

HEAD = """You are answering one question about a local git repository. Work read-only.

Answer format: the last line of your final message must be exactly

    ANSWER: <value>

and <value> must be {answer_format} and nothing else. No backticks, no prose on
that line. If you cannot find it, write ANSWER: unknown.
"""

# `available` is the paragraph a real integrator writes: the tool exists, here is
# where it is, here is its own documentation. Nothing in it argues for the tool
# or hints that this task is one it suits. Whether the agent reaches for it is
# the measurement, so the paragraph must not do the reaching.
AVAILABLE_DOC = """
The command-line tool jevify is installed at {jevify}. Call it by that full path.
Its documentation for agents follows.

{block}
"""

# `required` is the same paragraph plus one instruction. It answers a different
# question -- does the tool help when it is used -- and its number must never be
# read as an adoption figure.
REQUIRED_DOC = AVAILABLE_DOC + """
For this task you must use jevify for the step that chooses among candidates.
Run it at least once and let what it returns decide your answer.
"""

CONTROL_DOC = """
The tool jevify is not installed on this machine.
"""

# `thin` is `available` with the thin baseline in place of jevify: the tool, its
# path and its own usage, told once and not required. The usage is the whole of
# what `jev` can do, the way `jevify init agents` is the whole of jevify.
THIN_DOC = """
The command-line tool jev is installed at {jev}. Call it by that full path.
Its documentation for agents follows.

# jev
jev ranks a list of options against a question about meaning, using the Jev
model through one call.
  jev "question" option1 option2 ...
  some-command | jev "question"        # one option per stdin line
It prints every option as probability<TAB>option, best first. 2 to 100 options,
each at most 200 characters. Exit codes: 0 ok, 2 bad input, 4 service unavailable.
"""


def sh(*a, **kw):
    return subprocess.run(a, capture_output=True, text=True, **kw)


def build(task_id, arm, rep, model, per_run_config=False):
    t = TASKS[task_id]
    # the model is part of the cell: a haiku run and a sonnet run of the same task,
    # arm and repetition are two cells, not one cell run twice
    rid = f"{task_id}-{arm}-{model}-r{rep}"
    run = STUDY / "runs" / rid
    for d in ("work", "cache", "tmp", "bin"):
        (run / d).mkdir(parents=True, exist_ok=True)

    src = STUDY / "corpus" / t["repo"]
    # -c asks APFS for a copy-on-write clone: instant, and no run shares bytes with
    # another run or with the pinned corpus. A rerun of the same cell never reuses a
    # working copy an earlier attempt could have touched: it takes the next free name.
    k = 0
    while True:
        dst = run / "work" / (t["repo"] if k == 0 else f"{t['repo']}.{k}")
        if not dst.exists():
            subprocess.run(["cp", "-Rc", str(src), str(dst)], check=True)
            break
        clean = sh("git", "-C", str(dst), "status", "--porcelain").stdout.strip() == ""
        if clean and sh("git", "-C", str(dst), "rev-parse", "HEAD").stdout.strip() == t["pin"]:
            break
        k += 1
    head = sh("git", "-C", str(dst), "rev-parse", "HEAD").stdout.strip()
    assert head == t["pin"], f"{dst} is at {head}, not the pin {t['pin']}"

    # a rerun of the same cell starts its call log empty, so the log is this
    # attempt's and not two attempts' added together
    (run / "jevify.jsonl").write_text("")
    scratch = pathlib.Path("/private/tmp/claude-501") / ("-" + str(run / "work").strip("/").replace("/", "-"))
    scratch.mkdir(parents=True, exist_ok=True)
    rules = arm_rules(arm, STUDY)
    if per_run_config:
        claude_home = run / "claude-config"
        claude_home.mkdir(parents=True, exist_ok=True)
        seed_claude_config(claude_home)
        claude_read_denies = ""
    else:
        claude_home = HOME / ".claude"
        claude_read_denies = "\n".join(
            f'(deny file-read* (subpath "{claude_home / n}"))' if (claude_home / n).is_dir()
            else f'(deny file-read* (literal "{claude_home / n}"))'
            for n in SHARED_CLAUDE_DENY)
    siblings = "\n".join(f'(deny file-read-data (subpath "{d}"))'
                         for d in sorted((STUDY / "runs").iterdir()) if d != run and d.is_dir())
    (run / "profile.sb").write_text(PROFILE.format(
        run=run, claude_home=claude_home, claude_read_denies=claude_read_denies,
        scratch=scratch, tmpdir=run / "tmp",
        repo=REPO, study=STUDY, home=HOME, arm=rules, siblings=siblings))

    prompt = HEAD.format(answer_format=t["answer_format"])
    if HAS_JEVIFY[arm]:
        wrapper = run / "bin" / "jevify"
        wrapper.write_text(
            "#!/bin/sh\n"
            f'JEVSTUDY_LOG="{run}/jevify.jsonl" '
            f'JEVIFY_CACHE_DIR="{run}/cache" exec python3 "{STUDY}/bin/jevify_log.py" "$@"\n')
        wrapper.chmod(0o755)
        block = (STUDY / "bin" / "init-agents.txt").read_text().strip()
        prompt += (REQUIRED_DOC if arm == "required" else AVAILABLE_DOC).format(
            jevify=wrapper, block=block)
    elif arm == "thin":
        # logged exactly like jevify: every call, its argv, exit, stdout and stderr
        wrapper = run / "bin" / "jev"
        wrapper.write_text(
            "#!/bin/sh\n"
            f'JEVSTUDY_LOG="{run}/jevify.jsonl" exec python3 "{STUDY}/bin/jevify_log.py" --thinjev "$@"\n')
        wrapper.chmod(0o755)
        prompt += THIN_DOC.format(jev=wrapper)
    else:
        prompt += CONTROL_DOC
    prompt += "\nTask: " + t["question"].format(repo=dst) + "\n"
    (run / "prompt.txt").write_text(prompt)
    return t, run, rid, claude_home


# The four keys the CLI needs in its configuration directory to consider itself
# set up and signed in. The OAuth token itself lives in the Keychain, not here.
# Only these four are copied: the operator's own ~/.claude.json holds project
# paths, MCP server definitions and history, and a run can read its own
# configuration directory, so copying the whole file would hand all of that to
# the agent under test.
CONFIG_SEED_KEYS = ("oauthAccount", "userID", "hasCompletedOnboarding", "lastOnboardingVersion")
KEYCHAIN_SERVICE = "Claude Code-credentials"

# Everything under the shared configuration directory that is the operator's and
# not the run's. The directory itself has to stay writable -- the CLI keeps its
# state there and, on this machine, authenticates only from that one path -- but
# nothing in this list is anything a run has business reading.
SHARED_CLAUDE_DENY = ("history.jsonl", "projects", "todos", "file-history", "downloads",
                      "shell-snapshots", "debug", "logs", "statsig", "commands",
                      "skills", "plugins", "agents", "hooks", "CLAUDE.md", "AGENTS.md",
                      "settings.json", "settings.local.json", "backups", "daemon", "jobs")


def seed_claude_config(claude_home):
    """Give this cell its own CLAUDE_CONFIG_DIR.

    Per-run means nothing a cell writes to its configuration directory can reach
    another cell. It costs what the harness's fault list said it would cost: the
    CLI consults the login Keychain only for the default directory, so a per-run
    directory has to carry the OAuth token as a file, and the agent under test
    runs as the process that reads that file. That is a worse hole than the one
    it closes, which is why --per-run-config is off by default and why the study
    was not run with it.
    """
    f = claude_home / ".claude.json"
    if not f.exists():
        src = HOME / ".claude.json"
        seed = {}
        if src.exists():
            whole = json.loads(src.read_text())
            seed = {k: whole[k] for k in CONFIG_SEED_KEYS if k in whole}
        seed["hasCompletedOnboarding"] = True
        f.write_text(json.dumps(seed))
        f.chmod(0o600)
    c = claude_home / ".credentials.json"
    if not c.exists():
        p = sh("security", "find-generic-password", "-s", KEYCHAIN_SERVICE, "-w")
        if p.returncode != 0 or not p.stdout.strip():
            raise SystemExit("--per-run-config needs the login Keychain entry "
                             f"{KEYCHAIN_SERVICE!r}, which is not readable here")
        c.write_text(p.stdout.strip())
        c.chmod(0o600)


def marker(run):
    m = run / ".marker"
    m.write_text(str(time.time()))
    return m


def scan(run):
    """Every path outside this run whose mtime moved while the cell ran.

    This is NOT write attribution and it does not show that the run wrote
    anything: `find -newer` reports an mtime, not a writer, and on a machine
    doing anything else at all it reports the operator's own files. It is a
    smoke test -- an empty list is weak reassurance, a non-empty list is a
    prompt to go and look. The evidence that the sandbox holds is canary.py,
    which tries each forbidden write through the same profile and reports what
    the kernel did.
    """
    roots = [str(HOME / "Projects"), str(HOME / ".config"), str(HOME / ".cache"),
             str(HOME / ".cargo"), str(HOME / ".local"), str(HOME / ".ssh"),
             "/private/tmp", str(STUDY)]
    # the build and package caches are pruned: they hold hundreds of thousands of
    # files, they are what this machine's own tooling writes while a block runs,
    # and the profile denies writing them anyway. .git is not pruned.
    out = sh("find", *[r for r in roots if os.path.exists(r)], "-xdev",
             "(", "-name", "target", "-o", "-name", "node_modules", "-o", "-name", "registry",
             "-o", "-name", ".cache", "-o", "-name", ".ruff_cache", ")", "-prune", "-o",
             "-newer", str(run / ".marker"),
             "(", "-type", "f", "-o", "-type", "d", ")",
             "-not", "-path", str(run), "-not", "-path", str(run) + "/*",
             "-not", "-path", str(STUDY / "runs") + "/*",
             "-not", "-path", "/private/tmp/claude-501/*",
             # the agent-mail server and the agent's own config are the host's, not
             # the run's: the canary shows both move while no run is going
             "-not", "-path", str(HOME / ".local/state/mcp_agent_mail") + "*",
             "-not", "-path", str(HOME / ".claude") + "/*", "-print")
    # a root directory whose own mtime moved is not a file the run wrote
    lines = sorted(x for x in out.stdout.splitlines() if x.strip() and x not in roots)
    (run / "mtime_moved.txt").write_text("\n".join(lines) + "\n")
    return lines


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--task", required=True)
    ap.add_argument("--arm", required=True, choices=list(ARMS))
    ap.add_argument("--rep", type=int, default=1)
    ap.add_argument("--model", default="haiku")
    ap.add_argument("--budget", type=float, default=1.0, help="max USD of agent inference per cell")
    ap.add_argument("--timeout", type=float, default=900, help="seconds before a cell is abandoned")
    ap.add_argument("--per-run-config", action="store_true",
                    help="give the cell its own CLAUDE_CONFIG_DIR. Closes the shared-directory "
                         "hole and opens a worse one: see seed_claude_config")
    a = ap.parse_args()

    t, run, rid, claude_home = build(a.task, a.arm, a.rep, a.model, a.per_run_config)
    env = dict(os.environ)
    env["TMPDIR"] = str(run / "tmp")
    # a stale PWD outside the sandbox makes every /bin/sh print a getcwd warning
    # into the agent's context, which costs tokens and is not the task
    env["PWD"] = str(run / "work")
    env["JEVIFY_CACHE_DIR"] = str(run / "cache")
    env["PATH"] = f"{run / 'bin'}:{env['PATH']}" if TOOL[a.arm] else env["PATH"]
    # per-run, so no two cells share a configuration directory and nothing a cell
    # writes there can reach another. The OAuth credentials this machine
    # authenticates with live in the Keychain, not in the configuration
    # directory, so moving it costs nothing.
    if a.per_run_config:
        env["CLAUDE_CONFIG_DIR"] = str(claude_home)
    for k in ("TYPESAFE_API_KEY", "TYPESAFE_API_KEY_FILE"):
        env.pop(k, None)  # the study runs jevify keyless, so no arm can read a key

    cmd = ["/usr/bin/sandbox-exec", "-f", str(run / "profile.sb"),
           "claude", "-p", "--safe-mode", "--no-session-persistence",
           "--permission-mode", "bypassPermissions",
           "--model", a.model, "--output-format", "stream-json", "--verbose",
           "--max-budget-usd", str(a.budget),
           "--tools", "Bash", "Read", "Grep", "Glob"]
    marker(run)
    t0 = time.time()
    timed_out = False
    try:
        p = subprocess.run(cmd, cwd=str(run / "work"), env=env,
                           input=(run / "prompt.txt").read_text(), capture_output=True,
                           text=True, timeout=a.timeout)
    except subprocess.TimeoutExpired as e:
        # a cell that never returns would stall the whole block, so it is abandoned
        # and recorded as abandoned rather than left to run
        timed_out = True
        p = subprocess.CompletedProcess(cmd, -1, e.stdout or "", e.stderr or "")
    wall = time.time() - t0
    (run / "transcript.jsonl").write_text(p.stdout)
    (run / "stderr.txt").write_text(p.stderr)
    mtime_moved = scan(run)

    msgs = []
    for line in p.stdout.splitlines():
        try:
            msgs.append(json.loads(line))
        except ValueError:
            pass
    res = next((m for m in reversed(msgs) if m.get("type") == "result"), {})
    text = res.get("result", "") or ""
    ans = ""
    for line in reversed(text.splitlines()):
        if line.strip().startswith("ANSWER:"):
            ans = line.split("ANSWER:", 1)[1].strip()
            break
    (run / "answer.txt").write_text(ans + "\n")

    calls = []
    jl = run / "jevify.jsonl"
    if jl.exists():
        calls = [json.loads(l) for l in jl.read_text().splitlines() if l.strip()]
    u = res.get("usage") or {}
    tools = sum(1 for m in msgs if m.get("type") == "assistant"
                for c in (m.get("message") or {}).get("content") or [] if c.get("type") == "tool_use")
    meta = {
        "rid": rid, "task": a.task, "arm": a.arm, "rep": a.rep, "model": a.model,
        "tool": TOOL[a.arm], "budget_usd": a.budget,
        "binary_sha256": (STUDY / "bin" / "jevify.sha256").read_text().strip()
                         if (STUDY / "bin" / "jevify.sha256").exists() else None,
        "repo": t["repo"], "pin": t["pin"],
        "turns": res.get("num_turns"), "tool_uses": tools,
        "input_tokens": (u.get("input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0)
                        + (u.get("cache_creation_input_tokens") or 0),
        "output_tokens": u.get("output_tokens"),
        "agent_cost_usd": res.get("total_cost_usd"), "wall_s": round(wall, 1),
        "api_duration_ms": res.get("duration_api_ms"), "is_error": res.get("is_error"),
        "stop": "timeout" if timed_out else (res.get("terminal_reason") or res.get("subtype")),
        "jevify_calls": len(calls),
        "jevify_exits": [c.get("exit") for c in calls],
        "jevify_argv": [c.get("argv") for c in calls],
        "jevify_requests": sum((c.get("meta") or {}).get("requests") or 0 for c in calls),
        "jevify_questions": sum((c.get("meta") or {}).get("questions") or 0 for c in calls),
        "jevify_cache_hits": sum((c.get("meta") or {}).get("cache_hits") or 0 for c in calls),
        "mtime_moved_outside_run": mtime_moved,
        "cli_exit": p.returncode,
    }
    (run / "meta.json").write_text(json.dumps(meta, indent=1))
    print(json.dumps({k: v for k, v in meta.items() if k not in ("jevify_argv",)}))


if __name__ == "__main__":
    main()
