# scripts/study — the paired study harness

One cell is one task, one arm, one repetition. The with arm has jevify and is
told about it; the without arm does not have it and is told so. Nothing else
differs between the two prompts.

```
sh  scripts/study/setup.sh [path-to-jevify]   # clone and pin the corpus, stage the binary
python3 scripts/study/canary.py               # what the sandbox lets each arm do
sh  scripts/study/run_study.sh "S1 S8" 3      # 2 tasks x 2 arms x 3 reps
python3 scripts/study/blind.py                # arm-free records + the unblinding map
python3 scripts/study/score.py                # score them without seeing the arm
python3 scripts/study/unblind.py              # join, per-cell Wilson intervals
python3 scripts/study/probe.py                # what one jevify call costs per task
python3 scripts/study/price.py --tasks 8 --reps 5   # project the full run
```

Everything a run touches lives under `$JEVSTUDY` (default `~/jevify-study`),
outside the repository. `tasks.jsonl` holds the gold answers and stays in the
repository, which the sandbox denies reading.

## The four properties, and where each is enforced

**The control arm is unrestricted.** Both arms get `Bash`, `Read`, `Grep` and
`Glob` and may run anything: `git log --grep`, `rg`, `find`, `fzf`, `cat` over a
whole listing. The two prompts are the same bytes except for one paragraph, and
the two Seatbelt profiles differ by two lines, which are the two lines that say
whether jevify may be executed. `diff` on either pair shows the whole
difference.

**The answers are checkable by a third party.** Every task names a public
repository at a sha, and every gold answer is a commit hash, a repository-
relative path or a branch name at that sha. `score.py` resolves both sides with
`git rev-parse` in the pinned clone, so 7, 8 or 40 characters score alike.
`provenance` in `tasks.jsonl` says which command produced the candidate list the
task was written from.

**The scoring does not see the arm.** `blind.py` writes
`$JEVSTUDY/blind/<bid>.json` holding the task, the answer and a leak boolean,
and nothing else; `bid` is a truncated sha256 of the run id, so the two arms of
a pair sort apart and the file name carries nothing. The unblinding map is
written to `$JEVSTUDY/unblind_map.json`, outside `blind/`. `score.py` opens only
`blind/*.json`, `tasks.jsonl` and the pinned clones, and asserts that no record
it reads carries an arm.

**The agent cannot leave the sandbox or see the answer.** `run_cell.py` runs
`/usr/bin/sandbox-exec -f <profile> claude -p ...`, so the sandbox wraps the
agent process itself. Every child process inherits it and so does every
in-process read by the built-in file tools; there is no per-command wrapper to
step around, and `cd` changes nothing about what may be touched. Writes are
denied everywhere but the run directory, the agent's own configuration and its
scratch directory. Reads of the repository (which holds `tasks.jsonl` and its
gold answers), of the study tree above the run, and of `~/.ssh` are denied.
Both arms are denied exec of `~/.cargo/bin/jevify` and of any `target/*/jevify`,
so the with arm's only reachable copy is the logged one and the call log is
complete by construction. After every run, `scan()` lists every file written
outside the run directory.

`canary.py` runs those checks through the same profiles the agents get and
prints what each arm may do.

## Reading the numbers

`meta.json` per cell: turns, tool uses, input and output tokens, wall time, the
agent's own inference cost, and for the with arm every jevify call with its
argv, exit code, request count and question count, read from the envelope and
not from the agent's report.

`unblind.py` reports each cell as k of n with a 95 % Wilson interval. At n = 3
that interval is about [0.29, 1.00] for 3 of 3 and [0.00, 0.56] for 0 of 3: wide
enough that no pair of cells at n = 3 separates. That is the point of printing
it.

## Known faults

They are written up in `benchmarks/agents/STUDY.md`, in the section of that
name, with what each one would take to close.
