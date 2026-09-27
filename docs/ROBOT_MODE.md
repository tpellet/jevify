# jevify — robot mode

jevify works on two sides of a command: `fill` turns descriptions into real input arguments
and runs the command; output verbs turn existing records into pointers or decisions.
It selects and never generates. Use `jevify capabilities --json` as the source of truth;
`jevify init agents` prints an instruction block derived from its command and exit tables.

## When to call it

| Trigger | Verb | Classical twin |
|:---|:---|:---|
| about to list branches, commits, files, PRs or runs only to choose one, or a tool can list the needed value | `fill` | argument lookup |
| want the handle without executing | `pick --from KIND` | selection |
| an option depends on unread context | `fill` with `one` or `flag` | conditional arguments |
| a long failed build or a grep that found only the symptom | `why` | — |
| one record described but not named | `pick` | `fzf --filter` |
| many records or files, one question | `filter` | `grep` |
| every record needs a bucket | `label` | an `awk` key |
| the next step depends on a fact | `is` | `test` |
| an unfamiliar task on a large PATH | `route` | command discovery |
| tracked changes mixed across topics | `add` | `git add -p` |
| files that need a home among existing folders | `sort` | folder placement |

Cheap tools go first. Skip jevify when literal search answers the question, the input is short
enough to read, or the exact command is known. Use one `filter` or `label` process for many
records, never a loop of `is` calls. Write a literal statement about the evidence, not a vague
request for advice. No counting, arithmetic, date comparisons or quality judgments; English
works best.

```sh
gh run view --log-failed | jevify why --json
git log --oneline | jevify pick --json 'the commit that renamed the project'
fd -0 -e txt | jevify filter -0 --files 'asks for a refund'
gh issue list | jevify label bug,feature,question | cut -f1 | sort | uniq -c
printf 'All tests passed.\n' | jevify is 'the tests passed' && printf 'ready\n'
jevify route --json 'keep my mac awake for an hour'
jevify add --json --dry-run 'the token expiry fix'
```

`jq`, `cut`, `grep`, `head` and another jevify verb can consume selected records. `pick` and
`filter` preserve their exact bytes and input order; `label` prints `LABEL<TAB>RECORD` with the
record unchanged after the tab. `filter` keeps the records where the statement holds and the
records that do not say, so its output carries the unsure ones and `U unsure` on the status line
counts them; `--strict` keeps only the records where the statement holds. Check a `pick` call's exit before using its output as an
argument; unchecked substitution can turn abstention into an empty argument.

## Verbs and data

- `fill [--dry-run] [-q] [--candidates FILE] [--context FILE] [--field N | --key KEY]
  [-0 | --para] -- COMMAND ARGS...` resolves every marker or runs nothing. Data:
  `argv` on successful dry run, `markers[{arg,kind,reason,handle,p,candidates,total,omitted}]`,
  `reason`. Machine formats require `--dry-run`. The command inherits the environment and
  directory, and owns output, signals and exit code. Consumed stdin becomes empty for it.
  `JEVIFY_STATUS_FILE` records whether the command started; see [The exec status](#the-exec-status).
- `pick '<intent>' [-n N] [--index | --files] [-0 | --para]` reads stdin records.
  Data: `matches[{line,text,ordinal,p,lossy?}]`, `any`, `source`. Exit 0 found, 3 nothing fits.
  `--files` is boolean: `git ls-files | jevify pick --files 'where man pages are parsed'`.
  Paths are ranked first, then eligible excerpts of at most 24 finalists. Input is not saved.
  `pick --from KIND '<intent>' [-n N]` lists a kind's candidates (`branch`, `commit`, `file`,
  `dir`, `tool`, `pr`, `issue`, `ci-run`, `stash`, `process`, `container`, `pod`, or a user
  recipe); plain `pick` uses stdin. It prints handles,
  starts no user command, and conflicts with `--files`, `--index`, `-0`, `--para`.
  Data adds `reason`, `candidates`, `total`, `omitted`, `windows`, `finalists_per_window`;
  its matches have `text`, `ordinal`, `p`, `lossy`, without `line`.
