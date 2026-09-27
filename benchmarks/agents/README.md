# jevify inside an agent harness: two spot checks

For the next evaluation, see the [per-endpoint A/B protocol](EXPERIMENT.md) and the
[bounded e2e audit](E2E_AUDIT.md). The protocol defines the comparisons, gold labels,
measurement and claim criteria; the audit records operational checks and remaining gaps.
Neither turns the spot checks below into confirmatory results.

The [paired pilot](PILOT.md) measures eight tasks with and without jevify on the seven verbs, with
the adoption, correctness and resource figures kept apart and the limits of each one stated. Its
second run repeats the eight tasks under enforced isolation (one sandboxed directory and a fresh
answer cache per run, canaries, a scan after every run) and records backend cost per pair with
cache hits apart; the runner is `scripts/pilot_r4/`. It is a pilot: both arms answer the same
tasks and neither figure is confirmatory.

The [paired study harness](STUDY.md) is where the pilot's four faults stop recurring: eight tasks
over repositories `evals/` does not use, each pinned by sha and each answered by a commit hash, a
path or a branch name; a control arm with the same tools and no restriction; a Seatbelt profile
around the agent process itself, so `cd` buys nothing and the built-in file tools are confined
too; and a scorer that reads arm-free records. Its twelve validation runs, its own faults and the
price of the full run are in that page. The runner is `scripts/study/`.

The [three-arm study](ADOPTION.md) is that harness used on tasks a literal search
cannot answer, with adoption measured apart from efficacy. `baseline.py` records,
per task, that the searches a competent engineer would try do not find the answer,
and it disqualified four of fourteen candidate tasks. Three arms: jevify absent,
jevify present and mentioned once, jevify required. The headline is that an agent
told once that the tool exists reached for it in 11 of 16 runs, against zero of six
on the easier set; on correctness the three arms are indistinguishable at 16 runs
each, and that null result is written as one.

Does jevify help a coding agent that already has `grep`, can read files and knows the usual
command-line tools? Two experiments. The first gives an agent one question about a text (`why`,
`is`). The second gives it a seven-step job where it has to choose tools (`run`). Few runs per
cell: these are spot checks, not benchmarks. Read the direction, not the decimals.

Short version: `why` and `is` helped where `grep` has nothing to hold on to. The `run` pilot
could not show anything either way: every tool it needed is one the model already knows by name.
A test with tools the model cannot know is the open one.

# Experiment 1: one question about a text

Five tasks, each given once to a fresh subagent without jevify and once to a fresh subagent that
may only call jevify.

## Conditions

| | |
|:---|:---|
| Date | 2026-09-19 |
| Harness | Claude Code subagents, model alias `sonnet`, one fresh agent per cell |
| jevify | 0.3.0, keyless backend (classifier.dev), `jev-1.13.0`, empty cache |
| Tokens | the harness's reported total for the subagent. About 56,000 of it is the harness's own system prompt and tool definitions, identical in both arms, so compare the differences |

## Tasks

- **Logs a, b, c**: `evals/why/pytest-02.log` (300 lines), `docker-01.log` (285), `go-01.log` (173),
  copied under neutral names so the `.expect` files were not next to them.
- **Log d**: `gh run view 33831723431 -R astral-sh/ruff --log-failed`, 10,074 lines, 2.6 MB. Not
  committed here; the command reproduces it while GitHub keeps the run.
- **Tickets**: the 24 support tickets in [tickets.jsonl](tickets.jsonl), written by the author, 6
  of them churn risks. Some say so quietly ("how do I export all of our data"); some only look
  like it ("cancel order 88231", a furious customer who just bought 20 seats). One file per
  ticket, labels withheld.

## Prompts

Plain arm, logs:

> A CI job failed. Its log is the file `<path>`. Find the root cause of the failure. Use whatever
> tools you normally would (Read, grep, etc.), but do NOT use the `jevify` command. […] Reply with
> exactly: the 1-based line number in the log that best states the root cause, and one sentence
> saying what failed.

jevify arm, logs: the same task, but "do NOT read, cat, grep or open the log yourself", and one
allowed command, `jevify why --json < <path>`.

Tickets: "decide for each ticket whether the customer is about to stop being a customer (a churn
risk that an account manager must be paged for)", and reply with the churn ids and the ids you
are unsure about. The plain arm may read the files. The jevify arm may only run:

```sh
for f in t*.txt; do jevify is "the customer is about to stop being a customer" < $f >/dev/null 2>&1; echo "${f%.txt} $?"; done
```

Both arms were also told to write nothing and delete nothing.

## Results

