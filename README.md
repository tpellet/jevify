# jevify

jevify is a command-line tool for finding errors in long logs and finding existing tests,
commits and PRs by description.

Give it the output of a failed command, and it points at a diagnostic with the surrounding
lines. Describe the test you want to run, and it finds the test's name. You can use it in a
shell pipeline or give it to a coding agent.

You choose the command. jevify chooses among existing lines and identifiers, reports
confidence scores, and can answer that nothing fits.

[![CI](https://github.com/tpellet/jevify/actions/workflows/ci.yml/badge.svg)](https://github.com/tpellet/jevify/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/jevify)](https://crates.io/crates/jevify)

[Guide](docs/guide/README.md) · [Agent interface](docs/ROBOT_MODE.md) · [Privacy](PRIVACY.md)

## Find the error in a failed command

The useful error can be thousands of lines above the final “build failed”. Pipe the full
output to `why`:

```sh
gh run view <id> --log-failed | jevify why
```

In this repository's build fixture, one error is buried under 300 warnings in 1,812 lines:

```console
$ jevify why < docs/demo/build.log
      1 │    Compiling buildfail v0.1.0 (benchmarks/fixtures/demo/buildfail)
>     2 │ error[E0425]: cannot find value `conifg` in this scope
      3 │    --> src/main.rs:306:20
      4 │     |
      5 │ 306 |     println!("{}", conifg);
```

For a local test run, include stderr: `cargo test 2>&1 | jevify why`. The full log stays on
disk for seven days, and jevify prints its path on stderr. `--no-save` skips that copy.

On 34 failed GitHub Actions runs from 30 public repositories, `why` selected the labelled
diagnostic first in **30 runs**. A `grep | tail -n 5` baseline kept a diagnostic somewhere in
its five lines in **11 runs** (TypeSafe, 2026-09-28). `why` selected a wrong line in the other
four and abstained in none. Read the selected lines and their context before deciding what
to fix. [Cases, method and baselines](benchmarks/why-ci.md).

## Install

Install on macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tpellet/jevify/releases/latest/download/jevify-installer.sh | sh
```

Or use Rust 1.87 or later: `cargo install jevify --locked`. You can try it without an account
using the [free tier](#free-tier-and-keys).

## Run the test you mean

You remember what a test checks, but not its name. List the tests and put the description
where the name belongs, inside `@{…}`. `fill --dry-run` shows the command with the selected
name. In this repository, using TypeSafe:

```console
$ cargo test -- --list 2>/dev/null | sed -n 's/: test$//p' | jevify fill --dry-run -- cargo test '@{-:a 429 response is retried and the answer is cached}' -- --exact
'cargo' 'test' 'a_429_is_retried_and_the_answer_is_cached' '--' '--exact'
```

Omit `--dry-run` to run that test. On 50 descriptions checked against two repositories'
test listings, `fill --dry-run` selected the expected test for **43 of the 45** that had one.
It abstained on the other two and on all five descriptions with no matching test, and selected
no wrong test (TypeSafe, 2026-10-02). [Gold set, pytest and Go recipes](benchmarks/test-by-behaviour.md).

## Find a branch, commit or PR by description

Use the same marker in a command that needs a branch name, commit hash or PR number:

```sh
jevify fill -- git switch '@{branch:the auth refactor}'
jevify fill -- git show '@{commit:fixes retry backoff}'
jevify fill -- gh pr checkout '@{pr:keeps the grid aligned when a tab follows a multibyte character}'
```

Use descriptions that fit your repository. The PR example, in a checkout of
[sharkdp/bat](https://github.com/sharkdp/bat), resolves to `gh pr checkout 4018` among 1,000
open and closed PRs (TypeSafe, 2026-09-28). More [selection cases](benchmarks/fill-sample.md)
cover commits, branches, files and supplied CI runs across three repositories.

Keep each marker argument in single quotes. Use `--dry-run` to inspect the selected identifier;
`fill` executes the command directly, without a shell. If nothing fits or the choice is
ambiguous, it exits **3 and runs nothing**. The nearest candidates are in `data.shortlist`.
When a command has several markers, all must resolve before it runs.

[Kinds and supplied lists](docs/guide/kinds.md) cover files, CI runs, pods and your own
recipes. File selection uses names and content excerpts; it can confidently choose the wrong
file. Check the path with `--dry-run` before running a command that changes it.

## Check a claim against the output

After reading a log, you or an agent may want to check a statement about it. `is` answers
whether the supplied text establishes that statement. Its exit code works with `&&` or `if`:

```sh
jevify is 'the failure is in TestSqlUpdate' --context evals/why/go-01.log && echo established
```

On 20 claims checked against 11 real logs, `is` agreed with all 20 labels and gave no false
yes to a claim the log contradicted or left open (TypeSafe, 2026-10-01,
[claim check](benchmarks/claim-check.md)). A yes means the text establishes the claim. A no
means it does not establish it; the claim may still be true.

## Use jevify with a coding agent

The CLI works through an agent's shell. A failure hook can add the selected diagnostic after
a failed command, and the MCP server lets an agent call `why`, `is` and `pick` directly.
You still decide which commands the agent may run.

### Claude Code

Install the CLI above, then in Claude Code:

```text
/plugin marketplace add tpellet/jevify
/plugin install jevify@jevify
# Restart Claude Code to load the plugin.
```

The plugin runs `jevify why --hook claude` after a failed Bash command. When the output has at
least 80 lines, the hook adds the selected diagnostic, context and saved-log path beside the
error. It stays silent on short output, abstention or timeout and never blocks the tool call.
Claude Code can truncate failed output; pipe the full log to `why` when the error is in the
missing middle.

The plugin also registers the [MCP server](#mcp-server) and the
[skill](plugins/jevify/skills/jevify/SKILL.md).

### Codex

Add a [`PostToolUse` hook](https://developers.openai.com/codex/hooks) in
`~/.codex/hooks.json`, or `.codex/hooks.json` in the repository:

```json
{"hooks": {"PostToolUse": [{"matcher": "Bash",
  "hooks": [{"type": "command", "command": "jevify why --hook codex", "timeout": 30}]}]}}
```

The hook stays silent on successful commands. You can also copy the
[skill](plugins/jevify/skills/jevify/SKILL.md) into `~/.agents/skills/`.

### MCP server

`jevify mcp` exposes `why`, `is` and `pick` over stdio. When nothing fits, the agent receives
an abstention result with the nearest candidates in `data.shortlist`.

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

The action installs jevify and writes the selected diagnostic to the job summary. It leaves
the summary alone when `why` abstains. With `classes`, it also classifies the failure using
the selected lines. On the 34 benchmark runs, 26 match the hand-labelled class, 4 abstain and
4 are wrong (TypeSafe, 2026-10-02, [triage](benchmarks/why-triage.md)).

## Work with lists and changes

| Need | Command |
|:---|:---|
| Get a branch name | `jevify pick --from branch 'the auth refactor'`; also `commit`, `file`, `pr`, `ci-run`, `tool` |
| Records where a statement holds | `jevify filter 'reports a crash' < lines`; `--strict` drops unsure records |
| A bucket for each record | `jevify filter --label bug,feature,question < lines` prints `LABEL<TAB>RECORD` |
| Check several claims | `jevify is 'the failure is in TestSqlUpdate' 'the failure is in TestSqlInsert' --context evals/why/go-01.log` |
| Stage one topic's hunks | `jevify add --dry-run 'the token expiry fix'`; `--yes` stages |

`pick` and `filter` preserve record bytes and input order. `-0` reads NUL-separated records;
`--para` reads paragraphs; `--files` judges a list of paths by their content.
Use `grep`, `rg`, `jq` or `awk` when you know the exact text or field you need. Use jevify when
you can describe the match but need it to judge the meaning. [Every verb](docs/guide/verbs.md).

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

Without a key, jevify asks [classifier.dev](https://classifier.dev/developers), which serves
Jev without an account. The free allowance is $0.50 per IP per UTC day, shared by everyone
behind that IP, subject to $100 per day across everyone and four concurrent requests.
For CI or regular use, get a TypeSafe key from <https://console.typesafe.ai/keys>:

```sh
export TYPESAFE_API_KEY_FILE=/path/to/key
```

When the budget is spent, jevify reports `quota_exhausted` (exit 4). Wait for the daily reset
or use a TypeSafe key. `jevify health` checks the backend. Both backends support the same verbs
and exit codes, with different score scales.

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
