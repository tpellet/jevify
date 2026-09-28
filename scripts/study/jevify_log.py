#!/usr/bin/env python3
"""The only executable copy of jevify a run can reach, wrapped so every call is
recorded whether or not the agent says it made one.

The Seatbelt profile denies exec on ~/.cargo/bin/jevify and on any
target/*/jevify in BOTH arms, and allows exec on $JEVSTUDY/bin/jevify only for
the available and required arms. So an agent cannot reach the binary except through this wrapper,
and the log is complete by construction rather than by the agent's report.

Arguments are passed through byte for byte. When the agent did not ask for a
machine format, a shadow call with --json (and --dry-run for fill/add) runs
first on the same stdin so the envelope's request, question and cache figures
are recorded; the agent's own call then replays those answers from the run's
cache. One JSON line per agent call goes to $JEVSTUDY_LOG, with the head of
what the tool printed on stdout AND on stderr: a call that fails prints its
reason on stderr only, and a log of stdout alone reads it as "printed nothing".

`--thinjev` as the first argument logs the thin baseline `jev` instead (the
`thin` arm): same log line, one call, no shadow, since `jev` has no envelope.
"""

import json
import os
import subprocess
import sys
import time

# the binary sits next to this file, both put there by setup.sh: the executable is
# fixed by where this script lives and is never taken from the environment
THIN = sys.argv[1:2] == ["--thinjev"]
REAL = os.path.join(os.path.dirname(os.path.realpath(__file__)), "jev" if THIN else "jevify")
if not (os.path.isfile(REAL) and os.access(REAL, os.X_OK)):
    sys.exit(f"no executable {os.path.basename(REAL)} next to {__file__}")
LOG = os.environ["JEVSTUDY_LOG"]
args = sys.argv[2:] if THIN else sys.argv[1:]
os.environ.pop("TYPESAFE_API_KEY_FILE", None)  # the study is keyless in both arms
os.environ.pop("TYPESAFE_API_KEY", None)

pre = args[: args.index("--")] if "--" in args else args
verb = next((a for a in pre if not a.startswith("-")), None)
flags = [a for a in pre if a.startswith("-")]
machine = any(f in ("--json", "--jsonl", "--toon") or f.startswith("--format") for f in flags)
if THIN:
    reads_stdin = len(args) == 1  # jev takes its options from stdin only when argv has none
else:
    reads_stdin = verb in ("pick", "filter", "label", "why", "is", "add") and "--from" not in flags \
        and not (verb == "is" and "--context" in flags)
data = sys.stdin.buffer.read() if (reads_stdin and not sys.stdin.isatty()) else None


def run(argv, stdin_bytes):
    t0 = time.time()
    kw = {"input": stdin_bytes} if stdin_bytes is not None else {"stdin": subprocess.DEVNULL}
    exe = [sys.executable, REAL] if THIN else [REAL]
    p = subprocess.run(exe + argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kw)
    return p, int((time.time() - t0) * 1000)


HEAD_CHARS = 4000  # enough to hold the handle jevify chose, short enough to read


def head(b):
    """What the tool printed, clipped. The required arm asks how often jevify's
    answer became the agent's, and that cannot be checked from counters alone:
    it needs the value the tool actually printed."""
    return b[:HEAD_CHARS].decode("utf-8", "replace")


def meta_of(stdout):
    try:
        j = json.loads(stdout.decode("utf-8", "replace"))
    except Exception:
        return None
    m = j.get("meta") or {}
    t = m.get("telemetry") or {}
    return {"exit_code": j.get("exit_code"), "error_kind": (j.get("error") or {}).get("kind"),
            "model": m.get("model"), "requests": m.get("requests"), "cache_hits": m.get("cache_hits"),
            "questions": t.get("semantic_questions"), "elapsed_ms": m.get("elapsed_ms")}


rec = {"ts": round(time.time(), 3), "tool": "jev" if THIN else "jevify", "verb": verb, "flags": flags, "argv": args,
       "stdin_bytes": None if data is None else len(data), "shadow": None}
if machine or THIN:
    p, ms = run(args, data)
    rec.update(exit=p.returncode, elapsed_ms=ms, meta=None if THIN else meta_of(p.stdout),
               stdout_head=head(p.stdout), stderr_head=head(p.stderr))
else:
    sh = list(args)
    ins = sh.index("--") if "--" in sh else len(sh)
    extra = ["--json"] + (["--dry-run"] if verb in ("fill", "add") and "--dry-run" not in flags else [])
    sh = sh[:ins] + extra + sh[ins:]
    sp, sms = run(sh, data)
    rec["shadow"] = {"argv": sh, "exit": sp.returncode, "elapsed_ms": sms,
                     "meta": meta_of(sp.stdout), "stdout_head": head(sp.stdout),
                     "stderr_head": head(sp.stderr)}
    p, ms = run(args, data)
    rec.update(exit=p.returncode, elapsed_ms=ms, stdout_head=head(p.stdout), stderr_head=head(p.stderr),
               meta=meta_of(p.stdout) if p.stdout[:1] == b"{" else rec["shadow"]["meta"])

try:
    sys.stdout.buffer.write(p.stdout)
    sys.stdout.flush()
    sys.stderr.buffer.write(p.stderr)
    sys.stderr.flush()
except BrokenPipeError:
    pass
err = p.stderr.decode("utf-8", "replace")
rec["stderr_status"] = [l for l in err.splitlines() if l.startswith(("jevify", "jev:"))][-4:]
with open(LOG, "a") as f:
    f.write(json.dumps(rec) + "\n")
sys.exit(p.returncode)
