# scripts/study — the paired study harness

One cell is one task, one arm, one model, one repetition. Four arms:

| arm | tool | what the prompt says | what the arm answers |
|:---|:---|:---|:---|
| `control` | none | "jevify is not installed on this machine" | the floor |
| `thin` | `scripts/thinjev/jev` | where it is, plus its usage, once | what the bare model adds, without jevify |
| `available` | jevify | where it is, plus `jevify init agents`, once | does an agent reach for it |
| `required` | jevify | the same, plus one sentence telling it to | does it help when used |

`available` and `required` get the same Seatbelt profile; the prompt is the only
difference between them. `thin` is `available` with jevify swapped for `jev`, one
keyless call to the same model ranking the options the agent hands it, with no
listers, no evidence, no second round and no threshold: `thin` against
`available` is what jevify adds over a wrapper of the API. The four are reported
separately and never averaged: `available` measures adoption, `required`
measures efficacy, and averaging them answers neither question.

```
sh  scripts/study/setup.sh [path-to-jevify]   # clone and pin the corpus, stage jevify and jev
python3 scripts/study/baseline.py D1 F1 ...   # prove the tasks are not lexically reachable
python3 scripts/study/canary.py               # what the sandbox lets each arm do
python3 scripts/study/price.py --from ~/jevify-study-d --tasks 17 --block haiku:4:control,thin
P=6 MODELS="haiku:4:1 sonnet:2:2" sh scripts/study/run_parallel.sh "D1 D2"   # 6 cells at a time
sh  scripts/study/run_study.sh "D1 D2" 3      # the same, one cell at a time
python3 scripts/study/blind.py                # arm-free records + the unblinding map
python3 scripts/study/score.py                # score them without seeing the arm
python3 scripts/study/unblind.py --max-rep 4  # join; per model and arm: intervals, adoption, exits
python3 scripts/study/probe.py                # what one jevify call costs per task
```

`run_parallel.sh` creates every run directory of the block before the first
cell starts, so each profile denies reading every sibling, and appends its
concurrency to `$JEVSTUDY/concurrency.log`: at `P=6` a cell's wall time includes
waiting on five others and on the shared keyless backend, and a report quoting
wall time says so.

## The task set has to be one lexical search cannot answer

A question `git log --grep` answers is a question jevify cannot improve, and a
task set where both arms are perfect measures nothing. `baseline.py` is the
gate: for each task it takes the question's own content words and runs them
through `git log --grep`, `git log -S`, `git log --all --grep`, `git grep -il`
and `git ls-files`, plus a handful of identifiers a reader of the question might
guess at, written down per task by hand. A probe that returns the gold among at
most three candidates is a HIT and the task is dropped. Four of fourteen
candidate tasks were dropped that way; `benchmarks/agents/ADOPTION.md` lists
them with the probe that killed each one.

Everything a run touches lives under `$JEVSTUDY` (default `~/jevify-study`),
outside the repository. `tasks.jsonl` holds the gold answers and stays in the
repository, which the sandbox denies reading.

## The four properties, and where each is enforced

**The control arm is unrestricted.** Every arm gets `Bash`, `Read`, `Grep` and
`Glob` and may run anything: `git log --grep`, `rg`, `find`, `fzf`, `cat` over a
whole listing. The prompts are the same bytes except for one paragraph, and the
profiles differ by two lines, which are the two lines that say whether jevify may
be executed. `diff` on any pair shows the whole difference.

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
denied everywhere but the run directory, the CLI's configuration directory and
the run's scratch directory. Inside the shared configuration directory every
path that is the operator's rather than the run's -- history, projects, skills,
plugins, settings, shell snapshots -- is denied for reading. Reads of the repository (which holds `tasks.jsonl` and its
gold answers), of the study tree above the run, and of `~/.ssh` are denied.
Both arms are denied exec of `~/.cargo/bin/jevify` and of any `target/*/jevify`,
so the with arm's only reachable copy is the logged one and the call log is
complete by construction. After every run, `scan()` lists every path outside the
run directory whose mtime moved. That is a smoke test and not attribution: an
mtime names no writer, and on a machine doing anything else it reports the
operator's own files. `canary.py` is the containment evidence.

`canary.py` runs those checks through the same profiles the agents get and
prints what each arm may do.

## Reading the numbers

`meta.json` per cell: turns, tool uses, input and output tokens, wall time, the
agent's own inference cost, and for the with arm every jevify call with its
argv, exit code, request count and question count, read from the envelope and
not from the agent's report.

`unblind.py` reports each cell as k of n with a 95 % Wilson interval, each arm
the same way over all its runs, the adoption rate in `available`, and for
`required` how often jevify was called and how often what it printed became the
agent's answer. It ends with every jevify call that abstained or named something
other than the gold, quoted.

At n = 3 a cell's interval is about [0.29, 1.00] for 3 of 3 and [0.00, 0.56] for
0 of 3: wide enough that no pair of cells at n = 3 separates. That is the point
of printing it. An arm's interval, over 8 tasks x 3 reps, is the one worth
reading.

`--max-rep` exists because pilot and canary cells share `runs/` with the study
and nothing may be deleted from a finished study tree; they are numbered above
the study's repetitions and excluded by number.

## The configuration directory

`--per-run-config` gives a cell its own `CLAUDE_CONFIG_DIR`, which closes the
shared-directory hole. It is off by default and the study was not run with it,
because on this machine the CLI consults the login Keychain only for the default
directory: a per-run directory has to carry the OAuth token as a file, and the
agent under test is the process that would read it. That is a worse hole than the
one it closes. Measured, not assumed -- a per-run directory seeded from
`~/.claude.json` alone, and a whole clone of `~/.claude`, both answer
`Not logged in`.

## Known faults

They are written up in `benchmarks/agents/STUDY.md`, in the section of that
name, with what each one would take to close.
