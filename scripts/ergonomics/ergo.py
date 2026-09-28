#!/usr/bin/env python3
"""Call-ergonomics harness: many short headless agent runs, every tool call logged.

    python3 scripts/ergonomics/ergo.py stage --label v0.13.0 --kind jevify --src ~/.cargo/bin/jevify
    python3 scripts/ergonomics/ergo.py stage --label thin --kind thin --src scripts/thinjev/jev
    python3 scripts/ergonomics/ergo.py canary --stage v0.13.0
    python3 scripts/ergonomics/ergo.py run --phase base --stage v0.13.0 --models haiku:3,sonnet:1
    python3 scripts/ergonomics/report.py --phase base

A run is one task, one model, one repetition: `claude -p` inside a Seatbelt
profile that wraps the agent process itself, so every child and every built-in
file read inherits it. The agent sees what an agent normally has: the tool's own
agent instructions (`jevify init agents`, or the thin wrapper's README), `--help`
on demand, a pinned clone of a small public repository, the task's data files,
and one task. The tool on its PATH is a logging shim (calllog.py); the real
binary sits in the stage directory, readable only by that stage's runs.

Everything a run touches lives under ~/jevify-ergo, outside
the repository. The repository, which holds tasks.jsonl and its gold answers, is
denied for reading, as are ~/.ssh, the paired-study tree, other stages and every
other run.
"""

import argparse
import concurrent.futures
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import threading
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import fixtures  # noqa: E402

REPO = HERE.parents[1]
HOME = pathlib.Path.home()
# fixed, not taken from the environment: every path a profile names and every
# command the harness runs derives from it
ROOT = (HOME / "jevify-ergo").resolve()
CORPUS_URL = "https://github.com/sharkdp/hyperfine"
CORPUS_PIN = "f12f3d9f86f3643b3b7deace5e160b1f0f44d2b7"
CORPUS = ROOT / "corpus" / "hyperfine"
TASKS = [json.loads(l) for l in (HERE / "tasks.jsonl").read_text().splitlines() if l.strip()]
TASK = {t["id"]: t for t in TASKS}

# Everything in the shared CLI configuration directory that belongs to the operator
# rather than to the run (the same list as scripts/study/run_cell.py).
SHARED_CLAUDE_DENY = ("history.jsonl", "projects", "todos", "file-history", "downloads",
                      "shell-snapshots", "debug", "logs", "statsig", "commands",
                      "skills", "plugins", "agents", "hooks", "CLAUDE.md", "AGENTS.md",
                      "settings.json", "settings.local.json", "backups", "daemon", "jobs")

PROFILE = """(version 1)
(allow default)

; ---- writes: this run, the CLI's own state, its scratch directory
(deny file-write*)
(allow file-write* (subpath "{run}"))
(allow file-write* (subpath "{claude_home}"))
{claude_denies}
(allow file-write* (subpath "{scratch}"))
(allow file-write* (subpath "/dev"))
; the CLI's Bash tool records its working directory in /tmp/claude-XXXX-cwd after
; every command, whatever TMPDIR says; denied, every command reports exit 1
(allow file-write* (regex #"^/private/tmp/claude-[0-9a-f]+-cwd$"))
; zsh writes every here-document to $TMPPREFIX, /tmp/zsh by default; denied,
; `cat <<'EOF' | tool` pipes nothing and the tool is blamed for empty input
(allow file-write* (regex #"^/private/tmp/zsh[A-Za-z0-9]*$"))

; ---- reads: not the harness and its gold, not the keys, not other runs or stages.
; file-read-data, not file-read*: stat stays allowed so git can walk the parents.
(deny file-read-data (subpath "{repo}"))
(deny file-read-data (subpath "{home}/.ssh"))
(deny file-read-data (subpath "{home}/jevify-study"))
(deny file-read-data (subpath "{root}/corpus"))
{others}
(deny file-read* (subpath "{root}/bin"))
(allow file-read* (literal "{root}/bin/calllog.py"))
(allow file-read* (subpath "{stage}"))
(allow file-read* (subpath "{run}"))

; ---- the tool: the stage's copy is the only one, reached through the shim
(deny file-read* (literal "{home}/.cargo/bin/jevify"))
(deny process-exec* (literal "{home}/.cargo/bin/jevify"))
(deny file-read* (regex #"/target/[^/]+/jevify$"))
(deny process-exec* (regex #"/target/[^/]+/jevify$"))
"""

