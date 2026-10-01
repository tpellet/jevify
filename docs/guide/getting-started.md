# Getting started

## Install

On macOS or Linux, no Rust needed:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tpellet/jevify/releases/latest/download/jevify-installer.sh | sh
```

With Rust 1.87 or later:

```sh
cargo install jevify --locked
```

Build from `main` with `cargo install --git https://github.com/tpellet/jevify --locked jevify`.

## No key needed

Without a key, jevify asks [classifier.dev](https://classifier.dev), which serves Jev free and
without an account: $0.50 per IP per UTC day, shared by everyone behind that IP, subject to
$100 per day across everyone and four concurrent requests. The free tier is for trying jevify;
CI and sustained or team use need a TypeSafe key, which uses your own credits:

```sh
export TYPESAFE_API_KEY_FILE=/path/to/key
```

`TYPESAFE_API_KEY` works too. jevify reads the key file only when a request needs the key, and
never prints or logs it. `JEVIFY_BACKEND=typesafe|classifier` forces a backend. The two
backends give the same verbs and exit codes; their probabilities are not the same scale.

```sh
jevify health
```

`health` makes a small uncached classification: exit 0 answered, 4 unavailable or exhausted
quota/credits, 5 missing or rejected TypeSafe key. It consumes backend budget.

## First commands

The fixtures live in [docs/demo](../demo) of the repository; `bash docs/demo/examples.sh`
runs the local fixture examples.

Find the error in a failed build. On a live build, pipe both streams, since compilers write
errors to stderr: `cargo build 2>&1 | jevify why`.

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

Keep the lines where a statement holds, or pick the one line you describe:

```console
$ jevify filter 'reports a crash' < docs/demo/issues.txt
jevify filter: 10 records, 10 distinct, 1 requests
#312 Crash when the config file is empty
#290 Panic on non-UTF-8 file names
jevify filter: kept 2 of 10, 0 unsure, full output: ~/Library/Caches/jevify/outputs/c85c7cb6f33fc1f7.log

$ jevify pick 'what I paid a streaming service' < docs/demo/downloads.txt
jevify pick: candidates 8, windows 1
spotify_receipt.pdf
```

Each record gets one of three answers, not two: the statement holds, it does not hold, or the
record does not say. `filter` keeps the records where the statement holds and the records that
do not say, and `--strict` keeps only the records where it holds. On lines with nothing to judge
the difference is the whole output:

```console
$ printf 'build started\nerror: connection timed out\nbuild stopped\n' | jevify filter --strict 'reports a network failure'
jevify filter: 3 records, 3 distinct, 1 requests
error: connection timed out
jevify filter: kept 1 of 3, 2 unsure, full output: ~/Library/Caches/jevify/outputs/bd58efb6741c3dbd.log
```

Drop `--strict` and all three lines come back: `build started` and `build stopped` say nothing
either way about a network failure, so they are unsure, not a no. The status line counts them,
so `2 unsure` is the warning that the question did not reach two of the records.

Put a bucket in front of each line, then count the buckets with `cut`, `sort` and `uniq`:

```console
$ jevify label bug,feature,question < docs/demo/issues.txt | cut -f1 | sort | uniq -c
jevify label: 10 records, 10 distinct, 1 requests
jevify label: labelled 10 of 10, 0 unsure
   4 bug
   3 feature
   3 question
```

Ask a yes/no question and get the answer as an exit code:

```console
$ jevify is 'asks for a refund' < docs/demo/mail.txt && echo refund
refund
```

Find the file that does something, among the files of a repository:

```console
$ git ls-files | jevify pick --files 'where the command-line flags are defined'
jevify pick: candidates 319, windows 4
jevify pick: excerpts withheld: 0
src/cli.rs
```

When nothing fits, stdout stays empty and the exit code is 3:

```console
$ jevify pick 'the tax return' < docs/demo/downloads.txt
jevify pick: candidates 8, windows 1
$ echo $?
3
```

`why` and `filter` save their whole input, secrets included, under the cache directory's
`outputs/` and print the path on stderr. The file keeps for seven days, like a cached answer.
`--no-save` skips the save for one call; `JEVIFY_NO_SAVE=1` skips it for every call.

## Fill an argument

`fill` puts a real value where you wrote a description, then becomes the command. The
description goes in a marker, `@{kind:description}`, and the whole argument goes in single
quotes so that the shell leaves it alone.

```console
$ git log --oneline v0.8.3..v0.9.3 | jevify fill --field 1 --dry-run -- git show --stat --format=%s '@{-:made route abstain when two commands are too close}'
jevify fill: - 317cbf7 0.70 (next 0.15, none 0.15) 317cbf7 fix: route abstains when two commands are too close (hunch-1zs); candidates 30, windows 1; model jev-1.13.0
jevify fill: would run 'git' 'show' '--stat' '--format=%s' '317cbf7'
'git' 'show' '--stat' '--format=%s' '317cbf7'
```

The status line on stderr gives the winner, its probability, the probabilities of the next
candidate and of "none of them", the evidence, the number of candidates and the model. The
listing is the thirty commits of one release of this repository. This run, on the keyless
backend over those 30 commits on 2026-09-24, answers 0.70 with the runner-up at 0.15 and "none
of them" at 0.15. A winner resolves by standing clear of both, not by passing
the threshold on its own score — see
[what the threshold decides](how-it-works.md#what-the-threshold-decides).

Without `--dry-run`, `exec` names the command and the command owns everything after that: its
output, its exit code, your terminal. `--dry-run` prints the command instead of running it.
Look at it; never `eval` it.

```sh
jevify fill --dry-run -- git switch '@{branch:the auth refactor}'
printf 'retry_backoff\nparse_header\n' | jevify fill --dry-run -- cargo test '@{-:the test that retries a failed request}'
```

Three families of value exist. Things a tool can list: `branch`, `commit`, `file`, `dir`,
`tool`, `pr`, `issue`, `ci-run`, `stash`, `process`, `container`, `pod`. Lines you pipe in:
`@{-:…}`. Options you write yourself, judged against a text on stdin or in `--context FILE`:
`'@{one:bug|feature|docs:what kind of report is this}'` picks one option, and
`'@{flag:--draft:the report lacks steps to reproduce}'` keeps the flag on yes, drops it on no,
and stops on doubt. Several markers resolve together; if one fails, nothing runs. stdin has one
role per call: the lines of `@{-:…}`, or the context of `one` and `flag`; the other side comes
from `--candidates FILE` or `--context FILE`.

`pick --from KIND` gives you the handle without a command:

```sh
jevify pick --from branch 'the auth refactor'
```

[Verbs](verbs.md#fill) covers escaping and the exact input rules; [Kinds](kinds.md) lists every
kind and how to add your own.

## Find an installed tool

```sh
jevify pick --from tool 'keep my mac awake for an hour'
```

The `tool` kind selects from PATH summaries and finalist man-page evidence. It prints a
handle and starts no user command. `jevify init agents` prints instructions for an agent.

## Scripting on exit codes

Write the condition so that yes means act. One `is` statement prints nothing; several print
`VERDICT<TAB>STATEMENT` lines. `--context FILE` reads a file instead of stdin.

```sh
printf 'Please refund order 42.\n' | jevify is 'asks for a refund' 'mentions an order'
jevify is 'asks for a refund' --context mail.txt
```

Exit 0 means all yes, 1 at least one no, 3 unsure. `&&` acts on 0 only; use `case` when no,
unsure and backend errors need different handling. Check a `pick` call's exit code before its
output becomes an argument: on abstention the output is empty, and an empty argument is one
that many commands accept.

The codes are 0 ok, 1 no, 2 usage, 3 abstain, 4 unavailable, 5 auth, 6 input and
130 declined at the `add` confirmation. [Verbs](verbs.md) lists the data and flags of each
command. `--json` gives one machine envelope; [Agents](agents.md) describes it. `fill --json`
requires `--dry-run`. `fill` exits 2 to 6 when nothing ran; once the command runs, the exit
code is the command's. A dry run exits 0 when every marker resolves.
