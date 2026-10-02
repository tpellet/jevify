---
name: jevify
description: Find the line that explains a failure, or the ID you can describe but cannot name. Fill command arguments from real branches, commits, files, PRs, runs or supplied candidates; pick --from returns the handle alone. Filter or label records, branch on a fact, or stage authorized hunks. Use literal tools for known values, counting and arithmetic. Never generate text or make security decisions.
---

# jevify

## The two sides of a command

A command takes arguments and produces output. On the input side, `fill` selects real handles
for arguments and runs the command you write. On the output side, jevify turns a long stream
into a pointer, a subset, or a decision. It selects existing text and never invents values.

Use cheap tools first: `grep`, `jq`, `head`. Skip jevify when a literal search answers the
question, the input is short enough to read, or you already know the required value.

Measured 2026-09-28 on TypeSafe across 34 failed GitHub Actions runs from 30 public repositories:
`why` puts the root-cause line first in 30/34, points at a wrong line in four, and abstains in
none; `grep -iE 'error|fail|panic' | tail -n 5` keeps a gold line in 11/34 and `tail -n 50` in
10/34. A separate `-n 3` invocation finds it in the top three in 31/34. p50 latency is 0.87 s on
a loaded machine, 0.5 s on a quiet one. These are development-set retrieval measurements, not
agent task success. [Cases, limits and baselines](../../../../benchmarks/why-ci.md).

## The verbs

Each situation below has one complete command. Reach for it at the moment you would otherwise
run the listing (`git branch -a`, `git log | grep`, `gh run list`, `ls` then one read per file)
only to choose from it by eye.

| Situation | Command | Result |
|:---|:---|:---|
| About to list branches only to choose one | `jevify fill -- git switch '@{branch:the auth refactor}'` | The real branch, then the command |
| A commit by what it did | `jevify fill -- git show '@{commit:restricted the correction to the primary metrics}'` | The hash, then the command |
| A failed CI run by description | `jevify fill -- gh run view --log-failed '@{ci-run:the failed run on tag v0.7.0}' \| jevify why` | The run's log, then its cause |
| A file or PR by description | `jevify fill -- cat '@{file:parses the marker}'`, `jevify fill -- gh pr view '@{pr:the Windows path fix}'` | A real path or number, then the command |
| A tool can list the needed value | `git log --oneline \| jevify fill -- git revert '@{-:the pricing change}'` | A handle from a supplied record |
| Run the one test you can describe | `cargo test -- --list 2>/dev/null \| sed -n 's/: test$//p' \| jevify fill -- cargo test '@{-:what the test checks}' -- --exact` | That test runs; exit 3 runs nothing (`benchmarks/test-by-behaviour.md`: 43/50 hit, 0 wrong) |
| Want the value without the run | `jevify pick --from branch 'the auth refactor'` | A handle, or abstention |
| An option depends on text you have not read | `jevify fill --context report.md -- gh issue create --label '@{one:bug\|feature\|docs:what kind of report}'` | A caller-written option; `'@{flag:--draft:question}'` for a conditional flag |
| A failed build has more than about 50 lines, or grep finds only the symptom | `cargo test 2>&1 \| jevify why` | A cause with line number and context; read that, not the whole log |
| A claim to check before acting on it | `jevify is 'the failure is in TestSqlUpdate' --context test.log` | Exit 0 when the log establishes it, 1 when it does not; 20/20 on real logs, no false yes (`benchmarks/claim-check.md`) |
| Many records, one question | `gh issue list \| jevify filter 'reports a crash'` | Matching and unsure records; the words need not match |
| Many files, one question | `fd -0 -e rs \| jevify filter -0 --files 'tests backend throttling'` | Paths judged by file content |
| Every record or file needs a bucket | `ls reports/*.md \| jevify filter --files --label bug,feature,docs` | Each record with its label, `?` when unsure; one call, not one read per file |
| One record or file out of many, described rather than named | `git ls-files \| jevify pick --files 'guards downloads against internal addresses'` | A selected input record, or abstention |
| The next step depends on a fact | `cargo test 2>&1 \| jevify is 'every failure is a network timeout' && cargo test` | An exit code, like `test` |
| An unfamiliar task in the long tail of a large PATH | `jevify pick --from tool 'render a terminal demo from a tape file'` | An installed tool; nothing executes |

