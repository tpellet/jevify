#!/bin/sh
# Run a block of cells, one at a time.
#
#   sh scripts/study/run_study.sh "D1 D2" 3 haiku
#   ARMS="available required" sh scripts/study/run_study.sh "D1" 5
#
# Three arms by default: control, available, required. They are reported
# separately and never averaged, so nothing here mixes them either.
#
# Sequential on purpose: two runs at once share the machine and the keyless
# backend's rate, and the wall time of a cell is one of the numbers being
# measured. Each cell writes its own directory; a cell that fails does not stop
# the block.
set -u
TASKS="${1:?usage: run_study.sh \"D1 D2\" <reps> [model]}"
REPS="${2:-3}"
MODEL="${3:-haiku}"
ARMS="${ARMS:-control available required}"
DIR="$(cd "$(dirname "$0")" && pwd)"
R=1
while [ "$R" -le "$REPS" ]; do
  for T in $TASKS; do
    for ARM in $ARMS; do
      echo "--- $T $ARM r$R"
      python3 "$DIR/run_cell.py" --task "$T" --arm "$ARM" --rep "$R" --model "$MODEL" || echo "CELL FAILED: $T $ARM r$R"
    done
  done
  R=$((R + 1))
done
