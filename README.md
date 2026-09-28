# jevify

Find the line that explains a failure, and the ID you can describe but cannot name.

[![CI](https://github.com/tpellet/jevify/actions/workflows/ci.yml/badge.svg)](https://github.com/tpellet/jevify/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/jevify)](https://crates.io/crates/jevify)

## Install

On macOS or Linux, no Rust needed:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tpellet/jevify/releases/latest/download/jevify-installer.sh | sh
```

Or with Rust 1.87 or later: `cargo install jevify --locked`.

No key or account needed. [classifier.dev](https://classifier.dev) provides a free budget of
$0.50 per IP per UTC day, subject to $100 per day across everyone and four concurrent requests.
Exhaustion is `quota_exhausted` (exit 4), not an invitation to retry. To use your TypeSafe
credits, set `TYPESAFE_API_KEY_FILE=/path/to/key`. `jevify health` checks the backend.

## Why did it fail?

```sh
gh run view <id> --log-failed | jevify why
```

**Root-cause line first in 28/34 failed GitHub Actions runs**, measured on 2026-09-28 with
TypeSafe across public repositories. It points at a wrong line in the remaining six.
[Cases, method and baselines](benchmarks/why-ci.md). A pointer is something to check.

For a local build, pipe stderr too: `cargo build 2>&1 | jevify why`.
The repository's failed-build fixture produces this numbered answer:

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

The full log stays on disk; `--no-save` skips that copy. `why` searches bounded evidence,
so inspect the cause and its context before acting.

### Claude Code

Install the CLI above, then run these commands in Claude Code:

```text
/plugin marketplace add tpellet/jevify
/plugin install jevify@jevify
# Restart Claude Code to load the plugin.
# Failed Bash output reaches the PostToolUseFailure hook.
```

The [hook](plugins/jevify/skills/jevify/SKILL.md#why-on-every-failure-the-hook-and-the-github-action)
runs `why` on long failures and supplies the cause beside the error. It adds nothing on
abstention or timeout. Claude Code can truncate the failed output; pipe the full log to
`why` when the cause is in the missing middle.

### GitHub Actions

Save the failing step's output with `2>&1 | tee build.log` and `shell: bash`, then add
these four lines, replacing `<tag>` with a release tag containing the action:

```yaml
- uses: tpellet/jevify@<tag>
  if: failure()
  with:
    log: build.log
```

The action installs jevify and writes the cause to the job summary. An optional
`typesafe-api-key` input selects TypeSafe; abstention leaves the summary alone.

## Which ID was it?

In a checkout of [sharkdp/bat](https://github.com/sharkdp/bat):

```sh
jevify fill --dry-run -- gh pr checkout '@{pr:keeps the grid aligned when a tab follows a multibyte character}'
```

The resolved command is `'gh' 'pr' 'checkout' '4018'`. On 2026-09-28, TypeSafe selects that
PR from 1,000 open and closed PRs in 6.9 seconds, with 424× fewer bytes than reading the listing.
`fill` selects a real handle and substitutes it into the command you wrote; it generates no text.

```sh
jevify fill --dry-run -- git show '@{commit:fixes retry backoff}'
jevify fill --dry-run -- git switch '@{branch:the auth refactor}'
jevify fill --dry-run -- cat 'src/@{file:parses the marker}'
```

Keep the whole marker argument in single quotes. Inspect `--dry-run`; never `eval` it.
Omit `--dry-run` to run the command. If nothing fits or two candidates are too close,
`fill` exits **3 and nothing runs**. Several markers resolve together; any failure stops them all.
[Kinds and supplied lists](docs/guide/kinds.md) cover PRs, commits, files, pods and your own recipes.

## Also

- `jevify pick 'description' < lines`: one record; `pick --from tool 'task'` selects an installed tool.
- `jevify filter 'statement' < lines`: matching and unsure records; `--strict` keeps only matches.
- `jevify label bug,feature,question < lines`: a label and a tab before each original record.
- `jevify is 'statement' < text`: an exit code for `if` or `&&`.

`pick` and `filter` preserve record bytes and input order. `-0` reads NUL-separated records;
`--para` reads paragraphs. `add --dry-run 'topic'` previews the tracked hunks to stage.

## Exit codes

| Code | Meaning |
|---:|:---|
| 0 | yes, found, done |
| 1 | no (`is`), nothing kept (`filter`) |
| 2 | usage error |
| 3 | nothing fits, ambiguous, or unsure |
| 4 | backend unavailable or quota exhausted |
| 5 | TypeSafe key missing or rejected |
| 6 | empty, oversized or unreadable input |
| 130 | declined at the `add` confirmation |

After `fill` starts the command, its exit code belongs to that command.
`JEVIFY_STATUS_FILE=PATH` records whether anything ran; [agent contract](docs/ROBOT_MODE.md).

## For agents

`jevify init agents` prints a short block for `AGENTS.md`. `jevify capabilities --json` lists
commands, kinds and limits. `--json` prints one line containing
`{ok, command, version, exit_code, data, meta, error}`; branch on `exit_code`.
Use `fill --dry-run --json` for a resolved argv, and authorize execution as you would the
underlying command. The [Claude Code skill](plugins/jevify/skills/jevify/SKILL.md) also works
in Codex when copied into `~/.agents/skills/`.

## Privacy and license

Descriptions and evidence go to TypeSafe or classifier.dev with best-effort redaction.
`why` and `filter` save raw input locally for seven days, secrets included; `JEVIFY_NO_SAVE=1`
disables it. [Privacy](PRIVACY.md) · [Guide](docs/guide/README.md) · [Changelog](CHANGELOG.md).

Powered by Jev from [TypeSafe AI](https://typesafe.ai). [MIT](LICENSE).