- `why [-C N] [-n N] [--no-save]` (`JEVIFY_NO_SAVE=1` for every call) reads stdin logs and
  prints numbered causes with context;
  it takes no split option. Data: `causes[{line,text,p,context[]}]`, `any`, `considered`, `total`,
  `hint`, `saved_input`, `complete`. Exit 0 found, 3 abstain. Pipe stderr with `2>&1`.
- `filter '<statement>' [-v] [-c] [--strict] [-0 | --para] [--files] [--no-save]`
  (`JEVIFY_NO_SAVE=1` for every call) keeps records.
  `-v` inverts, `-c` counts, unsure records stay unless `--strict`. `--verbose` has no short flag.
  Each record is judged three ways: the statement holds, it does not hold, or the record does
  not say. A record that says nothing either way (`Merge branch 'pr-248'` under "is a bug fix")
  is unsure, not a no; `p` is the probability that the statement holds, and the gate's `none`
  in `meta.decision` is the probability that the record does not say.
  Data: `records[{text,ordinal,p,verdict,lossy?,unreadable?}]`, `kept`, `total`, `unsure`,
  `complete`, `saved_input`, `excerpts_withheld`. Exit 0 kept some, 1 kept none, 3 every record unsure.
- `label a,b,c [-0 | --para] [--files]` tags every record with one of the labels and prints
  `LABEL<TAB>RECORD` in input order; `?` marks an unsure record. Labels: at least two, distinct,
  none empty, none `?` or `NONE`, at most the backend window (99 on classifier.dev, 200 on
  TypeSafe), else exit 2. Data: `records[{label,text,ordinal,p,lossy?,unreadable?}]`, `labelled`,
  `total`, `unsure`, `complete`, `excerpts_withheld`. Exit 0 labelled, 3 every record unsure. Saves nothing.
  Stderr: `jevify label: labelled N of M, U unsure`. `cut -f2-` gives line records back without
  their blank lines; with `-0` and `--para` the record follows the tab unchanged.
- `is '<statement>' ['<statement>' ...] [--context FILE] [--band 0.15]` reads one context.
  One statement prints nothing; several print `VERDICT<TAB>STATEMENT` (`yes`, `no`, `unsure`).
  Data for one: `p`, `verdict`, `truncated`; for several: `statements[{statement,verdict,p}]`,
  aggregate `verdict`, `truncated`. Oversized context adds a reason and null probabilities,
  with no inference. Exit 0 all yes, 1 any no, 3 otherwise.
- `route <intent...>` prints a tool, summary and synopsis; starts no user command and selects
  no arguments. Data: `tool`, `summary`, `synopsis`, `fit`, `ties[{tool,fit}]`,
  `alternatives[{tool,fit}]`. Exit 0 found, 3 nothing fits or two commands are too close to
  tell apart (above the threshold and within 0.10 of each other): `tool` is null and `ties`
  names them, the best first. Missing synopsis is null.
- `add '<topic>' [--dry-run | --yes]` scores tracked unstaged hunks. Data:
  `hunks[{file,header,p,staged}]`. Exit 0 scored or staged, 3 no match, 6 empty or oversized,
  130 declined. Machine mode stages only with `--yes`; never commits.
- `sort <DIR> [--into ROOT] [--apply | --undo LOG]` proposes existing folders.
  Data: `moves[{from,to,p}]`, `skipped[{file,reason}]`, `undo_log`, `applied`. Exit 0 success,
  3 nothing placed or restored, 6 input error. Moves require `--apply` or `--undo`; no prompt.
  Atomic no-replace moves and a unique JSONL recovery journal protect occupied destinations; the
  journal holds absolute path bytes and file identity, is never expired, and goes to the system
  temporary directory rather than the cache directory under `--no-cache`.
  Symlink entries are skipped; same volume only; concurrent source replacement unsupported.
- `capabilities` prints commands, flags, data fields, exit codes, environment and limits.
- `robot-docs [guide|commands|exit-codes|examples|privacy]` prints `topic` and `text` in machine mode.
- `health` reports `backend`, `base_url`, `key`, `api`, `latency_ms`, `models`; exit 0, 4 or 5.
- `init zsh|bash|agents` prints `script`. Shell integration routes through `jevify route`.