PROMPT = """You are a coding agent. Your working directory is a git repository at {work}, a checkout
of hyperfine, a command-line benchmarking tool. Data files for the task are in {data}.
Work read-only{staging}.

{doc}

Task: {task}

You must use {tool} for this task: run it at least once for the step that chooses among
candidates or judges meaning, and let what it returns decide your answer. Be brief.

The last line of your final message must be exactly

    ANSWER: <value>

where <value> is {answer_format}, and nothing else. If you cannot tell, write ANSWER: unknown.
"""

DOC = {
    "jevify": ("The command-line tool jevify is installed and on your PATH (`jevify --help`,\n"
               "`jevify <command> --help`). Its instructions for agents follow.\n\n{text}"),
    "thin": ("The command-line tool jev is installed and on your PATH (`jev --help`).\n"
             "Its documentation follows.\n\n{text}"),
}
TOOL = {"jevify": "jevify", "thin": "jev"}


def sh(*a, timeout=120):
    return subprocess.run(a, capture_output=True, text=True, timeout=timeout)


def sha256(p):
    return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()


# ------------------------------------------------------------------ stage

def ensure_corpus():
    if not CORPUS.is_dir():
        CORPUS.parent.mkdir(parents=True, exist_ok=True)
        local = HOME / "jevify-study/corpus/hyperfine"
        src = str(local) if (local / ".git").is_dir() else CORPUS_URL
        subprocess.run(["git", "clone", "--quiet", src, str(CORPUS)], check=True, timeout=900)
        if src != CORPUS_URL:
            # the remote branches are part of task pk1, so they come from the source clone
            subprocess.run(["git", "-C", str(CORPUS), "fetch", "--quiet", src,
                            "+refs/remotes/origin/*:refs/remotes/origin/*"], check=True, timeout=300)
    subprocess.run(["git", "-C", str(CORPUS), "checkout", "--quiet", "--detach", CORPUS_PIN],
                   check=True, timeout=120)
    head = sh("git", "-C", str(CORPUS), "rev-parse", "HEAD").stdout.strip()
    assert head == CORPUS_PIN, f"corpus at {head}, not {CORPUS_PIN}"
    return head


def cmd_stage(a):
    ensure_corpus()
    d = ROOT / "bin" / a.label
    d.mkdir(parents=True, exist_ok=True)
    name = TOOL[a.kind]
    dst = d / name
    shutil.copy2(a.src, dst)
    dst.chmod(0o755)
    shutil.copy2(HERE / "calllog.py", ROOT / "bin" / "calllog.py")
    info = {"label": a.label, "kind": a.kind, "src": str(a.src), "sha256": sha256(dst),
            "commit": a.commit, "staged_at": time.strftime("%Y-%m-%dT%H:%M:%S%z")}
    if a.kind == "jevify":
        info["version"] = sh(str(dst), "--version").stdout.strip()
        doc = sh(str(dst), "init", "agents").stdout
    else:
        info["version"] = "thinjev " + (a.commit or "")
        # the README's usage and contract paragraphs; its title and the sentences
        # about the implementation and about jevify are not what a user of jev reads
        paras = (pathlib.Path(a.src).parent / "README.md").read_text().split("\n\n")
        doc = re.sub(r",\s+which is what makes.*", ".", "\n\n".join(paras[2:]), flags=re.S)
    (d / "doc.txt").write_text(doc.strip("\n") + "\n")
    (d / "stage.json").write_text(json.dumps(info, indent=1))
    print(json.dumps(info))


def load_stage(label):
    d = ROOT / "bin" / label
    info = json.loads((d / "stage.json").read_text())
    return d, info


# ------------------------------------------------------------------ build

def apply_edits(work, edits):
    for e in edits:
        f = work / e["file"]
        text = f.read_text()
        if "append" in e:
            text += e["append"]
        else:
            assert e["find"] in text, f"{e['find']!r} not in {e['file']}"
            text = text.replace(e["find"], e["replace"], 1)
        f.write_text(text)


