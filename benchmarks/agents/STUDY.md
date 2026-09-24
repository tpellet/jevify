# The paired study harness, and twelve runs that validate it

`benchmarks/agents/PILOT.md` records two runs of eight tasks with one run per
cell, a baseline restricted in ways a real user is not, a sandbox one agent
stepped out of with `cd`, and one cell where the with arm never called jevify.
Those four faults belong to how the study was run. The harness in
`scripts/study/` is where they stop recurring, and the twelve runs below are
what shows it working, not a measurement of jevify.

Measured 2026-09-24 on a macOS dev machine. jevify 0.13.0, the binary built
from `2822f72` (sha256 `53d9bbeb77cee2…`), keyless in both arms against
classifier.dev, answering model `jev-1.13.0`. Agent model `claude-haiku-4-5`,
one agent process per cell.

## The task set

Eight tasks over three repositories that `evals/` does not draw from, each
pinned by sha. Every answer is a commit hash, a repository-relative path or a
branch name, so a third party scores it without knowing which arm produced it.

| Task | Repository at its pin | Kind | Answer |
|:---|:---|:---|:---|
| S1 | ripgrep `3fce3b5bb0236da2df6d99672afb8a719642eca7` | commit | `435f59f` |
| S2 | ripgrep, same pin | commit | `f55548b` |
| S3 | ripgrep, same pin | path | `crates/ignore/src/default_types.rs` |
| S4 | hyperfine `f12f3d9f86f3643b3b7deace5e160b1f0f44d2b7` | commit | `b5a6860` |
| S5 | hyperfine, same pin | branch | `track-memory-usage` |
| S6 | hyperfine, same pin | path | `src/outlier_detection.rs` |
| S7 | fzf `b1be3a8be1b833ce5b92fbbac11637643d60a046` | commit | `961793c` |
| S8 | fzf, same pin | path | `src/algo/algo.go` |

`scripts/study/tasks.jsonl` carries the question, the answer and the
`provenance` field that names the command whose output the question was written
from: `git log --format='%h%x09%s' -n 120` at the pin for the ripgrep commits,
`-n 100` for hyperfine, `-n 70` for fzf, `git branch -r` for S5, `git ls-files`
for the three paths. Each question is written from the candidate it names
without reusing that candidate's own words, so a description and its answer do
not share a keyword by construction. Whether a literal search finds the answer
anyway is the control arm's business, and the control arm is free to try.

## The runner

`run_cell.py` runs one task, one arm, one repetition:

```
/usr/bin/sandbox-exec -f <profile> claude -p --safe-mode --no-session-persistence \
    --permission-mode bypassPermissions --model haiku \
    --output-format stream-json --verbose --max-budget-usd 1 \
    --tools Bash Read Grep Glob
```

`--safe-mode` turns off this machine's CLAUDE.md, skills, plugins, hooks and MCP
servers, so the agent under test is the model and its built-in tools and nothing
the operator happens to have installed. The prompt goes in on stdin and the
stream-json transcript comes out; `meta.json` holds turns, tool uses, input and
output tokens, wall time, the agent's own inference cost, and every jevify call
with its argv, exit code, request count and question count.

**The control arm is unrestricted.** Both arms get `Bash`, `Read`, `Grep` and
`Glob`, and may run whatever they like: `git log --grep`, `rg`, `find`, `cat`
over a listing. The two prompts differ by one paragraph — with arm: "The
command-line tool jevify is installed at …" plus the output of
`jevify init agents`; control arm: "The tool jevify is not installed on this
machine." The two Seatbelt profiles differ by two lines, the two that say
whether that binary may be executed. Everything else, including the task text
and the answer format, is the same bytes.

**Every jevify call is logged whether the agent reports it or not.** Both arms
are denied exec of `~/.cargo/bin/jevify` and of any `target/*/jevify`; the with
arm is allowed exec of one path, `$JEVSTUDY/bin/jevify`, which the run reaches
through a wrapper that records argv, exit code and the envelope's counters. An
agent cannot reach the binary any other way, so the log is complete by
construction.

## The sandbox, and what holds it