`pick`, `filter` and `label` split lines by default, NUL records with `-0`, paragraphs with
`--para`. They limit distinct records to 20,000 within 64 MiB. `filter` and `label` judge
identical records once and restore all occurrences. Non-UTF-8 machine records carry `text`, `lossy: true` and `ordinal`;
human output preserves exact bytes. `pick` also uses `line` for its 1-based input position.

`--files` paths remain candidates when excerpts are withheld. Hidden or secret-looking components
and symlink file entries receive no excerpt. A file whose bytes cannot be read (missing, a
directory in its place, a permission or sandbox denial on it or on a directory above it) is
named on stderr as `excerpt unreadable: PATH: REASON`; `filter` and `label` never judge it by
its name: it is unsure (`?`, p 0) without a request, and its record carries `unreadable: REASON`.
`pick` finalists compete on their names. Status counts both kinds as `excerpts withheld: N`. See
[Privacy](../PRIVACY.md) for the exact checks. Excerpts are not complete file evidence.

Only `why` and `filter` save raw inputs, secrets included. The directory is
`JEVIFY_CACHE_DIR/outputs` or the platform cache directory's `jevify/outputs`, and a saved input
is kept for seven days: each save deletes the store's own files past that age, saving the same
input again refreshes its file, and the pruning stays inside that one directory, follows no
symlink and leaves files it did not write alone.
`--no-save` turns saving off for one call and `JEVIFY_NO_SAVE=1` for every call in an
environment; both are independent of `--no-cache`, which governs answers only. Stderr names the
saved file: `jevify why: full output: PATH` or
`jevify filter: kept N of M, U unsure, full output: PATH`.
A failed or skipped save reports `full output: not saved (REASON)`, with `saved_input=null`
and `complete=false`.

`complete` is about the run's own output, never about judgments. On `label` and `filter` it is
true when every record was emitted, and on `why` and `filter` also when the raw input reached the
store. For judgment coverage read `unsure` against `total`, and for `why`'s selection coverage
`considered` against `total`.

## The marker and kinds

```sh
jevify fill --dry-run -- git switch '@{branch:the auth refactor}'
jevify fill --dry-run -- git revert '@{commit:made folder moves atomic}'
jevify fill --dry-run -- cat 'src/@{file:parses the marker}'
printf 'retry_backoff\nparse_header\n' | jevify fill --dry-run -- cargo test '@{-:the test that retries a failed request}'
printf 'A crash with no reproduction steps.\n' | jevify fill --dry-run -- printf '%s\n' \
  '@{one:bug|feature|docs:what kind of report is this}' '@{flag:--draft:the report lacks steps to reproduce}'
jevify pick --from branch 'the auth refactor'
```

Three families: existing things (`branch`, `commit`, `file`, `dir`, `tool`, and the recipes
`pr`, `issue`, `ci-run`, `stash`, `process`, `container`, `pod`), caller-written options
(`one`, `flag`), supplied records (`-`). `capabilities.kinds` lists every kind with its exact
lister argv and its origin, `coded`, `shipped` or `user`; `-`, `tool`, `one` and `flag` have
empty lister arrays. `branch` and `commit` are newest first; `branch` folds remote twins.
`file` and `dir` take a literal prefix ending in `/`; a `file` finalist adds first lines, withheld
for the patterns in `capabilities.withheld`. A recipe kind is one JSON line in `kinds.jsonl`
under `JEVIFY_CONFIG_DIR` or the platform configuration directory: `kind`, `list`, `field` or
`key`, `ordered`; `capabilities.recipes` states the fields and the rules. jevify reads no recipe
from a repository, a user recipe cannot replace a shipped kind, and a bad file is exit 6
`recipe_invalid` with its line number. Every lister has one 20 s deadline; a missing or
unauthenticated tool is exit 6 `lister_failed` with its own text. `--field N` is a 1-based
whitespace field; `--key KEY` extracts a JSON handle while retaining the record as evidence.
The `fill` status line of a resolved marker reads
`candidates N[ of M[, newest first]][, omitted K], windows W[, excerpts withheld: E]`; the
`not run:` line of an abstention reads `candidates N of M, omitted K`, both parts always
present and no window count.
The last stderr line is `jevify fill: exec <quoted argv>` in a run, `jevify fill: would run
<quoted argv>` under `--dry-run`, or `jevify fill: not run: <reason>` when nothing ran; only
`exec` means the command started.

