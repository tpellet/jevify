#!/usr/bin/env bash
# Regenerate evals/scorecard/SCORECARD.md: every gold-labelled set through the released binary on
# the keyless backend (and on TypeSafe when TYPESAFE_API_KEY_FILE is in the environment), the
# same items through the thin wrapper scripts/thinjev/jev, then the report.
#
#   bash evals/scorecard/run.sh
#   TYPESAFE_API_KEY_FILE=/path/to/key bash evals/scorecard/run.sh
#
# JEVIFY_BIN     the binary under test (default ~/.cargo/bin/jevify)
# SCORECARD_OUT  where runs are written (default evals/out/scorecard, untracked)
# CAP_KEYLESS    keyless classifications this run may spend (default 6000)
# CAP_TYPESAFE   TypeSafe classifications this run may spend (default 20000)
# RESUME=1       keep a step's existing output instead of running it again
#
# Before each step the ledger adds the step's estimate to what the run has spent; a step that
# would cross a cap is not started, and the run stops with exit 5.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${JEVIFY_BIN:-$HOME/.cargo/bin/jevify}
OUT=${SCORECARD_OUT:-$ROOT/evals/out/scorecard}
CAP_KEYLESS=${CAP_KEYLESS:-6000}
CAP_TYPESAFE=${CAP_TYPESAFE:-20000}
HELPER="$ROOT/evals/scorecard/scorecard.py"
RG_PIN=3fce3b5bb0236da2df6d99672afb8a719642eca7
mkdir -p "$OUT/repos"

backends=classifier
if [ -n "${TYPESAFE_API_KEY_FILE:-}" ]; then backends="classifier typesafe"; fi

log() { printf '[scorecard %s] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

# budget <keyless estimate> <typesafe estimate> <step name>
budget() {
  if ! python3 "$HELPER" spent "$OUT" $((CAP_KEYLESS - $1)) $((CAP_TYPESAFE - $2)) >&2; then
    log "stopping before $3: its estimate would cross a cap"
    exit 5
  fi
}

# step <output file> <command...>: skipped under RESUME=1 when the output is already there
step() {
  local out=$1
  shift
  if [ -n "${RESUME:-}" ] && [ -s "$out" ]; then
    log "keeping $out"
    return 0
  fi
  log "-> $out"
  "$@"
}

est() { # est <backend> <keyless> <typesafe>: the estimate that applies to this backend
  if [ "$1" = typesafe ]; then echo "0 $3"; else echo "$2 0"; fi
}

log "checking the manifests"
python3 "$ROOT/scripts/validation_gold.py" check >&2
python3 "$ROOT/scripts/holdout_score.py" check >&2

log "stamping $BIN"
python3 - "$BIN" "$OUT/stamp.json" "$HELPER" "$ROOT" <<'EOF'
import datetime, hashlib, json, subprocess, sys
binary, out, helper, root = sys.argv[1:5]
stamp = {
    "date": datetime.date.today().isoformat(),
    "host": "a macOS dev machine",
    "version": subprocess.run([binary, "--version"], capture_output=True, text=True,
                              check=True).stdout.strip(),
    "sha256": hashlib.sha256(open(binary, "rb").read()).hexdigest(),
    "thin_sha": subprocess.run(["git", "-C", root, "log", "-1", "--format=%h", "--",
                                "scripts/thinjev/jev"], capture_output=True, text=True,
                               check=True).stdout.strip(),
    "thin_model": subprocess.run([sys.executable, helper, "model"], capture_output=True,
                                 text=True, check=True).stdout.strip(),
}
open(out, "w").write(json.dumps(stamp, indent=1) + "\n")
print(json.dumps(stamp), file=sys.stderr)
EOF

log "preparing the commit-subjects repositories"
if [ ! -d "$OUT/repos/liars" ]; then
  bash "$ROOT/evals/commit-subjects/make_repo.sh" "$OUT/repos/liars" >&2
fi
if [ ! -d "$OUT/repos/ripgrep/.git" ]; then
  git clone --quiet https://github.com/BurntSushi/ripgrep "$OUT/repos/ripgrep"
fi
git -C "$OUT/repos/ripgrep" checkout --quiet --detach "$RG_PIN"

for b in $backends; do
  # shellcheck disable=SC2046
  budget $(est "$b" 1500 1500) "validation $b"
  step "$OUT/validation-$b.jsonl" python3 "$ROOT/scripts/validation_run.py" --backend "$b" \
    --binary "$BIN" --out "$OUT/validation-$b.jsonl"
  # shellcheck disable=SC2046
  budget $(est "$b" 900 900) "holdout $b"
  step "$OUT/holdout-$b.jsonl" python3 "$ROOT/scripts/holdout_run.py" --backend "$b" \
    --binary "$BIN" --clone --out "$OUT/holdout-$b.jsonl"
  for arm in repeat order; do
    # shellcheck disable=SC2046
    budget $(est "$b" 700 700) "variance $arm $b"
    step "$OUT/$arm-$b.jsonl" python3 "$ROOT/evals/variance/variance_run.py" "$arm" \
      --backend "$b" --binary "$BIN" --scratch "$OUT/permuted" --out "$OUT/$arm-$b.jsonl"
  done
  for half in scratch ripgrep; do
    repo=$OUT/repos/liars
    if [ "$half" = ripgrep ]; then repo=$OUT/repos/ripgrep; fi
    # shellcheck disable=SC2046
    budget $(est "$b" 600 400) "commits $half $b"
    step "$OUT/commits-$half-$b.jsonl" env JEVIFY_BIN="$BIN" python3 \
      "$ROOT/evals/commit-subjects/run.py" "$b" "$ROOT/evals/commit-subjects/cases-$half.jsonl" \
      "$repo" "$OUT/commits-$half-$b.jsonl"
  done
done

for set in validation holdout; do
  budget 300 0 "thin $set"
  step "$OUT/$set-thin.jsonl" python3 "$HELPER" thin "$set" "$OUT/$set-thin.jsonl"
done
for arm in repeat order; do
  budget 300 0 "thin $arm"
  step "$OUT/$arm-thin.jsonl" python3 "$HELPER" thin-variance "$arm" "$OUT/$arm-thin.jsonl"
done
for half in scratch ripgrep; do
  repo=$OUT/repos/liars
  if [ "$half" = ripgrep ]; then repo=$OUT/repos/ripgrep; fi
  budget 20 0 "thin commits $half"
  step "$OUT/commits-$half-thin.jsonl" python3 "$HELPER" thin-commits \
    "$ROOT/evals/commit-subjects/cases-$half.jsonl" "$repo" "$OUT/commits-$half-thin.jsonl"
done

python3 "$HELPER" spent "$OUT" "$CAP_KEYLESS" "$CAP_TYPESAFE" >&2 || true
python3 "$HELPER" report "$OUT" > "$ROOT/evals/scorecard/SCORECARD.md"
log "wrote evals/scorecard/SCORECARD.md"
