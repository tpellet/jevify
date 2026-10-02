# FAQ

## Do I need an API key?

No. classifier.dev provides a free budget of $0.50 per IP per UTC day, subject to $100 per
day across everyone and four concurrent requests. Set `TYPESAFE_API_KEY_FILE=/path/to/key`
to use TypeSafe credits. `JEVIFY_BACKEND=typesafe|classifier` forces a backend;
`jevify health` checks it. Scores are not assumed interchangeable between backends.

## What does a call cost?

The free backend draws from a dollar budget, so its number of calls depends on request size.
TypeSafe bills your account. `meta.usage` reports requests, waits, cache hits and available
token counts; unknown usage is null. Repeated identical questions can use cached answers for
seven days. jevify does not calculate a billing estimate.

## What leaves my machine?

The description and candidate evidence go to the selected backend with best-effort redaction.
The `tool` kind includes names, summaries and finalist man-page evidence. `--files` can send
file excerpts. [Privacy](../../PRIVACY.md) lists the evidence for every verb.

Only `why` and `filter` save raw input locally, secrets included, under the cache directory's
`outputs/`. A save prunes the store's own files older than seven days. `--no-save` skips the
copy for one call; `JEVIFY_NO_SAVE=1` skips it for every call. `--no-cache` controls the separate
answer cache. A failed or skipped save sets `data.complete=false`.

## Why does it exit 3?

Nothing fits, two candidates are too close, or the evidence is unsure. `pick` and `why` can
abstain instead of selecting. `fill` runs nothing if any marker abstains. Its reasons include
`no_match`, `ambiguous`, `unsure_flag` and `insufficient_evidence`; machine output has
`error: null` and the reason in `data`. A shortlist contains alternatives, not selected handles.

`filter` keeps unsure records unless `--strict`; `filter --label` marks them `?`. Both exit 3
when all records are unsure. `is` exits 3 when no statement is no and at least one is unsure, or the
context is oversized. Write conditions so yes means act: `&&` acts only on 0.

## Why do hidden files still appear?

A hidden or secret-looking path can remain a candidate, but its excerpt is withheld before
reading. The name can still reach the backend. Symlink files also receive no excerpt.
`excerpts withheld: N` includes unreadable files; `filter` leaves those unsure instead of
judging the filename. This is not a guarantee that all secrets are detected.

## Does it run a command or invent arguments?

`fill` substitutes listed handles or options you wrote, then starts the command you supplied.
Inspect `--dry-run`; never `eval` it. Authorize execution as you would the underlying command.
`pick --from KIND` prints a handle without execution; `pick --from tool 'task'` finds an
installed tool. Output verbs execute no user command. `add` stages authorized tracked hunks.

Kinds include `branch`, `commit`, `file`, `dir`, `tool`, `pr`, `issue`, `ci-run`, `stash`,
`process`, `container`, `pod`, supplied records (`-`) and caller-written options (`one`, `flag`).
User recipes live in `kinds.jsonl` in the configuration directory, never the cwd, and cannot
replace shipped kinds. See [Kinds](kinds.md).

## Why does a marker fail before inference?

Quote the whole argument: `'@{branch:the auth refactor}'`. Unknown kinds, an unterminated
marker and a `fill` without any marker are exit 2. Escape literal `@{` as `@@{`.
`@{u}`, `HEAD@{2}` and `user@host:path` pass through literally. Stdin cannot supply both
candidate records and option context; use `--candidates FILE` or `--context FILE` for one role.
A lister failure, overflow or timeout is an error, never a usable partial listing.

## How do I recover from exit 4 or 6?

`quota_exhausted` (4) means spent free budget or TypeSafe credits; immediate retries cannot
restore it. A per-request spending limit is `input_too_large` (6), so narrow the input.
`api_deadline` (4) means the overall time budget expired. `too_many` (6) means the candidate
ceiling was exceeded; use a prefix, `grep`, `head` or a narrower list.
20,000 is the distinct-record ceiling, not a daily service allowance.

## Can I trust a high score?

A score needs task- and backend-specific calibration. It neither grants permission nor proves
that missing evidence agrees. Text can influence its judge. Verify an expensive decision
against the selected evidence; [measurements](how-it-works.md#numbers) quantify particular tasks.
