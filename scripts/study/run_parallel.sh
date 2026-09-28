#!/bin/sh
# Run a block of cells with bounded concurrency.
#
#   P=6 MODELS="haiku:4:1 sonnet:2:2" ARMS="control thin available required" \
#       sh scripts/study/run_parallel.sh "D1 D2 F1"
#
# MODELS is a list of model:reps:budget_usd. Every cell of the block gets its run
# directory before any cell starts, so every Seatbelt profile names every sibling
# (a directory created after a profile was written would be readable from it).
# The cells run in a fixed shuffled order, so the arms interleave over the block
# and none of them meets the keyless backend's rate or the machine's load at a
# different time of the block than the others.
#
# Wall time is shared: P cells run at once and each one's wall_s includes waiting
# on the machine and on the backend. The concurrency is appended to
# $JEVSTUDY/concurrency.log with the time and the cell count, and belongs in any
# report that quotes a wall time. A cell that fails does not stop the block.
set -u
TASKS="${1:?usage: run_parallel.sh \"D1 D2\"}"
P="${P:-6}"
MODELS="${MODELS:-haiku:4:1}"
ARMS="${ARMS:-control thin available required}"
SEED="${SEED:-1}"
STUDY="${JEVSTUDY:-$HOME/jevify-study}"
DIR="$(cd "$(dirname "$0")" && pwd)"

CELLS="$(python3 - "$TASKS" "$ARMS" "$MODELS" "$SEED" <<'PY'
import random, sys
tasks, arms, models, seed = sys.argv[1].split(), sys.argv[2].split(), sys.argv[3].split(), int(sys.argv[4])
cells = []
for spec in models:
    model, reps, budget = spec.split(":")
    cells += [(t, a, model, r, budget) for t in tasks for a in arms for r in range(1, int(reps) + 1)]
random.Random(seed).shuffle(cells)
print("\n".join(" ".join(map(str, c)) for c in cells))
PY
)"
N="$(printf '%s\n' "$CELLS" | wc -l | tr -d ' ')"
printf '%s\n' "$CELLS" | while read -r T A M R B; do
  mkdir -p "$STUDY/runs/$T-$A-$M-r$R"
done
echo "$(date -u +%FT%TZ) P=$P cells=$N models=$MODELS arms=$ARMS tasks=$TASKS" >> "$STUDY/concurrency.log"
echo "running $N cells, $P at a time"
# xargs appends one cell's five words to the two fixed arguments:
# $0 harness dir, $1 study dir, $2 task, $3 arm, $4 model, $5 rep, $6 budget
# A cell with a meta.json finished in an earlier invocation and is skipped, so a
# block that was stopped resumes where it stopped; a cell stopped mid-run has no
# meta.json and runs again in a fresh working copy.
printf '%s\n' "$CELLS" | xargs -P "$P" -L 1 sh -c '
  [ -f "$1/runs/$2-$3-$4-r$5/meta.json" ] && { echo "skip   $2 $3 $4 r$5"; exit 0; }
  python3 "$0/run_cell.py" --task "$2" --arm "$3" --model "$4" --rep "$5" --budget "$6" \
    > "$1/runs/$2-$3-$4-r$5/cell.out" 2>&1 && echo "done   $2 $3 $4 r$5" || echo "FAILED $2 $3 $4 r$5"
' "$DIR" "$STUDY"