Quote the whole marker argument with single quotes, including any prefix or suffix. Spell an
apostrophe `'\''`. Marker escapes are `\}`, `\:` and `\|`; `one` separates options with `|`.
`flag` must occupy a whole argument: yes keeps it, no removes it, unsure abstains.
`@@{word:` spells a literal `@{word:`. `'{user}@{host:>8}'` is an unknown kind, exit 2;
`'{user}@@{host:>8}'` is literal. Missing closing braces, unknown kinds and no marker are exit 2.
Do not send markers through another shell (`ssh`, `make`, `xargs`). Preview with `--dry-run`,
never `eval`. Never use `"$(jevify pick …)"` as a command argument.

stdin supplies candidates for `-` or context for `one`/`flag`, never both. Use `--candidates FILE`
or `--context FILE` for the other role; file inputs preserve the command's stdin. All markers
resolve against one snapshot; any abstention prevents the entire execution.

Let W be the backend window: 99 on classifier.dev, 200 on TypeSafe. `fill` keeps three names
per window in the shortlist round and accepts F = W × floor(W / 3): 3,267 or 13,200 candidates,
the whole of which each backend answers, measured 2026-09-24;
with one window, a `file` or `dir` marker always runs its finals, and a `branch` or `commit`
marker runs them when the names leave it undecided or the runner-up stays in play; every name
not ruled out reaches the finals with its evidence, up to 24. `pick` and `pick --from`
accept min(W × W, 20,000): 9,801 or 20,000. They keep three finalists per window when those fit
W, else two when those fit, else one, always by rank within each window. `one` accepts at most
W options; more is exit 2. Ordered kinds retain the newest candidates and report coverage;
unordered overflow is `too_many`. No probabilities from separate requests are compared.

## Exit codes and recovery

Write the condition so that yes means act. `&&` acts only on exit 0; use `case` to distinguish
no, unsure and errors. Under `git bisect run`, map unsure exit 3 to 125.

| Code | Meaning / response |
|---:|:---|
| 0 | yes, found or successful operation |
| 1 | `is`: any no; `filter`: kept none |
| 2 | usage error: read the corrected command in `error.example` |
| 3 | nothing fits or unsure: inspect evidence; do not retry until it agrees; `filter` and `label`: every record unsure |
| 4 | unavailable or quota exhausted: read the error |
| 5 | missing or rejected TypeSafe key |
| 6 | empty, oversized or unreadable input; `too_many`: narrow with `grep` or `head` |
| 7 | reserved |
| 130 | declined at `add` confirmation |

`rate_limit_day` HTTP 429 is exit 4, `daily quota of the free backend reached`, without retry.
Exit 4 carries three situations that need different answers, and `error.kind` says which:
`api_unavailable` is a transport failure, so back off and retry; `api_deadline` is the overall
`JEVIFY_DEADLINE` budget passing, so raise the budget or split the input, since the work was
cancelled and not refused; `api_protocol` is a response jevify could not read, which is a bug to
report. Branch on the kind, never on the message.

For `fill`, exits 2–6 mean nothing ran; after `exec`, the command owns its exit code, including
2–6. A successful dry run exits 0. Stderr lines start with `jevify fill:`; `-q` keeps only
`not run:` lines. Stderr is for a person: read the exec status from a status file.