```sh
cargo build 2>&1 | jevify why
gh issue list | jevify filter 'reports a crash'
fd -0 | jevify filter -0 --files 'a test fixture'
gh issue list | jevify filter --label bug,feature,question | cut -f1 | sort | uniq -c
git ls-files | jevify pick --files 'where retries back off'
git log --oneline | jevify pick -n 3 'the pricing change'
gh pr list --json number,title | jq -c '.[]' | jevify filter 'touches the installer' | jq -r .number
cargo test 2>&1 | jevify is 'every failure is a network timeout' && cargo test
until kubectl get pods | jevify is 'every pod is ready'; do sleep 5; done
jevify is 'asks for a refund' 'mentions an order' --context mail.txt
jevify pick --from tool 'keep my mac awake for an hour'
```

`pick` and `filter` print input records byte for byte; `filter --label a,b,c` prints
`LABEL<TAB>RECORD`, the
record unchanged after the tab, so `cut -f1` counts and `cut -f2-` gives line records back. A
record is a line; `--para` reads blocks between blank lines, and `-0` reads NUL-separated
records. These two split modes are mutually exclusive. `--files` reads paths from stdin and
uses file excerpts as evidence.
`why` takes none of those split or file options: it prints numbered lines with context.
Pipe stderr with `2>&1` because compilers write errors there.

`filter` keeps unsure records: a dropped record can hide the answer. `--strict` drops them.
`filter -v` inverts the statement, as `grep -v`; `-c` prints the kept count. Verbosity is
`--verbose`. A failed request can leave a partial prefix on stdout; check the exit code before
treating the subset as complete.

`why` searches bounded evidence. In machine output, compare `data.considered` with
`data.total`; if much is omitted and the answer is a symptom, narrow to the failing job or
step. A saved full input is a way back, not proof that every line was judged.

### Habits

- One jevify process per question, however many records. Never start one process per record
  in a shell loop, and never read files one by one to classify or find one; use `filter`,
  `filter --files`, `filter --files --label` or `pick --files`.
- `why` answers with the cause and its line number: read that answer, and open the whole log
  only when the answer is a symptom or `considered` is far below `total`. `filter --label`
  takes no statement and at least two distinct labels, none `?` or `NONE`, at most 99 keyless
  or 200 on TypeSafe; it saves nothing. Polling a changing state with `until` is a different
  question on each snapshot.
- Write literal statements: “the customer is about to stop being a customer” avoids the
  ambiguity of “the customer is leaving.” Describe the evidence, not the fix you want.
- Write conditions so yes means act. In human output, `is` with one statement prints nothing
  on stdout: read its exit code. Several statements share one call and print one verdict each.
- `&&` acts only on yes. `until` also repeats on abstention and unavailable responses; inspect
  stderr and stop polling on an outage or quota exhaustion.
- Never put a `pick` command substitution in another command's argument: the shell discards
  its exit code and an abstention becomes an empty argument. Read the selected record and its
  exit code, then write the next command explicitly.
- Under `git bisect run`, map jevify exit 3 to 125 (skip), so uncertainty is not a bad commit.
  Handle operational errors separately; they are not evidence about the commit.
- Scores depend on backend and task. A higher threshold does not repair incomplete evidence,
  and text under judgment can argue with the judge. Do not use jevify as a security gate.

### Other verbs

| Situation | Command | Result |
|:---|:---|:---|
| Requested staging of one topic | `jevify add --dry-run 'the token expiry fix'` | Scores or stages individual hunks |

`jevify add --json --dry-run 'the token expiry fix'` scores without staging; `--yes` stages,
and only when the caller authorizes it (see Permissions and privacy).

## The marker

```sh
jevify fill --dry-run -- git switch '@{branch:the auth refactor}'
jevify fill --dry-run -- git revert '@{commit:made folder moves atomic}'
jevify fill --dry-run -- cat 'src/@{file:parses the marker}'
printf 'retry_backoff\nparse_header\n' | jevify fill --dry-run -- cargo test '@{-:the retry test}'
cargo test -- --list 2>/dev/null | sed -n 's/: test$//p' | jevify fill -- cargo test '@{-:a 429 is retried}' -- --exact
pytest --collect-only -q | sed -n '/::/p' | jevify fill -- pytest '@{-:a 429 is retried}'
printf 'A crash with no reproduction steps.\n' | jevify fill --dry-run -- printf '%s\n' \
  '@{one:bug|feature|docs:what kind of report is this}' '@{flag:--draft:the report lacks steps to reproduce}'
jevify pick --from branch 'the auth refactor'
```

```text
jevify fill -- kubectl logs '@{pod:the payment worker}'
jevify fill -- gh pr view '@{pr:the Windows path fix}'
```