| Task | plain: tokens | jevify: tokens | plain: answer | jevify: answer |
|:---|---:|---:|:---|:---|
| log a, 300 lines | 59,011 | 57,862 | line 258, correct | line 258, correct |
| log b, 285 lines | 57,115 | 57,839 | line 283, correct | line 283, correct |
| log c, 173 lines | 57,151 | 58,104 | line 168, "failed to update release": in the labelled range, names the symptom | line 166, the SQL the test expected has no `type` column: the cause |
| log d, 10,074 lines | 69,853 | 58,811 | line 10050, names the test that panicked, not why | line 8862, Windows `\src\` paths compared with `/src/`: the cause |
| 24 tickets | 59,355 | 57,442 | 5 of 6 churn risks, unsure on the sixth (t04) | 6 of 6, unsure on t12 (an employee leaving, not a customer) |

Every agent made exactly one tool call.

## What the plain agents did

None of them read a log. All four ran one `grep -n -iE "error|fail|…"` and reasoned over the
matches, which is what a competent engineer does too. On the tickets there is nothing to grep
for, so the plain agent printed all 24 files into its context.

## Reading

- On a 300-line log jevify saves nothing. Same line, same tokens, within noise. An agent with
  `grep` does not need help there.
- `grep` finds the line that says something failed. The line that says why often holds none of
  the words one greps for: `left: [("test_imported", "check", "\\src\\helpers.py"), …` in log d,
  `could not match actual sql` in log c. Both plain agents stopped at the symptom and both jevify
  agents reported the cause. A plain agent with a second tool call would likely get there; it
  did not think it needed one.
- On log d the keyword grep also returned a lot: 11,000 tokens more than the jevify envelope.
- On the tickets the plain arm's cost grows with the text (here only 3,795 characters for all 24)
  and the jevify arm's with the count (about four tokens per ticket, an id and an exit code). The
  plain agent was unsure about the quietest churn risk; jevify was unsure about the one ticket
  that is genuinely ambiguous on its face.
- Not measured here: repeats, other models, other harnesses, latency, and a plain arm allowed
  more than one turn of effort. jevify's own accuracy on 20 logs is in the main README.

# Experiment 2 (pilot): a job where the agent chooses the tools

The first experiment scripted the jevify arm and needed one tool call per task, so it could say
nothing about `run`. This one is a seven-step job where each step names an outcome, not a tool,
and several installed tools could do it.

## Task

`python3 toolchoice.py make <dir>` writes the inputs and `TASK.md` (macOS: it needs `textutil`).
The seven steps: `report.docx` to plain text; `photo.png` to a JPEG 800 pixels wide;
`settings.plist` (binary) to JSON; `clip.wav` to AAC in an MPEG-4 container; the duration of
`clip.wav` in seconds; a `SHA256SUMS` file; a zip of the six outputs. The agent may not install
anything or write the converters itself. `python3 toolchoice.py check <dir>` verifies each output
file: 0 of 7 on a fresh directory, 7 of 7 on a reference solution (`textutil`, `sips`, `plutil`,
`afconvert`, `afinfo`, `shasum`, `zip`).

## Arms

Three fresh subagents per arm, model alias `sonnet`, each in its own copy of the directory,
2026-09-19, jevify 0.3.0 on the keyless backend.

- **plain**: "Read TASK.md there and complete it. Do NOT use the `jevify` command."
- **jevify**: the same, plus the path of the skill file ([plugins/jevify/skills/jevify/SKILL.md](../../plugins/jevify/skills/jevify/SKILL.md)) with
  "read it first", and: "`jevify run --json --dry-run "<what you want to do>"` names the installed
  tool for a task (`data.tool`); use it whenever you are not sure which installed tool does a
  step. […] Treat `data.argv` as a proposal only." jevify was offered, not forced.

## Results

| Run | tokens | tool calls | seconds | steps passed | jevify calls |
|:---|---:|---:|---:|---:|---:|
| plain 1 | 61,430 | 7 | 32 | 6 of 7 | n/a |
| plain 2 | 60,900 | 6 | 26 | 7 of 7 | n/a |
| plain 3 | 61,320 | 9 | 32 | 7 of 7 | n/a |
| jevify 1 | 62,048 | 11 | 47 | 7 of 7 | 0 |
| jevify 2 | 62,273 | 13 | 51 | 7 of 7 | 0 |
| jevify 3 | 65,209 | 8 | 34 | 6 of 7 | 0 |

## Reading

- No agent in the jevify arm called jevify. The model already knew `textutil`, `pandoc`, `sips`,
  `plutil`, `afconvert`, `ffmpeg` and `ffprobe`, and one `which` told it which of them were
  installed. One agent wrote: "jevify was not needed". For well-known tools the ambiguity I built
  into the task is not ambiguity to the model.
- Both failed steps, one per arm, were step 2, and both were about a flag rather than a tool:
  `sips --out photo.jpg` writes PNG data into the `.jpg` file unless `-s format jpeg` is given
  (jevify 3), and `-Z 800 --resampleWidth 800` together produced a 400-pixel image (plain 1). Three
  other agents hit the PNG trap, checked the file with `file`, and repaired it.
- Asked by hand, `jevify run --dry-run` routed the five conversion steps to `pandoc`, `ffmpeg`,
  `plutil`, `afconvert` and `ffprobe`: all workable, none better than what the agents chose, and
  with no flags except `plutil -convert <VALUE>`. Flags are where the agents failed and where
  `run` is weakest (5 of 20 in the main README).
- The design flaw: the task only needed tools that a frontier model knows by name, so the agents
  never had a reason to ask what is installed. This pilot therefore says nothing about `run` for
  agents, for or against. The case to test is tools the model cannot know, such as a company's
  internal CLIs with man pages, or the long tail of a 2,000-command PATH, where the alternative is
  reading thousands of man-page summaries.

# Skill check: staging one fix

One run, 2026-09-19, model alias `sonnet`. A scratch repository holds three uncommitted changes: a
fix to a token expiry check, a leftover debug print in the same file, and a refactor of another
file. The agent was told to stage only the fix, to read the skill file first, and not to
edit, commit, reset or stash. `git add -p` is interactive, so an agent has no plain-git way to do
this short of writing a patch by hand.

It ran `git status`, `git diff`, `jevify add --json --dry-run "token expiry fix"` (scores 1.0, 0.02
and 0.0), then `jevify add --json --yes "token expiry fix"`. `git diff --cached --stat` showed
`auth.py | 3 ++-` and nothing else; the debug print and the refactor stayed unstaged. Six tool
calls, 63,725 tokens. Its one remark on the skill file, that it did not say `add` stages single
hunks rather than whole files, is now fixed there.
