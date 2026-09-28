# How it works

## Selection and NONE

jevify selects existing records, installed tools, handles and tracked hunks. Jev writes no text.
Code supplies the candidates and the evidence. Every choice includes NONE, so a candidate
need not win merely because something must. `fill` substitutes real handles or caller-written
options and starts the caller's command only when every marker resolves.

## What the threshold decides

`-t` (default 0.5) gates the yes/no fit of the pool: whether anything listed answers the request.
A winning Choice probability must beat both the runner-up and NONE, and reach twice the larger
of them. A winner's own probability need not exceed `-t`: Choice probabilities are relative
to the pool, not comparable across requests. Below threshold, NONE and a near tie abstain.
TypeSafe uses Noul for yes/no; classifier.dev translates it to binary Choice. Their scores
are not assumed calibrated alike.

## Bands and verdicts

`is` has an unsure band (`--band`, default 0.15): yes at threshold plus band or above, no below
threshold minus band, unsure between. Several statements share a context; all yes exits 0,
any no exits 1, otherwise exit 3. Oversized context abstains before inference.

`filter` asks whether a record says the statement holds, says it does not hold, or does not say.
Yes requires P(holds) at least threshold + 0.15; no requires P(does not hold) at least that
mark; everything else is unsure. It keeps unsure records unless `--strict` is set. `-v`
inverts yes/no selection. An unsure flag in `fill` abstains rather than dropping an option
that might be protective.

## Records and selection rounds

Records are lines, paragraphs (`--para`) or NUL-separated (`-0`). The two split flags conflict.
Blank records are omitted. Selected human output preserves original bytes, including CRLF
and non-UTF-8; machine output uses replacement text and `lossy: true` where needed.
Identical records are judged once, then mapped back to their occurrences; `pick` returns the
first occurrence of a selected distinct record. Stdin is bounded at 64 MiB and distinct
records at 20,000. These are input limits, not daily allowances.

Selection takes at most two rounds of parallel calls. TypeSafe windows have 200 candidates
plus NONE; keyless windows have 99 plus NONE. Finalists are retained by rank within windows,
then compared together; scores from different requests are not used as absolute ranks.
Finalist evidence receives budget in rank order. Commit, file and directory selection runs
the evidence round, including with `pick --from`, because names alone can be misleading.
The `tool` kind adds finalist man-page evidence to PATH names and summaries.

`fill` accepts 3,267 candidates per marker keyless and 13,200 on TypeSafe. `pick` and
`pick --from` accept 9,801 and 20,000. Ordered kinds retain the newest candidates and report
omissions; unordered overflow is exit 6. Lister failure or timeout never yields a partial list.
[Kind rules](kinds.md) describe the evidence and execution boundaries.

`filter` and `label` batch up to 60 records on classifier.dev, each judged alone. On TypeSafe,
20 records share one state, with each question naming its record; independence is not claimed.
Answers flow in input order. A late failure can leave a prefix on human stdout.

## The why prefilter

`why` reads a log and returns line numbers with context from the original input. It removes
blank and repeated lines from selection evidence, and bounds large logs using failure signals
and their neighborhoods. Compare `data.considered` and `data.total`: a selected cause does
not establish a whole-log verdict. Saved input provides a way back to the complete log.

## Cache and saved inputs

Answer-cache identity includes endpoint, backend, decision contract, model and redacted request.
Entries hold answers rather than evidence, and are ignored after seven days; expiry does not
remove files. `--no-cache` or `JEVIFY_NO_CACHE=1` bypasses them.

Only `why` and `filter` separately save raw input, including secrets. Files live in
`outputs/<blake3-16>.log` under the cache directory. A save prunes only the store's own files
older than seven days, without descending through subdirectories or symlinks. `--no-save`
skips one copy; `JEVIFY_NO_SAVE=1` skips every copy. A failed or skipped save sets
`data.complete=false`; completeness of saved output is distinct from evidence coverage.
[Privacy](../../PRIVACY.md) defines both stores and outbound masking.

## Models, retries and evidence

TypeSafe defaults to `jev-1.13.0`; `jev-latest` is a moving alias. Classifier.dev chooses its
own model and rejects overrides. `fill` refuses non-Jev answers or a missing model name,
even for a dry run. `meta.model` joins multiple answering models with `", "`.

`quota_exhausted` (exit 4) covers depleted TypeSafe credits or free budgets and is never retried.
A per-request spending limit is `input_too_large` (exit 6). Transient failures can retry inside
the overall `JEVIFY_DEADLINE`; its expiry is `api_deadline` (4). Requests honor accepted
`Retry-After` waits. HTTP 413 or 422 is `api_rejected_request` (6).
`meta.requests` counts inference attempts including retries, excluding health and prewarm GETs;
unknown token usage is null. [Robot mode](../ROBOT_MODE.md) defines telemetry.

## Numbers

Measured 2026-09-28 on TypeSafe: `why` names the root-cause line first in **28 of 34** failed
GitHub Actions runs from 30 public repositories. A separate `-n 3` run includes the root cause
in its top three in 30. The first-line run points at a wrong line in six and abstains in none.
`tail -n 50` contains the gold diagnostic in 10 cases; a five-line grep baseline contains it
in 11. Median `why` latency is 0.84 seconds.

The downstream agent receives 99.7% fewer estimated tokens in aggregate, comparing complete
JSON stdout with full failed-job logs. This is a payload measurement, not total inference cost
or agent task success; large logs dominate it. The purposive sample, gold labels and failures
are in [benchmarks/why-ci.md](../../benchmarks/why-ci.md).

Measured the same day on TypeSafe: in sharkdp/bat, the description “keeps the grid aligned when
a tab follows a multibyte character” resolves PR #4018 among 1,000 open and closed PRs in
6.9 seconds, 424× fewer bytes than the listing. One successful lookup does not establish a
success rate for all descriptions.

Repeated questions can differ. Measured 2026-09-24 at 0.11.0 on a fixed subset with the cache
off, five cold reruns change 1.6% of answers; eight candidate orders change 5.0%. Most movement
is an answer becoming abstention, but some is a different handle without a warning in the
score. The cache hides this variation by replaying an answer. Those measurements concern
that model, binary and sample; they establish no universal reliability guarantee.
