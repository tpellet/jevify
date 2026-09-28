#!/usr/bin/env python3
"""The logging shim between an agent and the tool under test (jevify or jev).

A run's bin/<tool> is a two-line sh wrapper that sets ERGO_LOG and ERGO_REAL and
execs this file. The shim is transparent: stdin is inherited, not read, so a call
that waits on a terminal waits here exactly as it would without the shim; stdout
and stderr are captured and then written through unchanged; the exit code is the
tool's. No shadow call is made, so every classification the log shows is one the
agent asked for.

Two JSON lines per call go to ERGO_LOG: a `start` line before the tool runs and an
`end` line after it. A call the agent's shell killed (a hang on stdin, a timeout)
leaves a `start` line whose `end` either never came or says `killed`, so a hang
is visible rather than missing.
"""

import json
import os
import signal
import stat
import subprocess
import sys
import time

REAL = os.environ["ERGO_REAL"]
LOG = os.environ["ERGO_LOG"]
CLIP = 12000
argv = sys.argv[1:]
cid = f"{os.getpid()}-{time.time():.6f}"


def stdin_kind():
    """What the agent's shell handed the tool as stdin: a pipe, a file, a
    terminal, /dev/null or nothing. `--files` with no stdin shows up here."""
    try:
        m = os.fstat(0).st_mode
    except OSError:
        return "closed"
    if stat.S_ISFIFO(m):
        return "pipe"
    if stat.S_ISREG(m):
        return "file"
    if os.isatty(0):
        return "tty"
    if stat.S_ISCHR(m):
        return "devnull" if os.path.samefile("/dev/fd/0", "/dev/null") else "chardev"
    if stat.S_ISSOCK(m):
        return "socket"
    return "other"


def log(rec):
    with open(LOG, "a") as f:
        f.write(json.dumps(rec) + "\n")


t0 = time.time()
kind = stdin_kind()
log({"phase": "start", "id": cid, "ts": round(t0, 3), "argv": argv, "cwd": os.getcwd(),
     "stdin": kind})

child = None


def on_signal(signum, _frame):
    if child is not None and child.poll() is None:
        child.send_signal(signum)
    log({"phase": "end", "id": cid, "argv": argv, "exit": 128 + signum, "killed": True,
         "signal": signum, "elapsed_ms": int((time.time() - t0) * 1000), "stdin": kind,
         "stdout": "", "stderr": ""})
    sys.exit(128 + signum)


for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
    signal.signal(s, on_signal)

cmd = [REAL] + argv if os.access(REAL, os.X_OK) else [sys.executable, REAL] + argv
child = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
out, err = child.communicate()
code = child.returncode if child.returncode >= 0 else 128 - child.returncode
log({"phase": "end", "id": cid, "argv": argv, "exit": code, "killed": False,
     "elapsed_ms": int((time.time() - t0) * 1000), "stdin": kind,
     "stdout_bytes": len(out), "stdout": out[:CLIP].decode("utf-8", "replace"),
     "stderr": err[:CLIP].decode("utf-8", "replace")})
try:
    sys.stdout.buffer.write(out)
    sys.stdout.flush()
    sys.stderr.buffer.write(err)
    sys.stderr.flush()
except BrokenPipeError:
    pass
sys.exit(code)
