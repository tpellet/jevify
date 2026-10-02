# jevify

Point at the line that explains a failed build, test or CI run, and put the test, branch,
commit or PR you can describe but cannot name into the command you were about to run.

[![CI](https://github.com/tpellet/jevify/actions/workflows/ci.yml/badge.svg)](https://github.com/tpellet/jevify/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/jevify)](https://crates.io/crates/jevify)

```sh
gh run view <id> --log-failed | jevify why
```

**Root cause first in 30 of 34 failed GitHub Actions runs, where `grep | tail` finds it in 11.**
TypeSafe backend, 2026-09-28, 34 runs from 30 public repositories; `why` points at a wrong line
in the other four and never abstains on these, so a pointer is something to check.
[Cases, method and baselines](benchmarks/why-ci.md).

On the repository's failed-build fixture, 1,812 lines with one error under 300 warnings, on the
keyless backend (`windows 13` is the keyless window count; TypeSafe uses larger windows):

```console
$ jevify why < docs/demo/build.log
jevify why: full output: ~/Library/Caches/jevify/outputs/1a419395094f905c.log
jevify why: 1812 lines, candidates 1212, windows 13
      1 │    Compiling buildfail v0.1.0 (benchmarks/fixtures/demo/buildfail)
>     2 │ error[E0425]: cannot find value `conifg` in this scope
      3 │    --> src/main.rs:306:20
      4 │     |
      5 │ 306 |     println!("{}", conifg);
```

For a local build, pipe stderr too: `cargo test 2>&1 | jevify why`. The full log stays on disk
for seven days; `--no-save` skips that copy.

## Install

On macOS or Linux, no Rust needed:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tpellet/jevify/releases/latest/download/jevify-installer.sh | sh
```

Or with Rust 1.87 or later: `cargo install jevify --locked`. No account needed to try it
([free tier](#free-tier-and-keys)).

## Run the test you mean

Write the command; where an ID goes, describe it in `@{…}`; jevify puts in the real ID, or runs
nothing. The test runner's own listing supplies the names, TypeSafe backend:

```console
$ cargo test -- --list 2>/dev/null | sed -n 's/: test$//p' | jevify fill --dry-run -- cargo test '@{-:a 429 response is retried and the answer is cached}' -- --exact
jevify fill: - a_429_is_retried_and_the_answer_is_cached 1.00 (next 0.00, none 0.00); candidates 207, windows 2; model jev-1.13.0
jevify fill: would run 'cargo' 'test' 'a_429_is_retried_and_the_answer_is_cached' '--' '--exact'
'cargo' 'test' 'a_429_is_retried_and_the_answer_is_cached' '--' '--exact'
```

Omit `--dry-run` and that one test runs. On 50 behaviours written against two repositories'
test listings, `fill` runs the right test for 43 of the 45 that have one, abstains on the other
two and on all 5 that have none, and never runs a wrong one
([gold set and pytest, go recipes](benchmarks/test-by-behaviour.md)). The same marker takes
things a tool can list:

```sh
jevify fill -- git switch '@{branch:the auth refactor}'
jevify fill -- git show '@{commit:fixes retry backoff}'
jevify fill -- gh pr checkout '@{pr:keeps the grid aligned when a tab follows a multibyte character}'
```

The branch and commit lines are placeholders for your own repository. The PR line, in a
checkout of [sharkdp/bat](https://github.com/sharkdp/bat), resolves to `gh pr checkout 4018`
among 1,000 open and closed PRs (TypeSafe, 2026-09-28). On 20 such requests across three
repositories, 17 come out right (16 handles and one correct refusal), 2 abstain with an answer
available, and none resolves to a wrong handle ([sample](benchmarks/fill-sample.md)).

Keep the whole marker argument in single quotes. Inspect `--dry-run`; never `eval` it. If
nothing fits or two candidates are too close, `fill` exits **3 and nothing runs**, with the
nearest candidates in `data.shortlist`. Several markers resolve together; any failure stops them
all. [Kinds and supplied lists](docs/guide/kinds.md) cover files, PRs, CI runs, pods and your
own recipes.

## In your agent

### Claude Code

Install the CLI above, then in Claude Code:

```text
/plugin marketplace add tpellet/jevify
/plugin install jevify@jevify
# Restart Claude Code to load the plugin.
```

The plugin's `PostToolUseFailure` hook on Bash is the one command `jevify why --hook claude`:
it reads the hook payload, and when the failed output has at least 80 lines it supplies the
pointed line, with context and the saved-output path, beside the error. It adds nothing on a
short output, an abstention or a timeout, and never blocks the tool call. Claude Code can
truncate the failed output; pipe the full log to `why` when the cause is in the missing middle.
The plugin also registers the [MCP server](#mcp-server) and the
[skill](plugins/jevify/skills/jevify/SKILL.md).

### Codex

Codex has `PostToolUse` and the same hook shape. In `~/.codex/hooks.json`, or
`.codex/hooks.json` in the repository:

```json
{"hooks": {"PostToolUse": [{"matcher": "Bash",
  "hooks": [{"type": "command", "command": "jevify why --hook codex", "timeout": 30}]}]}}
```

`--hook codex` prints nothing when the command succeeded. The skill works in Codex when copied
into `~/.agents/skills/`.

### MCP server

`jevify mcp` serves `why`, `is` and `pick` as MCP tools over stdio; an abstention is a plain
result with the nearest candidates in `data.shortlist`, never an error.

```sh
claude mcp add jevify -- jevify mcp
```

Codex, in `~/.codex/config.toml`:

```toml
[mcp_servers.jevify]
command = "jevify"
args = ["mcp"]

[mcp_servers.jevify.env]
TYPESAFE_API_KEY_FILE = "/path/to/key"
```

Claude Desktop takes a `.mcpb` bundle: `mcpb pack` on [packaging/mcpb](packaging/mcpb) with the
`jevify` binary at `server/jevify`; its one setting is the key file.
[The tools and their results](docs/ROBOT_MODE.md#the-mcp-server).

### GitHub Actions

Save the failing step's output with `2>&1 | tee build.log` and `shell: bash`, then run the
action after a failure, replacing `<tag>` with a release tag containing it:

```yaml
- uses: tpellet/jevify@<tag>
  if: failure()
  with:
    log: build.log
    typesafe-api-key: ${{ secrets.TYPESAFE_API_KEY }}
    classes: compile|test|flaky|infra
```

The action installs jevify and writes the cause to the job summary; abstention leaves the
summary alone. `classes` sorts the failure from the lines `why` printed: on the 34 benchmark
runs, 26 land in the hand-labelled class, 4 abstain and 4 are wrong
([triage](benchmarks/why-triage.md)).

## Check a claim

`is` answers whether a text establishes a statement, as an exit code for `&&` or `if`:

```sh
jevify is 'the failure is in TestSqlUpdate' --context evals/why/go-01.log && echo established
```

On 20 claims an agent writes after a run, against 11 real logs, `is` answers 20 of 20 with no
false yes on a claim the log contradicts or leaves open (TypeSafe, 2026-10-01,
[claim check](benchmarks/claim-check.md)). Yes means the log establishes the claim; no means
act as if it did not.

## Other verbs

| Need | Command |
|:---|:---|
| One record by description | `jevify pick 'what I paid a streaming service' < lines` |
| The handle alone, no command | `jevify pick --from branch 'the auth refactor'`; also `commit`, `file`, `pr`, `ci-run`, `tool` |
| Records where a statement holds | `jevify filter 'reports a crash' < lines`; `--strict` drops unsure records |
| A bucket for each record | `jevify filter --label bug,feature,question < lines` prints `LABEL<TAB>RECORD` |
| Several facts at once | `jevify is 'asks for a refund' 'mentions an order' --context mail.txt` |
| Stage one topic's hunks | `jevify add --dry-run 'the token expiry fix'`; `--yes` stages |

`pick` and `filter` preserve record bytes and input order. `-0` reads NUL-separated records;
`--para` reads paragraphs; `--files` judges a list of paths by their content.
[Every verb](docs/guide/verbs.md).

## Exit codes and errors

| Code | Meaning |
|---:|:---|
| 0 | yes, found, done |
| 1 | no (`is`), nothing kept (`filter`) |
| 2 | usage error |
| 3 | nothing fits, ambiguous, or unsure; `data.shortlist` lists the nearest candidates |
| 4 | backend unavailable or quota exhausted |
| 5 | TypeSafe key missing or rejected |
| 6 | empty, oversized or unreadable input |
| 130 | declined at the `add` confirmation |

After `fill` starts the command, its exit code belongs to that command;
`JEVIFY_STATUS_FILE=PATH` records whether anything ran. Every error prints a hint and a command
to run next, in the human output and as `error.hint` and `error.example` in `--json`:

```text
$ jevify why < /dev/null
jevify why: error: no input: stdin was empty
  hint: why reads the failing command's output on stdin; compilers write errors to stderr
  try:  cargo test 2>&1 | jevify why
```

## Latency

Uncached, on TypeSafe, from a typical macOS dev machine on a home connection, p50 wall time per
call (2026-10-01, [measurements](benchmarks/latency.md)): `why` on the 1,812-line fixture 0.51 s,
`is` 0.23 s, `fill` on a supplied list 0.24 s, `pick --files` over a repository 0.48 s; the
slowest of 330 runs took 0.81 s. On the 34 CI logs, the median `why` call is 0.87 s on a loaded
machine. A call is one or two rounds of requests; a repeated question answers from the cache.

## Free tier and keys

Without a key, jevify asks [classifier.dev](https://classifier.dev), which serves Jev free and
without an account: $0.50 per IP per UTC day, shared by everyone behind that IP, subject to
$100 per day across everyone and four concurrent requests. It is for trying jevify; CI and
teams need a TypeSafe key from <https://console.typesafe.ai/keys>:

```sh
export TYPESAFE_API_KEY_FILE=/path/to/key
```

A spent budget is `quota_exhausted` (exit 4), not an invitation to retry. `jevify health`
checks the backend. The two backends give the same verbs and exit codes; their scores are not on
the same scale.

## For agents

`jevify init agents` prints a short block for `AGENTS.md`. `jevify capabilities --json` lists
commands, kinds and limits. `--json` prints one line containing
`{ok, command, version, exit_code, data, meta, error}`; branch on `exit_code`.
Use `fill --dry-run --json` for a resolved argv, and authorize execution as you would the
underlying command. [Agent contract](docs/ROBOT_MODE.md).

## Privacy and license

Descriptions and evidence go to TypeSafe or classifier.dev with best-effort redaction.
`why` and `filter` save raw input locally for seven days, secrets included; `JEVIFY_NO_SAVE=1`
disables it. [Privacy](PRIVACY.md) · [Guide](docs/guide/README.md) · [Changelog](CHANGELOG.md).

Powered by Jev from [TypeSafe AI](https://typesafe.ai). [MIT](LICENSE).