def build(phase, stage_label, task, model, rep, others):
    stage, info = load_stage(stage_label)
    kind = info["kind"]
    rid = f"{task['id']}-{model}-r{rep}"
    run = ROOT / "runs" / phase / rid
    if run.exists():
        raise SystemExit(f"{run} exists; runs are never overwritten: use a new --phase or --rep-offset")
    for s in ("bin", "cache", "tmp"):
        (run / s).mkdir(parents=True)
    work = run / "work" / "hyperfine"
    work.parent.mkdir()
    subprocess.run(["cp", "-Rc", str(CORPUS), str(work)], check=True, timeout=300)
    assert sh("git", "-C", str(work), "rev-parse", "HEAD").stdout.strip() == CORPUS_PIN
    data = fixtures.write(run / "data")
    if task.get("edits"):
        apply_edits(work, task["edits"])

    scratch = pathlib.Path("/private/tmp/claude-501") / ("-" + str(work).strip("/").replace("/", "-"))
    scratch.mkdir(parents=True, exist_ok=True)
    claude_home = HOME / ".claude"
    claude_denies = "\n".join(
        f'(deny file-read* (subpath "{claude_home / n}"))' if (claude_home / n).is_dir()
        else f'(deny file-read* (literal "{claude_home / n}"))' for n in SHARED_CLAUDE_DENY)
    other_rules = "\n".join(f'(deny file-read-data (subpath "{o}"))' for o in others if o != run)
    (run / "profile.sb").write_text(PROFILE.format(
        run=run, claude_home=claude_home, claude_denies=claude_denies, scratch=scratch,
        repo=REPO, home=HOME, root=ROOT, others=other_rules, stage=stage))

    tool = TOOL[kind]
    shim = run / "bin" / tool
    shim.write_text("#!/bin/sh\n"
                    f'ERGO_LOG="{run}/calls.jsonl" ERGO_REAL="{stage / tool}" '
                    f'JEVIFY_CACHE_DIR="{run}/cache" exec python3 "{ROOT}/bin/calllog.py" "$@"\n')
    shim.chmod(0o755)
    (run / "calls.jsonl").write_text("")
    prompt = PROMPT.format(
        work=work, data=data, staging=" except for staging with git when the task asks for it"
        if task["verb"] == "add" else "", doc=DOC[kind].format(text=(stage / "doc.txt").read_text().strip()),
        task=task["task"].format(data=data), tool=tool, answer_format=task["answer_format"])
    (run / "prompt.txt").write_text(prompt)
    (run / "run.json").write_text(json.dumps({"rid": rid, "phase": phase, "task": task["id"],
                                              "verb": task["verb"], "model": model, "rep": rep,
                                              "stage": info}, indent=1))
    return run


def phase_run_dirs(phase):
    """Every run directory that is not under this phase, plus this phase's own:
    each run's profile denies all of them but itself."""
    runs = ROOT / "runs"
    out = [p for p in runs.iterdir() if p.is_dir() and p.name != phase] if runs.is_dir() else []
    return out


# ------------------------------------------------------------------ run

TOOL_CALL = {t: re.compile(r"(?:^|[|;&(`\n]|\bxargs\s|\btime\s|\bthen\s|\bdo\s|\bif\s|!\s)\s*"
                           r"(?:[A-Za-z_]+=\S*\s+)*(?:\S*/)?" + t + r"(?=\s|$|\))")
             for t in ("jevify", "jev")}


def parse_transcript(text):
    msgs = []
    for line in text.splitlines():
        try:
            msgs.append(json.loads(line))
        except ValueError:
            pass
    res = next((m for m in reversed(msgs) if m.get("type") == "result"), {})
    bash = [c.get("input", {}).get("command", "") for m in msgs if m.get("type") == "assistant"
            for c in (m.get("message") or {}).get("content") or [] if c.get("type") == "tool_use"
            and c.get("name") == "Bash"]
    tools = sum(1 for m in msgs if m.get("type") == "assistant"
                for c in (m.get("message") or {}).get("content") or [] if c.get("type") == "tool_use")
    return res, bash, tools


def extract_answer(text):
    for line in reversed((text or "").splitlines()):
        s = line.strip().strip("*`")
        if s.startswith("ANSWER:"):
            return s.split("ANSWER:", 1)[1].strip().strip("`*\"' ")
    return ""


def norm_path(s):
    s = s.strip().strip("`'\"").rstrip("/")
    for pre in (str(CORPUS) + "/",):
        s = s.replace(pre, "")
    s = re.sub(r"^.*?/work/hyperfine/", "", s)
    return s[2:] if s.startswith("./") else s