The three families are things that exist, caller-written options (`one`, `flag`), and supplied
records (`-`). The kinds that exist: `branch` (local and remote refs with subject and age),
`commit` (the log, newest first), `file` and `dir` (tracked and untracked paths, narrowed by a
literal prefix ending in `/`), `tool` (the PATH), and the recipes `pr`, `issue`, `ci-run`,
`stash`, `process`, `container`, `pod` (the owning tool's listing, one line per candidate).
`jevify capabilities --json` lists every kind with its exact lister argv, including the user's
own recipes from `kinds.jsonl` under explicit `JEVIFY_CONFIG_DIR`; a recipe is one JSON line with
`kind`, `list` and `field` or `key`, never read from the cwd or platform configuration directory.
A user recipe cannot shadow a shipped kind. `-` uses stdin or
`--candidates FILE`, with `--field N` or `--key KEY` to name a handle inside the evidence. A
list with no kind is a pipe into `'@{-:…}'`, shaped by `sed`, `cut` or `jq` first.
`one` and `flag` judge stdin or `--context FILE`. stdin has one role; supply the other with a file.
File inputs leave stdin for the command; consumed stdin becomes empty for it.

- Put the whole marker argument in single quotes, including prefixes and suffixes. An apostrophe
  is `'\''`. Never use double quotes around a marker whose contents the shell could expand.
- Marker escapes are `\}`, `\:` and `\|`. `one` separates options with `|` before the question's
  `:`. A `flag` is a whole argument: yes keeps it, no removes it, unsure abstains.
- A literal `@{word:` is spelled `@@{word:`. The Python format string `'{user}@{host:>8}'` is an
  unknown kind, exit 2; `'{user}@@{host:>8}'` preserves it literally.
- Use `--dry-run` to look, never `eval`. Omit it only when the underlying command is authorized.
  A marker does not survive a second shell (`ssh`, `make`, `xargs`).
- Never use `"$(jevify pick …)"` as an argument; abstention becomes an empty argument.
- Several markers resolve together against one snapshot. Any abstention means nothing runs.
  Free text stays literal: write titles, messages and new names yourself.

`fill` accepts 3,267 candidates per marker keyless, 13,200 on TypeSafe, and both backends serve
that whole count; `pick` and `pick --from`
accept 9,801 and 20,000. Ordered kinds keep the newest candidates and report coverage; unordered
overflow is `too_many`. `one` accepts 99 options keyless, 200 on TypeSafe. No call needs more
than two rounds of model requests.

## Exit codes and recovery

| Code | Meaning and next step |
|:---|:---|
| 0 | Found, kept records, or all statements yes |
| 1 | `is`: at least one no; `filter`: kept none |
| 2 | Usage: copy the corrected argument or command from the error |
| 3 | Abstention: nothing fits or unsure; read closer or narrow the evidence |
| 4 | Backend unavailable: read the line for the quota or model problem |
| 5 | Authentication: check backend and key configuration |
| 6 | Input error: check the input; for `too_many`, narrow with `grep` or `head` |
| 130 | Declined |

For `fill`, exits 2–6 mean nothing ran; after execution the command owns its exit code, including
2–6. Stderr reports `exec` or `not run:` with the `jevify fill:` prefix; `-q` keeps only
`not run:` lines. A successful dry run exits 0. Machine output requires `--dry-run` and includes
`data.argv` on success and `data.markers[]` for individual results.

An exit-3 abstention has `error: null`: read `data.reason`, the first failed marker in argv
order, and `data.markers[].reason`, every marker's own reason. These are abstention reasons,
not error kinds:

| Reason or error | Recovery |
|:---|:---|
| `too_many` (exit 6) | use a prefix, `grep`, `head` or a narrower pipe |
| `ambiguous` | read the two handles, write one |
| `no_match` | read candidates N of M and narrow or correct the description |
| `unsure_flag` | write the flag or drop the marker |
| `insufficient_evidence` | supply a complete context that fits |
| `lister_failed` (exit 6) | run the named lister yourself; the message carries the tool's own text (a login, a rate limit) |
| `recipe_invalid` (exit 6) | fix the named line of `kinds.jsonl`; it does not parse or names a shipped kind |

The other input error kinds are `stdin_is_tty` and `cannot_run`. The `fill` status line reads
`candidates N[ of M[, newest first]][, omitted K], windows W[, excerpts withheld: E]`; `of M,
newest first` means an ordered listing was cut to its newest part. `fill` refuses
non-Jev answers with exit 4, `api_unavailable`, `answered by <model>, not Jev`; a missing model
name appears as `unknown` in `meta.model` and refuses too. This guard also applies to dry runs.

`filter`, with or without `--label`, exits 3 when every record is unsure, even when it prints those records. `is` exits 0
when all statements hold, 1 when any is no, and 3 otherwise. Oversized `is` input abstains
without a model call. Do not retry unchanged evidence to turn uncertainty into certainty.

`quota_exhausted` (exit 4) means TypeSafe credits or classifier.dev's free budget are spent;
it is never retried. The free budget is $0.50 per IP per UTC day, subject to $100 per day
across everyone and four concurrent requests. A per-request spending limit is
`input_too_large` (exit 6): narrow the request. `health` makes a small uncached classification
and detects exhaustion through these errors; it consumes backend budget.

Use `--json` for one envelope on one line when you need scores or structured errors; leave it off for record pipelines and
silent predicates. The envelope is `{ok, command, version, exit_code, data, meta, error}`;
`error` contains `kind`, `message`, `hint`, and `example`: every error names a corrected
command to inspect before running. Branch on the process exit code or `exit_code`, then inspect
`data`. An abstention's `data.shortlist` lists the nearest candidates with scores, not chosen
answers; `fill` lists it for the first failed marker and `data.markers[].shortlist` per marker.
`jevify capabilities --json` prints the compact installed contract;
[ROBOT_MODE.md](../../../../docs/ROBOT_MODE.md) gives details. The plugin also registers
`jevify mcp`, the same `why`, `is` and `pick` as MCP tools.

## Permissions and privacy

Allow the output verbs (`why`, `pick`, `filter`, `is`) freely. They do not execute the
tool they select. Check the selected tool's help and write its arguments yourself.
Allow `jevify fill --dry-run` freely. Allow `fill` per command prefix
(`jevify fill -- git switch:*`), exactly as the command itself is allowed. jevify is not a
permission system. The description, option context and candidates' evidence leave the machine
with best-effort redaction; `fill` starts the command the caller wrote.

`add` changes the index, never commits. Inspect `--dry-run` first; use `--yes` only when
staging is authorized. It rejects oversized hunks instead of clipping evidence. A
noninteractive call without `--yes` or `--dry-run` exits 2; a person declining the interactive
confirmation exits 130.

Evidence goes to the configured API with best-effort masking. Do not supply secrets.
`why` and `filter` also save the raw input locally, secrets included, and print the saved path
on stderr. A saved input keeps for seven days, as a cached answer does; a later save deletes
the store's own files past that age. `--no-save` disables that raw copy for one call and
`JEVIFY_NO_SAVE=1` for every call; `--no-cache` and `JEVIFY_NO_CACHE` only disable the separate
answer cache, never the raw copy. A failed or disabled save is reported, so do not assume the
full input remains available.

`--files` withholds excerpts of hidden paths and files that look like secrets; stderr reports
`excerpts withheld: N`. Their names still reach the backend. Withholding an excerpt is not a
guarantee that the remaining input contains no sensitive information.

## `why` on every failure: the hook and the GitHub Action

The plugin's `PostToolUseFailure` hook on Bash (`hooks/why-on-fail.sh`) is the one command
`jevify why --hook claude`: it reads the hook payload on stdin and runs `why` on the failed
command's output when it has at least `--min-lines` lines (default 80, `JEVIFY_HOOK_MIN_LINES`).
The line it points at, with context and the saved-output path, arrives next to the error: read
that line first. The hook adds nothing when the command was interrupted, `jevify` is not on
PATH, `why` abstains or fails, or 20 seconds pass, and it never blocks a tool call. It uses the
session's backend configuration and never reads the key; the output goes to the backend as in
any `why` call. It judges the failed result as Claude Code passes it, about 10,000 characters
of head and tail, so a cause in the cut middle needs `cmd 2>&1 | jevify why` on the whole output.
Codex runs the same command as `jevify why --hook codex` on `PostToolUse` for Bash, from
`~/.codex/hooks.json` or the repository's `.codex/hooks.json`.

In GitHub Actions, the repository's `action.yml` installs jevify and writes the pointed line to
the job summary; it writes nothing on abstention and never fails the job. `shell: bash` gives
the step pipefail, so it fails with the build rather than with `tee`. Replace `<tag>` with a
release tag containing the action:

```yaml
- run: cargo test --locked 2>&1 | tee build.log
  shell: bash
- if: failure()
  uses: tpellet/jevify@<tag>
  with: { log: build.log, classes: compile|test|flaky|infra }  # optional: typesafe-api-key
```

`classes` sorts the failure from the lines `why` printed and writes the class under the
heading: 26 of the 34 benchmark runs land in their hand-labelled class, 4 abstain, 4 are wrong
(`benchmarks/why-triage.md`).
