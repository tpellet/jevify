# Verbs

## Global flags

| Flag | Meaning |
|:---|:---|
| `--json`, alias `--robot` | One JSON envelope on one stdout line, including usage errors |
| `-t, --threshold <0..1>` | Backend fit threshold, default 0.5 |
| `--model <id>` | TypeSafe model, default `jev-1.13.0`; unsupported on classifier |
| `--no-cache` | Bypass answer caching |
| `--verbose` | Scores, requests, tokens and timing on stderr |
| `-V, --version` | Version |

Descriptions can be quoted or supplied as bare words to `pick`, `filter` and `add`.
`is` takes one statement per argument: quote each sentence. Text beginning with `-` goes
after `--`. A verb's options may precede the verb. `filter -v` means inversion.

## fill

```sh
jevify fill --dry-run -- git switch '@{branch:the auth refactor}'
jevify fill --dry-run -- gh pr checkout '@{pr:keeps the grid aligned when a tab follows a multibyte character}'
printf 'retry_backoff\nparse_header\n' | jevify fill --dry-run -- cargo test '@{-:the retry test}'
```

`fill [--dry-run] [-q] [-C DIR] [--candidates FILE] [--context FILE]
[--field N | --key KEY] [-0 | --para] -- COMMAND ARGS...` resolves every marker, then
executes the caller's literal command without a shell. All markers must resolve before
anything runs. `--dry-run` prints shell-quoted argv; inspect it, never `eval` it.
Machine output requires `--dry-run` and supplies `data.argv` on success.

| Marker | Source |
|:---|:---|
| `@{branch:…}`, `@{commit:…}`, `@{file:…}`, `@{dir:…}`, `@{tool:…}` | Local candidate lister |
| `@{pr:…}`, `@{issue:…}`, `@{ci-run:…}`, `@{stash:…}`, `@{process:…}`, `@{container:…}`, `@{pod:…}` | Shipped recipe |
| `@{-:…}` | Stdin or `--candidates FILE` |
| `@{one:bug\|feature\|docs:question}` | Caller-written options, judged against context |
| `@{flag:--draft:question}` | Yes keeps the flag, no drops it, unsure stops execution |

Quote the whole marker argument, including prefixes and suffixes. Marker escapes are `\}`,
`\:` and `\|`; `@@{` preserves a literal `@{`. `@{u}`, `HEAD@{2}` and `user@host:path` are
literal. Unknown kinds, unterminated markers, markers in argv[0], and no marker are exit 2.
A flag marker occupies a whole argument. Do not pass markers through another shell.
A selected handle beginning with `-` receives `./` so it cannot become an option.

Stdin has one role: supplied candidates or option context. Use `--candidates FILE` or
`--context FILE` for the other. Consumed stdin becomes empty for the command; otherwise the
command inherits it. `--field N` selects a 1-based whitespace field; `--key KEY` selects a
JSON field while retaining the record as evidence. `-C DIR` controls the working directory;
a wrapped `git -C DIR` also scopes the listers.

`fill` accepts 3,267 candidates per marker keyless and 13,200 on TypeSafe. Ordered kinds keep
the newest candidates and report coverage; unordered overflow is `too_many` (exit 6).
A lister's failure, output overflow or deadline is an error, never a partial list.
[Kinds](kinds.md) defines recipes, evidence and limits.

Data: `argv`, `reason`, `markers[{arg,kind,reason,handle,p,candidates,total,omitted}]`.
Exit 3 has `error: null`: reasons include `no_match`, `ambiguous`, `unsure_flag` and
`insufficient_evidence`. `reason` names the first failed marker in argv order.
`fill` refuses non-Jev answers, including missing model names, with exit 4 even in a dry run.

Exits 2–6 before execution mean nothing ran. After execution the command owns its exit code.
Set `JEVIFY_STATUS_FILE=PATH` to distinguish them: it records
`{command, version, exit_code, ran, argv, reason, markers, error}` before execution.
A successful dry run has `ran: false`; an unwritable status path prevents execution with
exit 6. The command inherits the variable; unset it in wrappers that invoke another `fill`.

## pick

```sh
git ls-files | jevify pick --files 'where command-line flags are defined'
jevify pick --from tool 'keep my mac awake for an hour'
```

`pick '<description>' [-n N] [--index | --files | --from KIND] [-0 | --para] [-C DIR]`
selects records or handles. Without `--from` it reads stdin; `--from -` also means stdin.
`--index` prints 1-based positions. `--files` judges paths using eligible finalist excerpts;
without piped paths it inventories the current directory and reports that choice on stderr.
`--from KIND` lists candidates itself and conflicts with `--files`, `--index` and split flags.

`pick --from tool` uses PATH summaries and finalist man-page evidence, and starts no user
command. `JEVIFY_INVENTORY_FILE` supplies a fixed inventory when set. Commit, file and dir
selection runs an evidence round. Near ties, NONE and below-threshold fit exit 3 with no
selected record. `-n` requests multiple results; results retain input order.