The sandbox wraps the agent process, not each of its commands. Every child
process inherits it, and so does every in-process read by `Read`, `Grep` and
`Glob`. `cd` buys nothing: the profile names paths, not a working directory. The
pilot's escape — an agent that `cd`-ed out of its wrapper — has no counterpart
here, because there is no wrapper to leave.

`canary.py` runs the containment checks through the same profile the agent gets.
On 2026-09-24, with a real run present as a sibling:

| Canary | with arm | control arm |
|:---|:---|:---|
| write into `~/Projects/testify` | denied | denied |
| write into `/tmp` | denied | denied |
| write into `~` | denied | denied |
| read `scripts/study/tasks.jsonl` (the gold) | denied | denied |
| read another run's `answer.txt` | denied | denied |
| read another run's `transcript.jsonl` | denied | denied |
| read `~/.ssh/typesafe-ai-key` | denied | denied |
| exec `~/.cargo/bin/jevify` | denied, 126 | denied, 126 |
| exec `target/release/jevify` | denied, 126 | denied, 126 |
| exec the study's logged copy | allowed | denied, 126 |
| `cd ~` then write | denied | denied |
| `cd` into the repository and read `AGENTS.md` | denied | denied |
| write inside the run's own `work/` | allowed | allowed |
| the network | allowed | allowed |
| `jevify pick` through the logged wrapper | allowed, and the call appears in `jevify.jsonl` | n/a |
| files written outside the run, after all of it | 0 | 0 |

The key is denied in both arms because the study runs jevify keyless. The
pilot's stated limit — the with arm's shell can `cat` the key because jevify
reads it in the same sandbox — does not arise.

## Blind scoring

`blind.py` writes one record per run into `$JEVSTUDY/blind/`:

```json
{"bid": "3185342df1b1", "task": "S1", "answer": "435f59f", "leaked": false, "leak": []}
```

The task, the answer, a leak boolean. No arm, no repetition, no model, no run
id, no timing; `bid` is a truncated sha256 of the run id, so the two arms of a
pair sort apart and the file name says nothing. The unblinding map goes to
`$JEVSTUDY/unblind_map.json`, outside `blind/`.

`score.py` opens `blind/*.json`, `tasks.jsonl` and the pinned clones, and
nothing else — not the map, not a run directory, not a `meta.json`, not a
transcript. It asserts on every record that no arm or run id is present. It
normalises before comparing: a commit answer and the gold are both resolved with
`git rev-parse` in the pinned clone so 7, 8 or 40 characters score alike; a path
loses a leading `./` or repository name; a branch loses `origin/`.

**Refusing a leaked run.** The leak check runs in `blind.py`, because it needs
the transcript and a transcript names the arm; its verdict is a single boolean,
the same kind of fact for either arm, and `score.py` refuses rather than scores
a record carrying it. A run is leaked when the gold appears in the prompt it was
given, when the sentinel string that exists only in the gold file appears
anywhere in its transcript, or when any tool result in its transcript names the
harness directory or the task file. Zero of the twelve runs fired it, so it is
shown firing on three records made by hand, scored with `JEVSTUDY` pointed at a
directory holding only them:

```
{"bid":"aaaa…","task":"S1","status":"refused_leak","correct":null,"why":"gold-file sentinel in transcript"}
{"bid":"bbbb…","task":"S1","status":"scored","correct":0,"normalised":"3fce3b5bb0236da2df6d99672afb8a719642eca7"}
{"bid":"cccc…","task":"S3","status":"scored","correct":1,"normalised":"crates/ignore/src/default_types.rs"}
```

A leaked record is refused rather than scored, a wrong commit hash resolves and
scores 0, and `./ripgrep/crates/ignore/src/default_types.rs` normalises to the
gold and scores 1.

## The twelve validation runs

Two tasks, two arms, three repetitions each. S1 is a commit among 2,287 on the
checked-out branch; S8 is a path among fifteen files in one directory.

