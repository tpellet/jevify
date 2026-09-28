# Call ergonomics: can an agent drive jevify without friction?

A fast, cheap loop metric for the primary user of jevify, a coding agent. Many short
headless agent runs each get one meaning-shaped task and only what an agent normally
has: the `jevify init agents` block, `--help` on demand, a small public repository and
the task's data files. Every call to the tool goes through a logging shim. The headline
is **call quality**: does the first call parse and run, how many calls until an answer,
which errors agents hit and whether the error's hint gets them out. Correctness is
recorded but is secondary.

```
python3 scripts/ergonomics/ergo.py stage --label v0.13.0 --kind jevify --src <jevify binary> --commit <sha>
python3 scripts/ergonomics/ergo.py stage --label thin --kind thin --src scripts/thinjev/jev --commit <sha>
python3 scripts/ergonomics/ergo.py canary --stage v0.13.0
python3 scripts/ergonomics/ergo.py run --phase base --stage v0.13.0 --models haiku:3,sonnet:1 --jobs 8
python3 scripts/ergonomics/report.py base thin
```

## Method

**Runs.** One run is one task, one model, one repetition: `claude -p --model haiku|sonnet
--max-budget-usd 0.25 --tools Bash Read Grep Glob`, up to 8 in parallel, 420 s timeout.
The prompt carries the tool's own agent documentation, the task, one sentence requiring
the tool for the step that chooses or judges ("run it at least once and let what it
returns decide your answer"), and an `ANSWER:` line format. The prompt never shows a
command line for the task.

**Corpus.** `sharkdp/hyperfine` pinned at `f12f3d9`, 66 files, 1018 commits, 28 remote
branches, APFS-cloned per run. Data files (logs, tickets, mails, an inbox of files with
meaningless names and four destination folders) are written per run by
`scripts/ergonomics/fixtures.py`, byte-identical across runs. The two `add` tasks start
from a working tree with two unrelated edits.

**Sandbox.** A Seatbelt profile wraps the agent process itself, so every child and every
built-in file read inherits it. `canary` runs the forbidden and the allowed operations
through the same profile:

```
denied   exit=1    read the harness and its gold answers
denied   exit=1    cd into the harness and read AGENTS.md
denied   exit=1    list ~/.ssh
denied   exit=1    open a file in ~/.ssh
denied   exit=1    read the paired-study tree
denied   exit=128  read the pinned corpus
denied   exit=1    read a sibling run
denied   exit=1    read another stage
denied   exit=126  exec ~/.cargo/bin/jevify
denied   exit=126  exec a target/release/jevify
denied   exit=1    write into ~/Projects
denied   exit=1    write into the home directory
denied   exit=1    write into /tmp
ALLOWED  exit=0    write the CLI's /tmp/claude-XXXX-cwd (allowed)
ALLOWED  exit=0    a zsh here-document (allowed)
denied   exit=1    read the operator's ~/.claude/settings.json
ALLOWED  exit=0    write inside the run's working copy (allowed)
ALLOWED  exit=0    git log in the run's working copy (allowed)
ALLOWED  exit=0    read the run's data (allowed)
ALLOWED  exit=0    exec jevify through the shim (allowed)
ALLOWED  exit=0    the network (allowed: the tool is keyless)
probe exit=2    jevify pick --files one two three < /dev/null
probe exit=0    jevify --version
probe exit=0    printf 'alpha the cat sleeps\nbeta the dog barks\n' | jevify pick 'the …
call log: 4 records for 4 shim invocations (the --help check above is one)
  exit=0 stdin=devnull argv=['--help'] …
  exit=2 stdin=devnull argv=['pick', '--files', 'one', 'two', 'three'] … unexpected argument 'two'
  exit=0 stdin=devnull argv=['--version'] …
  exit=0 stdin=pipe argv=['pick', 'the one about a dog'] …
```

The two allowed `/tmp` writes matter. Claude Code's Bash tool writes
`/tmp/claude-XXXX-cwd` after every command, whatever `TMPDIR` says. Denied, every
command reports exit 1 to the agent even when it succeeded. zsh writes here-documents to
`/tmp/zsh*`. Denied, `cat <<'EOF' | jevify label …` pipes nothing and jevify is blamed
for empty input. The profile allows exactly those two name patterns.

**Call log.** `bin/jevify` in the run is a shim (`calllog.py`). It inherits stdin rather
than reading it, so a call that would hang still hangs. For each call it records argv,
the working directory, the kind of stdin (pipe, file, /dev/null, tty), exit, stdout,
stderr and elapsed time. It writes a start line before the tool runs, so a killed call
is visible. The real binary sits in the stage directory; the agent can reach it only by
reading the shim and calling that path. `report.py` counts such commands ("bypass
cmds"): 0 in every phase. It also counts runs whose transcript shows more tool
invocations than the log. The typed count is an upper bound: it includes the second
branch of `a || b` and here-document text, and a loop logs more calls than it types.

**Metrics.** A *work call* is any call except `--help`, `-h`, `--version`, `help`,
`capabilities`, `robot-docs`, `init`, `health` (and a bare `jev`).

- A work call is **valid** when its exit is not 2 (usage), not 6 (input) and below 128.
  128 and above covers killed calls and 130 (declined at a confirmation no one can
  answer).
- **First call valid** is the rate over runs that made a work call, with a 95 % Wilson
  interval.
- **Calls to first answer** counts calls up to the first exit 0, or exit 1 where 1 is an
  answer (`is` no, `filter` kept none).
- **Hint followed**: after an invalid call, the next call uses a flag or verb the error's
  hint or example named and the failed call lacked. When the hint is the generic "see
  `jevify --help` or `jevify capabilities --json`", the next call must be a help call.
- **Next call valid**: the call after an invalid one is valid.
- **Intended verb**: the run used the verb (and `--from`/`--files`) its task is written
  for.
- **Correct** scores the `ANSWER:` line against the gold: commits resolved with
  `git rev-parse`, paths by suffix, counts with a stated tolerance. For `add`, the score
  is the staged diff itself.

**Runs set apart.** A run where the backend refused a call (exit 4) or the shell could not
write a here-document is reported as set apart, not scored. `report.py --all` keeps them.

**Stages.** `v0.13.0` is the released binary (`jevify 0.13.0`, tag commit `2822f72`,
sha256 `9e2cfa88e867ce1a…`). `thin` is `scripts/thinjev/jev` at `f34ee5a`
(sha256 `b1be1c79dda5a3cc…`). Its documentation in the prompt is its README's usage and
contract paragraphs. Every run's `run.json` carries the stage's sha256.

## Tasks

37 tasks, every verb. The tool is chosen by the agent; the verb column is the one each
task is written for.

| id | verb | task | gold |
|---|---|---|---|
| pc1 | pick-from | Find the commit that lets the user give their own label to the baseline command that the other commands are compared against. | 2166c9f |
| pc2 | pick-from | Find the commit that makes the option for tolerating failing commands accept several specific exit statuses instead of all of them. | b5a6860 |
| pc3 | pick-from | Find the commit that stopped the tests relying on the Unix file-printing utility from running on Windows. | 3bd38f2 |
| pc4 | pick-from | Find the commit that started collecting how much memory the benchmarked commands use. | 6556b2b |
| pf1 | pick-files | Which tracked file flags measurements that sit far from the rest, using a score based on the median? | src/outlier_detection.rs |
| pf2 | pick-files | Which tracked file writes the results as a table in the markup of the Emacs outliner? | src/export/orgmode.rs |
| pf3 | pick-files | Which tracked script draws a box-and-whisker chart comparing several benchmark runs? | scripts/plot_whisker.py |
| pf4 | pick-files | Which tracked file splits a comma-separated list of parameter values while honouring backslash escapes? | src/parameter/tokenize.rs |
| pk1 | pick | Among the remote branches of this repository, which one holds the work on recording how much memory commands use? | track-memory-usage |
| pk2 | pick | Which dependency declared in Cargo.toml draws the progress bars in the terminal? | indicatif |
| fl1 | fill | Show the list of files changed by the commit that repaired how per-command names were applied when benchmarking over a range of parameter values. | 835fc43 |
| fl2 | fill | Print the first 10 lines of the source file that writes the results table in the markup language used by Asciidoctor. | src/export/asciidoc.rs |
| fl3 | fill | List the contents of the directory that holds the platform-specific code measuring CPU and wall-clock time. | src/timer |
| fl4 | fill | Show the full message of the commit that added the first test for the component deciding in which order benchmarks run. | 0705d6c |
| ft1 | filter | Of the 60 most recent commits, how many update the version of a third-party dependency? | 10 (±1) |
| ft2 | filter-files | How many of the Python scripts under scripts/ produce a chart or plot? | 5 |
| ft3 | filter | The job log at jobs.log mixes many kinds of outcome. How many jobs failed because something took too long? | 3 |
| ft4 | filter | The support tickets at tickets.txt are one per line. How many of them ask for money to be returned? | 3 |
| lb1 | label | Sort each support ticket in tickets.txt (one per line) into bug report, feature request or question. How many are bug reports? | 3 (±1) |
| lb2 | label | Classify each of the 40 most recent commit subjects as documentation, dependency update, or code change. How many are dependency updates? | 8 (±1) |
| lb3 | label-files | Classify every Python script under scripts/ as either plotting or statistics by what the file contains. How many are statistics scripts? | 2 |
| lb4 | label | The status log at events.log has one event per line. Classify each as outage, degraded or normal. How many are outages? | 3 |
| is1 | is | Does the customer who wrote mail_cancel.txt want to end their subscription? | yes |
| is2 | is | Does the customer who wrote mail_stay.txt want to end their subscription? | no |
| is3 | is | Does this repository's README say that results can be exported as Markdown? | yes |
| is4 | is | Does the MIT license file in this repository forbid commercial use of the software? | no |
| wy1 | why | The build failed; its output is in build.log. Which source file holds the error that stopped it? | src/cache.rs |
| wy2 | why | The test run in pytest.log collected nothing. Which Python module is missing? | yaml |
| wy3 | why | The CI job whose log is ci.log failed. Which lint rule failed it? | no-unused-vars |
| wy4 | why | The Go test run in gotest.log failed. In which source file (not a test file, not the standard library) did the crash happen? | handler.go |
| rt1 | route | Which installed command-line tool converts a property list file to JSON? | plutil |
| rt2 | route | Which installed command-line tool shows which process is holding a given file open? | lsof |
| rt3 | route | Which installed command-line tool reads a sentence aloud through the speakers? | say |
| st1 | sort | The files in inbox have meaningless names. Which of the existing folders under filing should scan_0042.txt go into? Do not move anything. | recipes |
| st2 | sort | The files in inbox have meaningless names. Which of the existing folders under filing should file_11.txt go into? Do not move anything. | medical |
| ad1 | add | The working tree has uncommitted changes on two unrelated topics. Stage only the change to the installation instructions. Do not commit. | README hunk staged, units.rs hunk not |
| ad2 | add | src/cli.rs has two unrelated uncommitted changes. Stage only the change to the help text of the warm-up option. Do not commit. | warm-up hunk staged, export-json hunk not |

## Results: jevify 0.13.0 and the thin wrapper

Measured 2026-09-28 on keyless classifier.dev. `base` and `base2` are the same stage
(0.13.0) and the same 148 runs, with the harness as described in `base2`. `base` ran
before the here-document rule existed, and its 3 runs that wrote a here-document are set
apart. The classifier.dev daily per-IP allowance ran out during `base2`, and its 28 runs
with an exit 4 are set apart; they are mostly the sonnet runs, which ran last. `thin`
ran before the here-document rule; its 27 here-document runs are set apart. Thin agents
feed options through here-documents far more often, so the thin runs that remain are
the ones that did not, a selection that favours thin.

| phase | model | runs | first call valid [95% CI] | calls to 1st answer (median) | never answered | work calls/run | invalid calls | hint followed | next call valid | intended verb | correct | turns (median) | input tok (median) | USD |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| base | haiku | 108 | 92% [85–96] | 1 | 3% | 1.49 | 9% | 5/14 | 12/14 | 88% | 98% | 4 | 47,945 | 2.63 |
| base | sonnet | 37 | 95% [82–99] | 1 | 0% | 1.59 | 15% | 2/9 | 5/9 | 95% | 97% | 3 | 30,209 | 1.26 |
| base2 | haiku | 103 | 86% [78–92] | 1 | 5% | 1.56 | 16% | 4/21 | 16/21 | 86% | 99% | 4 | 48,268 | 2.70 |
| base2 | sonnet | 14 | 93% [69–99] | 1 | 0% | 1.43 | 15% | 1/3 | 2/3 | 93% | 93% | 2 | 20,195 | 0.40 |
| thin | haiku | 86 | 83% [73–89] | 1 | 0% | 1.66 | 14% | 0/20 | 18/20 | – | 97% | 5.5 | 58,731 | 2.75 |
| thin | sonnet | 30 | 100% [89–100] | 1 | 0% | 3.47 | 0% | 0/0 | 0/0 | – | 97% | 4 | 45,891 | 1.34 |

Exit codes of work calls:

- base / haiku: 0 ×122, 3 ×19, 1 ×6, 130 ×5, 6 ×5, 2 ×4
- base / sonnet: 0 ×42, 6 ×6, 3 ×6, 1 ×2, 2 ×2, 130 ×1
- base2 / haiku: 0 ×118, 6 ×11, 3 ×10, 2 ×9, 1 ×8, 130 ×3, 128 ×2
- base2 / sonnet: 0 ×16, 2 ×3, 3 ×1
- thin / haiku: 0 ×123, 2 ×20
- thin / sonnet: 0 ×104

### By verb

Each cell reads *runs whose first work call was valid / runs that called* · *invalid /
work calls* · *next call used what the hint named / invalid calls followed by
another*. The 0.13.0 columns join `base` and `base2`.

| verb | 0.13.0 haiku | 0.13.0 sonnet | thin haiku | thin sonnet |
|---|---:|---:|---:|---:|
| pick-from | 21/21 · 0/26 · – | 4/4 · 0/4 · – | 6/12 · 8/22 · 0/8 | 4/4 · 0/5 · – |
| pick-files | 23/24 · 1/29 · 0/1 | 8/8 · 0/8 · – | 11/11 · 0/12 · – | 4/4 · 0/4 · – |
| pick | 9/12 · 4/20 · 0/2 | 3/3 · 0/6 · – | 3/5 · 2/7 · 0/2 | 2/2 · 0/2 · – |
| fill | 19/22 · 3/31 · 0/3 | 5/6 · 3/10 · 1/3 | 7/11 · 5/21 · 0/5 | 4/4 · 0/4 · – |
| filter | 18/18 · 0/33 · – | 5/5 · 0/6 · – | 7/7 · 0/7 · – | 2/2 · 0/2 · – |
| filter-files | 6/6 · 0/8 · – | 2/2 · 0/2 · – | 2/2 · 0/2 · – | – |
| label | 15/15 · 2/25 · 0/1 | 5/5 · 0/6 · – | 4/4 · 0/16 · – | 2/2 · 0/52 · – |
| label-files | 6/6 · 0/6 · – | 2/2 · 0/2 · – | 2/2 · 0/7 · – | 1/1 · 0/7 · – |
| is | 23/24 · 1/31 · 0/1 | 4/5 · 2/11 · 0/2 | 6/6 · 0/12 · – | 3/3 · 0/9 · – |
| why | 24/24 · 1/28 · 0/1 | 4/4 · 0/4 · – | 7/10 · 5/21 · 0/5 | 3/3 · 0/11 · – |
| route | 15/15 · 0/16 · – | 3/3 · 4/12 · 0/4 | 7/7 · 0/7 · – | 2/2 · 0/2 · – |
| sort | 0/10 · 16/32 · 2/16 | 1/2 · 2/4 · 1/2 | 5/5 · 0/5 · – | 2/2 · 0/5 · – |
| add | 7/12 · 11/37 · 7/10 | 2/2 · 1/4 · 1/1 | 4/4 · 0/4 · – | 1/1 · 0/1 · – |
| **all** | 186/209 · 39/322 · 9/35 | 48/51 · 12/79 · 3/12 | 71/86 · 20/143 · 0/20 | 30/30 · 0/104 · – |

### What the numbers say

- **Seven verbs have almost no first-call friction for haiku:** pick `--from`, pick
  `--files`, filter, label, is, why and route. The agents block is enough to write the
  call on the first try: 151 of 153 haiku runs on those verbs have a valid first call.
- **sort is the worst verb:** 0 of 10 haiku runs make a valid first call, and half of all
  sort calls are invalid. **add is second:** 11 of 37 haiku calls in add tasks are invalid,
  8 of them a confirmation that no one can answer.
- **Two patterns do not appear:** an unquoted multi-word description, and `--files` with
  nothing on stdin.
  - One `pick` split a path off from its description.
  - Every one of the 38 `--files` calls in `base` has a pipe on stdin.
  - No call hangs waiting on a terminal.
- **Hints rarely steer the next call.** After an invalid call, the next call uses what the
  hint named in 12 of 47 cases across both models. For exit 2 the hint is almost always
  the generic "see `jevify --help` or `jevify capabilities --json`". Agents re-guess
  instead: 35 of 47 next calls are valid anyway. add's hint is the one specific hint, "re-run
  with --yes", and add runs follow their hints 8 of 11 times.
- **thin versus jevify.** The thin wrapper's single call shape is simple, and sonnet never
  gets it wrong. Its failures are about what to feed it:
  - `pick --from commit` has no thin counterpart. Haiku either pipes all 1018 commits and
    hits the 100-option cap, or pipes nothing: 8 of 22 calls are invalid.
  - For labelling, the thin wrapper takes one call per record: 52 calls in one sonnet
    `label` run, and 240 in one haiku run set apart above. jevify's `label` is 1 to 2
    calls.
  - `sort`, `add`, `route` and `fill` have no thin verb. The agent does the listing and
    the git work itself, and uses `jev` only to rank what it listed.
  - Correctness is the same within noise: 97–98 % both ways.
  - Thin runs cost more turns (median 5.5 against 4 for haiku) and more input tokens
    (58.7 k against 47.9 k).

### Friction on 0.13.0, with argv

`$RUN` is the run directory. Counts are over the scored runs of `base` and `base2`.

| n | pattern | example argv | what the tool answers |
|---:|---|---|---|
| 9 | sort given a **file** instead of a directory (3 more file calls fail on the destination, next row but one) | `sort --json $RUN/data/inbox/scan_0042.txt`; with `--into`: `sort --json …/scan_0042.txt --into $RUN/data/filing` | exit 6 `input`: "no folders under …/scan_0042.txt to sort into", hint "check the input path and encoding"; with `--into`: "Not a directory" |
| 9 | add without `--yes` in a shell with no terminal | `add 'the installation instructions'`, `add --json 'the help text change for the warm-up option'` | exit 130 "declined", hint "re-run with --yes" |
| 4 | sort on the destination root, which holds only folders | `sort --json $RUN/data/filing` | exit 6 `empty_input` "no files to sort", hint "pipe text into jevify", example `ls \| jevify pick "what I paid a streaming service"` |
| 5 | sort destination guessed as a flag or a second path, or left out | `sort --json …/scan_0042.txt --to $RUN/data/filing/`, `--dest`, `--destinations invoices,medical,…`, a second positional; `sort --json $RUN/data/inbox` | exit 2, generic hint; without a destination, exit 6 "no folders under …/inbox". `--into` is not in the agents block |
| 5 | `--from -` for piped candidates, which the agents block lists as a kind | `pick --from - 'draws progress bars in the terminal'` | exit 2 "stdin is the default source; omit --from -", generic hint |
| 2 | a bare `-` as a positional | `pick - 'warmup help text'` | exit 2 unexpected argument '-', generic hint |
| 7 | `is --context` with a wrong path, with `-`, or with text | `is 'forbids commercial use of the software' --context LICENSE` (the file is LICENSE-MIT); `is '…' --context -`; `is '…' --context "$(man lsof \| head -30)"` | exit 6 "No such file or directory", hint "check the input path and encoding" |
| 2 | `fill` resolves a remote-only branch to a name git cannot show | `fill -- git show '@{branch:records how much memory commands use}'` | fill prints `track-memory-usage`; git exits 128 on the missing local branch (`origin/` prefix) |
| 2 | `--from` together with `--files` | `pick --from file --files 'writes the results table in asciidoctor markup'` | exit 2 "cannot be used with '--files'" |
| 2 | a flag from another verb, or one no verb has | `label '…' --strict` (a filter flag), `pick --dirs '…'` | exit 2, generic hint |
| 1 | `is` given file names as extra statements | `is 'flags measurements … based on the median' modified_zscores src/outlier_detection.rs` | exit 6 "no input: stdin was empty" |
| 1 | a fill marker kind used as a free question | `fill --dry-run -- lsof '@{flag:it should show which process holds a given file open}'` | exit 2 "expected options or flag followed by ':question'" |
| 1 | `fill` with no marker | `fill -- git ls-files src/timer` | exit 2 "no markers" |

## Keyless allowance

classifier.dev allows 20,000 classifications a day per IP. Three phases of 148 runs,
together with the other jevify work on the same machine that day, exhausted it. From then
on every call answers exit 4, `HTTP 402 … request_spending_limit`. The largest consumers:

- `pick --from commit` over 1018 commits: 11 windows per call.
- thin runs that call `jev` once per record: up to 240 calls in one run.

One phase is the most to run keyless per day on a shared IP. `report.py` sets apart
every run with an exit 4, so a phase that runs into the limit reports only its clean
runs.

## Cost

Agent inference, as reported by `claude -p`: base 4.03 USD, base2 5.10, thin 5.44,
pilots 0.23, for 14.80 USD in total. Median wall time per run is 15–20 s at 8 in
parallel; a phase of 148 runs takes about 7 minutes.