def score(task, ans, run):
    m, g = task["match"], task["gold"]
    a = (ans or "").strip()
    if m == "staged":
        work = run / "work" / "hyperfine"
        diff = sh("git", "-C", str(work), "diff", "--cached").stdout
        return all(x in diff for x in g["include"]) and not any(x in diff for x in g["exclude"])
    if not a or a.lower() == "unknown":
        return False
    if m == "commit":
        if not re.fullmatch(r"[0-9a-fA-F]{7,40}", a):
            return False
        r = sh("git", "-C", str(CORPUS), "rev-parse", "--verify", "--quiet", a + "^{commit}").stdout.strip()
        gg = sh("git", "-C", str(CORPUS), "rev-parse", g + "^{commit}").stdout.strip()
        return bool(r) and r == gg
    if m == "path":
        p = norm_path(a)
        return p == g or p.endswith("/" + g)
    if m == "word":
        w = a.lower().strip(".`'\" ")
        return w in [x.lower() for x in g]
    if m == "int":
        n = re.search(r"-?\d+", a)
        return bool(n) and abs(int(n.group()) - g) <= task.get("tol", 0)
    if m == "yesno":
        return a.lower().strip(". ").split()[0] == g if a.split() else False
    raise ValueError(m)


def run_one(run, model, budget, timeout):
    meta_f = run / "meta.json"
    rj = json.loads((run / "run.json").read_text())
    task = TASK[rj["task"]]
    tool = TOOL[rj["stage"]["kind"]]
    env = dict(os.environ)
    for k in ("TYPESAFE_API_KEY", "TYPESAFE_API_KEY_FILE", "JEVIFY_THRESHOLD", "JEVIFY_MODEL"):
        env.pop(k, None)  # keyless, default settings: the tool as a new user gets it
    work = run / "work" / "hyperfine"
    env.update(TMPDIR=str(run / "tmp"), PWD=str(work), JEVIFY_CACHE_DIR=str(run / "cache"),
               PATH=f"{run / 'bin'}:{env['PATH']}")
    cmd = ["/usr/bin/sandbox-exec", "-f", str(run / "profile.sb"),
           "claude", "-p", "--safe-mode", "--no-session-persistence",
           "--permission-mode", "bypassPermissions",
           "--model", model, "--output-format", "stream-json", "--verbose",
           "--max-budget-usd", str(budget),
           "--tools", "Bash", "Read", "Grep", "Glob"]
    t0 = time.time()
    timed_out = False
    try:
        p = subprocess.run(cmd, cwd=str(work), env=env, input=(run / "prompt.txt").read_text(),
                           capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        timed_out = True
        out = e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
        p = subprocess.CompletedProcess(cmd, -1, out, "")
    wall = time.time() - t0
    (run / "transcript.jsonl").write_text(p.stdout)
    (run / "stderr.txt").write_text(p.stderr or "")
    res, bash, tools = parse_transcript(p.stdout)
    ans = extract_answer(res.get("result"))
    calls = read_calls(run)
    typed = sum(len(TOOL_CALL[tool].findall(c)) for c in bash)
    u = res.get("usage") or {}
    meta = {
        **{k: rj[k] for k in ("rid", "phase", "task", "verb", "model", "rep")},
        "stage": rj["stage"]["label"], "kind": rj["stage"]["kind"],
        "stage_sha256": rj["stage"]["sha256"],
        "turns": res.get("num_turns"), "tool_uses": tools, "bash_commands": len(bash),
        "input_tokens": (u.get("input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0)
                        + (u.get("cache_creation_input_tokens") or 0),
        "output_tokens": u.get("output_tokens"),
        "cost_usd": res.get("total_cost_usd") or 0.0, "wall_s": round(wall, 1),
        "stop": "timeout" if timed_out else (res.get("subtype") or "no-result"),
        "answer": ans, "correct": score(task, ans, run),
        "calls_logged": len(calls), "calls_typed": typed,
        "calls_unfinished": sum(1 for c in calls if c.get("exit") is None),
        "cli_exit": p.returncode,
    }
    meta_f.write_text(json.dumps(meta, indent=1))
    return meta


def read_calls(run):
    """One record per call: the start line joined with its end line, in start order."""
    starts, ends = [], {}
    for l in (run / "calls.jsonl").read_text().splitlines():
        if not l.strip():
            continue
        r = json.loads(l)
        (starts.append(r) if r["phase"] == "start" else ends.__setitem__(r["id"], r))
    out = []
    for s in starts:
        e = ends.get(s["id"], {})
        out.append({**s, **{k: v for k, v in e.items() if k != "phase"}, "exit": e.get("exit")})
    return out


def parse_models(spec):
    out = []
    for part in spec.split(","):
        m, _, n = part.partition(":")
        out.append((m, int(n or 1)))
    return out


def cmd_run(a):
    ensure_corpus()
    ids = [t["id"] for t in TASKS] if a.tasks == "all" else a.tasks.split(",")
    plan = [(TASK[i], m, r) for m, n in parse_models(a.models) for r in range(1 + a.rep_offset, n + 1 + a.rep_offset)
            for i in ids]
    # every run of the phase is built before any starts, so each profile can name
    # all of its siblings: a run created later would be missing from earlier profiles
    phase_dir = ROOT / "runs" / a.phase
    phase_dir.mkdir(parents=True, exist_ok=True)
    rids = [f"{t['id']}-{m}-r{r}" for t, m, r in plan]
    others = phase_run_dirs(a.phase) + [p for p in phase_dir.iterdir() if p.is_dir()] \
        + [phase_dir / rid for rid in rids]
    runs = [(build(a.phase, a.stage, t, m, r, others), m) for t, m, r in plan]
    print(f"built {len(runs)} runs under {phase_dir}", flush=True)

    spent = [0.0]
    lock = threading.Lock()
    stop = threading.Event()

    def go(item):
        run, model = item
        if stop.is_set():
            return {"rid": run.name, "skipped": "budget"}
        m = run_one(run, model, a.budget, a.timeout)
        with lock:
            spent[0] += m["cost_usd"]
            if spent[0] >= a.max_total:
                stop.set()
            done = f"{spent[0]:.2f}"
        print(f"{m['rid']:26s} turns={m['turns']} calls={m['calls_logged']}/{m['calls_typed']} "
              f"exits={[c.get('exit') for c in read_calls(run)]} correct={m['correct']} "
              f"cost={m['cost_usd']:.3f} total={done}", flush=True)
        return m

    with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as ex:
        results = list(ex.map(go, runs))
    skipped = [r["rid"] for r in results if r.get("skipped")]
    print(json.dumps({"phase": a.phase, "runs": len(runs), "spent_usd": round(spent[0], 3),
                      "skipped_for_budget": skipped}))


# ------------------------------------------------------------------ canary

def cmd_canary(a):
    ensure_corpus()
    _, info = load_stage(a.stage)
    tool = TOOL[info["kind"]]
    phase = f"canary-{a.stage}-{int(time.time())}"
    phase_dir = ROOT / "runs" / phase
    phase_dir.mkdir(parents=True)
    sib = phase_dir / "sibling"
    (sib).mkdir()
    (sib / "answer.txt").write_text("SIBLING-SECRET\n")
    others = phase_run_dirs(phase) + [sib]
    run = build(phase, a.stage, TASK["ft4"], "haiku", 0, others)
    work, data = run / "work" / "hyperfine", run / "data"
    other_stage = next((p for p in (ROOT / "bin").iterdir() if p.is_dir() and p.name != a.stage), None)
    checks = [
        ("read the harness and its gold answers", f"head -c 60 {REPO}/scripts/ergonomics/tasks.jsonl"),
        ("cd into the harness and read AGENTS.md", f"cd {REPO} && head -1 AGENTS.md"),
        ("list ~/.ssh", "ls ~/.ssh"),
        ("read the TypeSafe key", "cat ~/.ssh/typesafe-ai-key"),
        ("read the paired-study tree", f"ls {HOME}/jevify-study/corpus"),
        ("read the pinned corpus", f"git -C {CORPUS} log -1 --oneline"),
        ("read a sibling run", f"cat {sib}/answer.txt"),
        ("read another stage", f"ls {other_stage}" if other_stage else "false"),
        ("exec ~/.cargo/bin/jevify", "~/.cargo/bin/jevify --version"),
        ("exec a target/release/jevify", f"{REPO}/target/release/jevify --version"),
        ("write into ~/Projects", f"touch {HOME}/Projects/CANARY-ergo"),
        ("write into the home directory", "touch ~/CANARY-ergo"),
        ("write into /tmp", "echo x > /tmp/canary-ergo.txt"),
        ("write the CLI's /tmp/claude-XXXX-cwd (allowed)", "pwd -P > /tmp/claude-c0de-cwd && echo ok"),
        ("a zsh here-document (allowed)", "/bin/zsh -c 'cat <<EOF\nheredoc ok\nEOF'"),
        ("read the operator's ~/.claude/settings.json", "head -c 20 ~/.claude/settings.json"),
        ("read the operator's ~/.claude/CLAUDE.md", "head -c 20 ~/.claude/CLAUDE.md"),
        ("write inside the run's working copy (allowed)", "touch ok.txt && echo ok"),
        ("git log in the run's working copy (allowed)", "git log -1 --oneline"),
        ("read the run's data (allowed)", f"head -1 {data}/tickets.txt"),
        (f"exec {tool} through the shim (allowed)", f"{tool} --help | head -1"),
        ("the network (allowed: the tool is keyless)", "curl -s -o /dev/null -w '%{http_code}' https://classifier.dev"),
    ]
    env = {**os.environ, "TMPDIR": str(run / "tmp"), "PWD": str(work),
           "PATH": f"{run / 'bin'}:{os.environ['PATH']}"}
    for k in ("TYPESAFE_API_KEY", "TYPESAFE_API_KEY_FILE"):
        env.pop(k, None)
    print(f"profile {run}/profile.sb  stage {info['label']} {info['sha256'][:12]}")
    for name, c in checks:
        p = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(run / "profile.sb"), "/bin/sh", "-c", c],
                           cwd=str(work), capture_output=True, text=True, env=env, timeout=60)
        out = (p.stdout + p.stderr).strip().splitlines()
        print(f"  {'ALLOWED' if p.returncode == 0 else 'denied ':8s} exit={p.returncode:<4d} {name:48s} "
              f"{(out[-1] if out else '')[:80]}")
    # the call log: a set of calls through the shim, including a failing one and one
    # whose stdin is /dev/null, must each leave a complete record
    probes = ([f"{tool} pick --files one two three < /dev/null", f"{tool} --version",
               f"printf 'alpha the cat sleeps\\nbeta the dog barks\\n' | {tool} pick 'the one about a dog'"]
              if tool == "jevify" else
              [f"{tool} < /dev/null", f"printf 'the cat sleeps\\nthe dog barks\\n' | {tool} 'which is about a dog'"])
    for c in probes:
        p = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(run / "profile.sb"), "/bin/sh", "-c", c],
                           cwd=str(work), capture_output=True, text=True, env=env, timeout=120)
        print(f"  probe exit={p.returncode:<4d} {c[:70]}")
    calls = read_calls(run)
    print(f"  call log: {len(calls)} records for {len(probes) + 1} shim invocations "
          f"(the --help check above is one)")
    for c in calls:
        print(f"    exit={c['exit']} stdin={c['stdin']} argv={c['argv']} "
              f"stdout={len(c.get('stdout') or '')}B stderr={(c.get('stderr') or '').strip()[:70]!r}")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("stage")
    s.add_argument("--label", required=True)
    s.add_argument("--kind", required=True, choices=list(TOOL))
    s.add_argument("--src", required=True, type=pathlib.Path)
    s.add_argument("--commit", default=None, help="the source commit the binary was built from")
    r = sub.add_parser("run")
    r.add_argument("--phase", required=True)
    r.add_argument("--stage", required=True)
    r.add_argument("--models", default="haiku:3,sonnet:1", help="model:reps,...")
    r.add_argument("--tasks", default="all")
    r.add_argument("--rep-offset", type=int, default=0)
    r.add_argument("--jobs", type=int, default=8)
    r.add_argument("--budget", type=float, default=0.25, help="max USD per run")
    r.add_argument("--max-total", type=float, default=12.0, help="stop starting runs past this USD")
    r.add_argument("--timeout", type=float, default=420)
    c = sub.add_parser("canary")
    c.add_argument("--stage", required=True)
    a = ap.parse_args()
    {"stage": cmd_stage, "run": cmd_run, "canary": cmd_canary}[a.cmd](a)


if __name__ == "__main__":
    main()
