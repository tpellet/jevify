#!/bin/sh
# Run a block of cells, one at a time.
#
#   sh scripts/study/run_study.sh "S1 S8" 3 haiku
#
# Sequential on purpose: two runs at once share the machine and the keyless
# backend's rate, and the wall time of a cell is one of the numbers being
# measured. Each cell writes its own directory; a cell that fails does not stop
# the block.
set -u
TASKS="${1:?usage: run_study.sh \"S1 S8\" <reps> [model]}"
REPS="${2:-3}"
MODEL="${3:-haiku}"
DIR="$(cd "$(dirname "$0")" && pwd)"
R=1
while [ "$R" -le "$REPS" ]; do
  for T in $TASKS; do
    for ARM in with without; do
      echo "--- $T $ARM r$R"
      python3 "$DIR/run_cell.py" --task "$T" --arm "$ARM" --rep "$R" --model "$MODEL" || echo "CELL FAILED: $T $ARM r$R"
    done
  done
  R=$((R + 1))
done