Data: `matches[{line,text,ordinal,p,lossy?}]`, `any`, `source`; kind selection omits `line`
and adds `reason`, `candidates`, `total`, `omitted`, `windows`, `finalists_per_window`.
On abstention, `shortlist` contains candidates with scores and evidence, not answers.
Exit 0 found, 3 abstained, plus common errors.

## why

```sh
gh run view <id> --log-failed | jevify why
cargo test 2>&1 | jevify why -n 3
```

`why [-C N] [-n N] [--no-save]` reads a log and prints numbered causes with context.
`-C` is context lines here, not a repository directory. It accepts no record split or file
options. Selection uses bounded, filtered evidence; compare `considered` with `total`.
A selected cause does not establish a verdict about unseen lines.

Data: `causes[{line,text,p,context[]}]`, `shortlist`, `any`, `considered`, `total`, `hint`,
`saved_input`, `complete`. Exit 0 found, 3 abstained, plus common errors.
`why` saves the full raw input unless `--no-save` or `JEVIFY_NO_SAVE=1` is set.

## filter

```sh
gh issue list | jevify filter 'reports a crash'
fd -0 -e rs | jevify filter -0 --files 'tests backend throttling'
```

`filter '<statement>' [-v] [-c] [--strict] [-0 | --para] [--files] [--no-save]`
keeps matching and unsure records. `--strict` drops unsure records; `-v` inverts yes/no
selection; `-c` prints the kept count. The question has three answers: holds, does not hold,
does not say. A record with no evidence either way is unsure, not no.

Data: `records[{text,ordinal,p,verdict,lossy?,unreadable?}]`, `kept`, `total`, `unsure`,
`saved_input`, `complete`, `excerpts_withheld`. Exit 0 kept some, 1 kept none, 3 every record
unsure. A later backend failure can leave a prefix on human stdout; check the exit code.
Raw input is saved unless `--no-save` or `JEVIFY_NO_SAVE=1` is set.

## label

```sh
gh issue list | jevify label bug,feature,question | cut -f1 | sort | uniq -c
```

`label a,b,c [-0 | --para] [--files]` prints `LABEL<TAB>RECORD` in input order, `?` when
unsure. Supply at least two distinct, nonempty labels, none `?` or `NONE`; the maximum is
99 keyless or 200 on TypeSafe. Invalid labels exit 2. Input is not saved.

Data: `records[{label,text,ordinal,p,lossy?,unreadable?}]`, `labelled`, `total`, `unsure`,
`complete`, `excerpts_withheld`. Exit 0 labelled, 3 every record unsure, plus common errors.

## is

```sh
jevify is 'asks for a refund' --context mail.txt
printf 'All tests passed.\n' | jevify is 'the tests passed' && echo ready
```

`is '<statement>' ['<statement>' ...] [--context FILE] [--band 0.15]` judges one context.
One statement prints nothing; several print `VERDICT<TAB>STATEMENT`. Exit 0 all yes, 1 any no,
3 otherwise. `--band` accepts 0 through 0.5. Oversized context abstains before inference.
Data for one: `p`, `verdict`, `truncated`; for several: `statements[{statement,verdict,p}]`,
aggregate `verdict`, `truncated`. Oversized input adds `reason` and null probabilities.

## add

```sh
jevify add --dry-run 'the token expiry fix'
```

`add '<topic>' [--dry-run | --yes]` scores complete unstaged hunks of tracked files.
`--dry-run` stages nothing. `--yes` authorizes staging; without it a noninteractive call
exits 2. Declining an interactive confirmation exits 130. It never commits or stages untracked
files. Oversized hunks and batches are rejected before staging, without clipping evidence.
`git apply --cached` rejection leaves the index unchanged; subdirectory calls use the repo root.
Data: `hunks[{file,header,p,staged}]`. Exit 0 scored or staged, 3 no match, plus common errors.

## Records and utilities

`pick`, `filter` and `label` read lines, paragraphs with `--para`, or NUL records with `-0`.
The split flags conflict. `pick` and `filter` preserve selected record bytes and input order;
`label` preserves bytes after the tab. Machine text with invalid UTF-8 has `lossy: true`.
`--files` withholds hidden, secret-looking and symlink excerpts; unreadable files stay unsure
in `filter` and `label`. See [Privacy](../../PRIVACY.md).

| Command | Data | Exit |
|:---|:---|:---|
| `jevify capabilities --json` | Compact commands, flags, exits, error kinds, environment and kinds | 0 |
| `jevify health --json` | `backend`, `base_url`, `key`, `api`, `latency_ms`, `model` | 0, 4, 5 |
| `jevify init agents` | Instruction block in `script` | 0 |

`health` makes a small uncached classification, so it detects quota or credit exhaustion.
`init agents` prints instructions and edits no files.

## Common exit codes

0 success, 1 no, 2 usage, 3 abstain, 4 unavailable or quota exhausted, 5 TypeSafe auth,
6 input, 130 declined at `add` confirmation. `quota_exhausted` is never retried.
A per-request spending limit is `input_too_large` (exit 6). `--json` uses one envelope;
[Agents](agents.md) and [Robot mode](../ROBOT_MODE.md) describe recovery.