| run | correct | turns | tool uses | input tokens | wall s | agent USD | jevify calls |
|:---|:---:|---:|---:|---:|---:|---:|---:|
| S1 with r1 | 1 | 3 | 2 | 33,882 | 11.5 | 0.0104 | 0 |
| S1 with r2 | 1 | 3 | 2 | 33,736 | 10.3 | 0.0142 | 0 |
| S1 with r3 | 1 | 3 | 2 | 34,029 | 12.3 | 0.0155 | 0 |
| S1 without r1 | 1 | 3 | 2 | 31,538 | 11.1 | 0.0129 | — |
| S1 without r2 | 1 | 6 | 5 | 65,712 | 17.8 | 0.0225 | — |
| S1 without r3 | 1 | 3 | 2 | 30,376 | 9.8 | 0.0114 | — |
| S8 with r1 | 1 | 4 | 3 | 70,342 | 15.1 | 0.0572 | 0 |
| S8 with r2 | 1 | 5 | 4 | 102,618 | 17.6 | 0.0618 | 0 |
| S8 with r3 | 1 | 7 | 6 | 107,453 | 21.5 | 0.0400 | 0 |
| S8 without r1 | 1 | 5 | 4 | 82,088 | 17.1 | 0.0600 | — |
| S8 without r2 | 1 | 5 | 4 | 72,995 | 16.5 | 0.0514 | — |
| S8 without r3 | 1 | 4 | 3 | 59,195 | 15.9 | 0.0488 | — |

Per cell, with the interval that is the reason for printing an interval:

| cell | k/n | 95 % Wilson | refused |
|:---|:---:|:---|:---:|
| S1 with | 3/3 | [0.44, 1.00] | 0 |
| S1 without | 3/3 | [0.44, 1.00] | 0 |
| S8 with | 3/3 | [0.44, 1.00] | 0 |
| S8 without | 3/3 | [0.44, 1.00] | 0 |

Arm totals over six runs each: with 25 turns, 19 tool uses, 382,060 input
tokens, 88.3 s, 0.1991 USD; without 26 turns, 20 tool uses, 341,904 input
tokens, 88.2 s, 0.2069 USD. No run wrote a file outside its own directory.

**Twelve runs cannot tell us anything about jevify.** Four cells of three runs
each, on two tasks, with one model. Every interval above spans more than half
the scale, so no two cells separate, and the arm totals differ by less than the
spread between two repetitions of the same cell. What the twelve runs establish
is that the harness produces the numbers it claims to produce, that the blind
path scores without seeing an arm, and that nothing escaped.

**The with arm called jevify zero times in six runs.** That is the pilot's
recorded gap happening again, now in six cells instead of one, and it is the
single most important thing the validation found. `haiku` answered S1 with two
`git log` commands and S8 with a `find` and one `Read`. On both tasks the
control arm did the same work at about the same cost. A study of eight tasks
priced below assumes adoption it has not yet observed; whether the with arm ever
reaches for the tool is itself the first thing the full study measures, and a
full study that comes back with adoption near zero has answered the question
the study was asked, in the negative, at that model.

**What one jevify call costs on each task**, measured outside any agent by
`probe.py`, is a separate fact, and the one the pricing rests on:

| Task | verb | requests | classifications | correct |
|:---|:---|---:|---:|:---:|
| S1 | `pick --from commit` | 25 | 50 | yes |
| S2 | `pick --from commit` | 25 | 50 | yes |
| S3 | `pick --from file` | 4 | 8 | yes |
| S4 | `pick --from commit` | 12 | 24 | yes |
| S5 | `pick --from branch` | 1 | 2 | yes |
| S6 | `pick --from file` | 1 | 2 | yes |
| S7 | `pick --from commit` | 39 | 78 | yes |
| S8 | `pick --from file` | 3 | 6 | yes |

110 requests and 220 classifications for one call on each of the eight tasks, on
a cold cache. Eight of eight name the gold. That is a fact about the verb, not
about an agent, and it does not enter the study's result.

## Faults in this harness

1. **The post-run scan cannot attribute a write.** It is `find -newer` over the
   home tree; on a machine doing anything else it lists the operator's own
   files. Four of the twelve runs list `/private/tmp`, which is that directory's
   own mtime and not a file any run wrote. The scan is a smoke test; the canary,
   run while nothing else moves, is the enforcement evidence. Closing it means
   running the study on an idle machine or attributing writes per process.
