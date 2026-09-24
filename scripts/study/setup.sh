#!/bin/sh
# Prepare the study tree outside the repository. Idempotent.
#
#   sh scripts/study/setup.sh [path-to-jevify-binary]
#
# Clones each repository the task set names, pins it to the sha in
# tasks.jsonl, and copies the binary under study plus its logging wrapper into
# $JEVSTUDY/bin. That copy is the only jevify a run can execute: the Seatbelt
# profile denies exec on ~/.cargo/bin/jevify and on any target/*/jevify.
set -eu
STUDY="${JEVSTUDY:-$HOME/jevify-study}"
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${1:-$HOME/.cargo/bin/jevify}"

mkdir -p "$STUDY/corpus" "$STUDY/bin" "$STUDY/runs"

python3 - "$REPO" "$STUDY" <<'PY'
import json, os, subprocess, sys
repo, study = sys.argv[1], sys.argv[2]
seen = {}
for line in open(os.path.join(repo, "scripts/study/tasks.jsonl")):
    if not line.strip():
        continue
    t = json.loads(line)
    seen.setdefault(t["repo"], (t["url"], t["pin"]))
for name, (url, pin) in seen.items():
    d = os.path.join(study, "corpus", name)
    if not os.path.isdir(d):
        subprocess.run(["git", "clone", "--quiet", url, d], check=True)
    subprocess.run(["git", "-C", d, "checkout", "--quiet", "--detach", pin], check=True)
    head = subprocess.run(["git", "-C", d, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    print(f"{name:12s} {head} {'ok' if head == pin else 'PIN MISMATCH'}")
PY

cp "$BIN" "$STUDY/bin/jevify"
cp "$REPO/scripts/study/jevify_log.py" "$STUDY/bin/jevify_log.py"
"$STUDY/bin/jevify" init agents > "$STUDY/bin/init-agents.txt"
"$STUDY/bin/jevify" --version > "$STUDY/bin/VERSION.txt"
shasum -a 256 "$STUDY/bin/jevify" | cut -d' ' -f1 > "$STUDY/bin/jevify.sha256"
echo "jevify   $(cat "$STUDY/bin/VERSION.txt") $(cat "$STUDY/bin/jevify.sha256")"
echo "agents block $(wc -c < "$STUDY/bin/init-agents.txt" | tr -d ' ') bytes"