`capabilities.error_kinds` enumerates every `error.kind` with the exit code it carries, and
`capabilities.exit_codes[].kinds` lists the kinds of each code. Input errors are exit 6:
`empty_input`, `input_too_large`, `api_rejected_request`, `input`, `too_many`, `stdin_is_tty`,
`lister_failed`, `cannot_run`, `recipe_invalid`, `status_file_unwritable`. Run the named lister
yourself for `lister_failed`; narrow with a prefix, `grep`, `head` or a narrower pipe for
`too_many`. `api_rejected_request` is the API refusing the request body: its hint points at the
token budget only when the service's own message names a size limit, and otherwise says the
request is malformed, which is a bug to report.

## The exec status

`fill` replaces itself with the command, so after the hand-off the exit code belongs to the
command, and exit codes 2 to 6 mean one thing before the hand-off and another after it. Exec
mode prints no envelope. `JEVIFY_STATUS_FILE=PATH` closes that gap: `fill` writes one JSON
object to PATH before it starts anything, for every outcome it decides.

```text
{command, version, exit_code, ran, argv, reason, markers, error{kind,message,hint,example} | null}
```

`ran` is the answer. `true` means `fill` handed the process to the command, so the exit code the
caller observes is the command's own. `false`, or no file at all, means nothing ran and the exit
code is jevify's, with `reason` for an abstention and `error` for an error. A successful dry run
is `exit_code` 0 with `ran` false: an argv was resolved and nothing started. `exit_code` is
jevify's own decision, never the command's, which jevify cannot know.

```text
S=$(mktemp)
JEVIFY_STATUS_FILE=$S jevify fill -q -- make test '@{branch:the auth refactor}'
code=$?
jq -e .ran "$S" >/dev/null && echo "the command exited $code" || echo "nothing ran, jevify exited $code"
```

A status file that cannot be written is exit 6 `status_file_unwritable` and nothing runs: jevify
never starts a command it cannot report having started. The command inherits the variable, so a
command that itself runs `jevify fill` overwrites the file; unset it in a wrapper. The one case
the file cannot cover is the `execvp` call failing after the write, which is exit 6 `cannot_run`
on the `jevify fill:` stderr line. Without the variable nothing is written and nothing changes.

Abstention is separate: exit 3, `error: null`, `data.reason` for the first failed marker in
argv order, and `data.markers[].reason` for every marker. `no_match`: read candidates N of M;
`ambiguous`: read the two handles and write one; `unsure_flag`: write the flag or drop the
marker; `insufficient_evidence`: supply a complete context that fits. These are not error kinds.
`fill` requires every answer to come from Jev, even for a dry run. Otherwise exit 4,
`api_unavailable`, `answered by <model>, not Jev`. A missing model name is `unknown` in
`meta.model` and refuses too. Read the quota or model line before retrying exit 4.

Filter batch requests honour numeric `Retry-After` through 60 seconds, refusing longer waits.
A failed later batch can leave a human-output prefix; do not treat it as complete input coverage.

## Envelope and telemetry

`--json` (alias `--robot`) prints one envelope, including usage errors. There is no automatic
JSON switch for pipes. `--format jsonl` prints one line; `--format toon` encodes the same fields.

```text
{ok, command, version, exit_code, data,
 meta{backend, model, elapsed_ms, requests, cache_hits, input_tokens, cost_usd,
      threshold, request_id, usage, telemetry, decision}, error{kind, message, hint, example} | null}
```

`ok` is not the field to branch on. It says only that jevify itself reached the end without an
error of its own, so it is true on exit 0, true on exit 1 where `is` answers no, and true on
exit 3 where a verb abstains and the `data` a caller expects is absent. Branch on `exit_code`,
which equals the process exit code, then read `data`. Error kinds are
stable identifiers. `meta.model` is a string, several answering models joined with `", "`.
TypeSafe defaults to `jev-1.13.0`; classifier chooses its model and rejects explicit overrides.
`meta.request_id` names the last TypeSafe inference request when reported; `health` records none.

`meta.decision` is what every decision of the verb was made with, in one structure:

```text
decision{verb, backend, model{requested, answering}, threshold, gates[{best, next, none, any, fails}],
         round_one?[{windows[{ranks[{index, p}], none, any}], finalists[], n}]}
```