2. **The agent may write to `~/.claude`.** The profile allows it because the CLI
   needs its own configuration directory, and that directory is shared between
   runs. Nothing in a task leads an agent there and no run touched it, but the
   hole is real. Closing it means a per-run `CLAUDE_CONFIG_DIR`, which on this
   machine costs the OAuth credentials the runs authenticate with.
3. **A run can list its sibling runs' directory names.** A single `deny
   file-read-data` on the `runs/` subtree stops the agent process from starting
   at all — node resolves its working directory through every parent — so the
   siblings are denied one rule each and the parent directory stays listable.
   Names only: every sibling's contents are denied, which the canary shows.
   A sibling created after the profile is written is not covered, so cells must
   not be run in parallel.
4. **Adoption is not controlled and was zero.** The harness measures what an
   agent does; it cannot make the with arm call the tool, and a prompt that
   pushed it to would not be measuring anything worth knowing. At zero adoption
   the two arms are the same experiment run twice.
5. **One model, and a cheap one.** `haiku` on two easy tasks answers 12 of 12.
   A task set where both arms are perfect measures nothing, and this one may be
   too easy at the sizes chosen.
6. **The transcript's token counts are the harness's, not an independent
   meter.** They come from the CLI's own `usage` object, the same source the
   pilot used, and no second measurement checks them.
7. **`claude -p` can fail before it starts.** Two cells returned exit 1 with
   `An unknown error occurred (Unexpected)` and an empty transcript; the cause
   was a profile rule, found by bisecting the profile. The runner records such a
   cell rather than retrying it, so a silent profile mistake shows up as a run
   with no turns, not as a missing run.

Two faults found by running rather than by reading, both now fixed and both
worth naming because they are the family the pilot's `hunch-3fj` belongs to: a
`deny file-read*` on the study tree stopped `git` walking the parent chain, so
`fill` failed with `lister_failed`, exit 6; and a stale `PWD` outside the
sandbox made every `/bin/sh` print a `getcwd` warning into the agent's context,
spending tokens on the harness instead of the task.

## Pricing the full study: 8 tasks, n = 5 per cell per arm, 80 runs

From the medians of the twelve validation runs and the probe above.

| | with arm | control arm |
|:---|---:|---:|
| median run: wall | 13.7 s | 16.2 s |
| median run: turns | 4 | 4 |
| median run: input tokens | 52,186 | 62,454 |
| median run: agent inference | 0.0277 USD | 0.0356 USD |
| x 40 runs: wall | 9.1 min | 10.8 min |
| x 40 runs: input tokens | 2.09 M | 2.50 M |
| x 40 runs: agent inference | 1.11 USD | 1.42 USD |

- **Wall time** 32 minutes sequential: 20 minutes of runs plus 12 minutes of
  post-run scans at 9 s each. Plus about 10 minutes to clone and pin the corpus
  once.
- **Tokens** 4.6 M input tokens of agent inference, 2.53 USD at the `haiku` list
  price the CLI reports.
- **Keyless classifications** 1,100 and 550 requests as a ceiling — every with
  arm run making one call, per-run caches so nothing is shared. That is 5.5 % of
  the 20,000 a day one IP is allowed as classifications, 2.8 % as requests. At
  the adoption the pilot saw, 5 of 8, about 690 classifications. At the adoption
  this validation saw, zero.

Assumptions, each of which can be wrong: the model is `haiku` and the medians
hold across the six tasks not yet run (S7's `pick` is three times S1's, so the
keyless ceiling is the shakiest of the three); one jevify call per adopted run,
where the pilot saw up to three on one task; the tasks stay as easy as S1 and
S8, so no run hits the 900 s cell timeout or the 1 USD budget; the machine is
otherwise idle, which the scan needs and the wall time assumes; and the
allowance counts classifications rather than requests, which this harness does
not establish.

**At half the budget** the repetitions go before the tasks: 8 tasks at n = 3 per
cell per arm, 48 runs, about 19 minutes and 1.50 USD. Eight tasks at n = 3 still
says something about which tasks an agent reaches for the tool on, which is the
open question; four tasks at n = 5 would buy a tighter interval on a cell nobody
has a reason to care about yet. The probe stays whole either way: it costs one
call per task and it is what prices the backend.