`model.requested` is the model the request names; it is null on classifier, which chooses its own.
`model.answering` is the model the service reported, `unknown` when it did not say; the two stay
apart. `threshold` is the one threshold in force; inside a gate it is compared to `any` alone,
never to `best`, and a selection resolves on the ratio between `best`, `next` and `none` (see
[what the threshold decides](guide/how-it-works.md#what-the-threshold-decides)). `gates` holds one
entry per decision, in decision order: `fill` one per marker (the `flag` and `one` markers first,
then the listing markers, each group in marker order, so a listing marker written before a `flag`
marker gets its gate after it), `is` one per statement, `filter` and `label` one per judged
record, `add` one per hunk, `sort` one per file, `pick`, `why` and `route` one. `best` and `next`
are the two top Choice probabilities, `none` is P(NONE) of that Choice, `any` is the Noul: the
absolute score of a selection, or the whole answer of a yes/no question (`is`, `add`, a `flag`
marker). `filter` asks a three-way Choice and fills three scores: `any` is P(the record says the
statement holds), `fails` is P(the record says it does not hold) and `none` is P(the record does
not say); a record is a yes at `any` ≥ threshold + 0.15, a no at `fails` ≥ the same mark (0.65 by
default), unsure otherwise, so a dropped record's "no" can be read back from its gate. A score the
verb does not use is null. The scores are the backend's own and are not a calibration; a threshold
set for one backend and task says nothing about another. They bound no spread between runs
either: the same question asked twice cold can answer differently at the same score, which
[how it works](guide/how-it-works.md#numbers) measures.

`round_one` is present only under `JEVIFY_DECISION=round_one`, on a verb that ran a tournament;
an ordinary envelope carries no such field, since the field holds every candidate of every
window (a 1,000-line `why` scores 1,000 lines). Asked for, it holds one entry per tournament,
in decision order: `fill` one per listing marker, `pick`, `why` and `route` one. Each entry is
what the shortlist round computed, with no extra request: `windows` in input order, each with
every candidate of that window by rank (`ranks[{index, p}]`), its P(NONE) and its Noul;
`finalists`, the items the finals request held, in its order: the shortlist's picks (`n` per
window, by rank then by window), widened by `fill` to every name not ruled out when one window
of names sends a `file` or `dir` marker to its finals, which it always does, or a `branch` or
`commit` marker whose names left it undecided or its runner-up in play, joined by the panic
lines `why` adds, capped at twelve by `route`; empty when one window decided alone; and `n`.
`index` is the verb's own number, 1-based: the line for `why`, the record for `pick`, the
listing position for `pick --from` and `fill`, the inventory position for `route`. A candidate
below NONE is still listed, so the rank of any item, and whether the finals judged it, reads
from one run.

`meta.requests` counts attempted inference POSTs, including retries and failures, excluding
health and prewarm GETs. `meta.telemetry` separates `inference_posts`, `health_gets`,
`prewarm_gets` and `semantic_calls`. Each group obeys:

```text
attempted = succeeded + failed + cancelled + in_flight
```

An attempt starts immediately before a send, not while waiting for a concurrency permit.
Transport success means HTTP 200 with the complete body, so invalid decisions can succeed at
transport and fail semantically. Dropped futures count as cancelled; pending prewarm stays in
flight. Semantic accounting includes cache hits and locally rejected calls. `semantic_questions`
counts submitted questions. `logical_rounds` is null: HTTP accounting cannot infer stage counts.

`retry_sends` counts actual sends after the first attempt. `retry_waits` counts the retry waits
started, and `retry_sleep_ms` sums elapsed completed or interrupted waits, excluding pending waits,
HTTP time and semaphore waits.

`meta.usage` is the spend at a glance, five fields: `attempted` and `succeeded` inference POSTs
(retries and failures included, as `inference_posts`), `waited{count, total_ms}` (the retry
waits and their elapsed sum), `cache_hits` (answers served from the local cache; a hit is never a
request) and `tokens{input, output}`, the service-reported counts, each `null` when any attempt
left it unknown. An unknown count is never a measured zero: a verb that made no request reports
`0`, one whose backend reports no usage reports `null`.

One request waits 5 seconds for its connection and 60 seconds for the response
(`limits.connect_timeout_s`, `limits.request_read_timeout_s`). Past either it counts as a
transport failure and is retried inside the overall budget, so a response that stalls or arrives
truncated costs the full 60 seconds before jevify gives up on it.

Every verb runs under one overall deadline, `JEVIFY_DEADLINE` seconds (600 by default). A retry
wait that would end past it is not started, the request queued for a permit or in flight at the
deadline is cancelled (`inference_posts.cancelled`), and the verb ends exit 4 `api_deadline`
with a message that names the deadline. No request is sent before a server's `Retry-After` ends.

`usage.input_tokens` and `usage.output_tokens` each contain `reported_subtotal`,
`reported_attempts`, `unknown_attempts` and `complete`. Only valid service-reported counts enter
subtotals; missing fields, malformed bodies, failed transport, cancellation and pending attempts
remain unknown. For each token field:

```text
reported_attempts + unknown_attempts = inference_posts.attempted
```

`meta.input_tokens` is null unless input usage is complete; unknown does not mean zero.
Cache hits add no inference attempt or service usage. `cost_estimate` supplies `basis`,
`input_price_per_mtok`, `reported_input_subtotal_usd` and `complete`. It estimates input tokens
only, not a billing receipt. `meta.cost_usd` is null when that basis is incomplete, except that
a configured zero price yields zero regardless of usage. Classifier defaults to `free_service`;
other pricing uses `configured_input_token_price`. No output-token price is invented.

## Permissions and judgment limits

Allow output verbs and `jevify fill --dry-run` freely. Allow `fill` per command prefix, such as
`jevify fill -- git switch:*`, exactly as the underlying command. jevify is not a permission
system. Output verbs start no user command. The caller authorizes `add` staging and `sort` moves.
`is` abstains on oversized context; `add` rejects oversized hunks and complete batches before
staging. A higher threshold cannot validate missing evidence or grant permission.

`filter` and `label` batch up to 60 records on classifier.dev, each judged alone. On TypeSafe
20 records share a request state and each question names its record; independence is not claimed.
`p` is a backend score. Calibration needs task- and backend-specific evidence; ranks past the
third are candidates without a reliability claim. Text can influence the model with embedded
instructions, so semantic judgments are not security gates.

Two runs of one question can disagree. Measured 2026-09-24 at 0.11.0 over a fixed subset of the
source-held-out set with the cache off, five identical reruns change 1.6 percent of answers and
disagree on 5 of 86 questions; eight orders of the same candidates change 5.0 percent and
disagree on 15 of 58. A ten-record `filter` kept set differs by one record between its union and
its intersection over five cold runs, always through `unsure` and never through `drop`. Most of
that movement is a decision becoming an abstention — exit 3, `unsure` or `none` — which a caller
can retry, widen or hand to a person; the rest is a different handle at the same confidence,
with nothing in the envelope marking it, so confirm a handle against the world before acting on
it wherever a wrong one costs something. Candidate order is part of the question: `git branch`,
`ls` and a find each impose one, and both backends move on the same shuffle. The seven-day
answer cache hides the spread by replaying the first answer, so it surfaces when two callers
race the same query cold.

Outbound secret masking is best effort. Answer cache keys use redacted requests, are ignored after
seven days and never cross backend, endpoint, model or decision-contract versions; the value is
the answer alone (model name, probabilities, chosen option), never record text, and expiry deletes
no files. Every verb that asks a question caches, and only `--no-cache` or `JEVIFY_NO_CACHE=1`
stops it. Raw saved inputs are a separate store, written only by `why` and `filter`, bounded by
the same seven days and turned off by `JEVIFY_NO_SAVE=1`. Two more files share the cache
directory and outlive both bounds: `route`'s tool inventory, which `--no-cache` stops while the
`tool` kind's copy of it reads `JEVIFY_CACHE_DIR` directly and is written even then, and
`sort --apply`'s recovery journal. See `capabilities.answer_cache`.
Neither telemetry nor diagnostics prints credentials.
