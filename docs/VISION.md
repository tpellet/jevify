# jevify vision

jevify finds the line that explains a failure, and the ID you can describe but cannot name.
An agent knows the command but needs a handle, or has the output but needs the relevant line.
jevify works at those two borders and leaves the command itself to its caller.

## Point at existing things

- Every selected value exists before the question: a line, path, branch, PR, commit or
  caller-written option. Jev writes no text.
- Code owns listing, eligibility, request limits and execution. The model chooses among
  candidates with evidence; it does not invent commands or grant permission.
- NONE is a real choice. Below threshold, no match, a near tie or insufficient evidence means
  abstention, exit 3. `fill` runs nothing if any marker fails.

## Two faces of one job

```sh
# The output is too long to read.
gh run view <id> --log-failed | jevify why
# The command needs an ID the caller can only describe.
jevify fill --dry-run -- gh pr checkout '@{pr:keeps the grid aligned when a tab follows a multibyte character}'
```

`why` returns a line number and context. It retains the full log locally unless saving is
switched off, and reports how much evidence it considered. A selected line is a pointer to
check, not a guarantee of the underlying defect.

`fill` resolves markers in an argv the caller writes. A dry run prints that argv; execution
uses no shell. A literal command name is required. Free text such as titles and messages
stays the caller's responsibility. A handle beginning with `-` gets a safe `./` prefix.

Kinds are things a tool can list (`branch`, `commit`, `file`, `dir`, `tool`, `pr`, `issue`,
`ci-run`, `stash`, `process`, `container`, `pod`), supplied records (`-`), or caller-written
options judged against context (`one`, `flag`). A user recipe specifies a lister and its
handle field. Recipes never load from the cwd or shadow shipped kinds.

```sh
jevify fill --dry-run -- git switch '@{branch:the auth refactor}'
jevify fill --dry-run -- git show '@{commit:fixes retry backoff}'
jevify fill --dry-run -- cat 'src/@{file:parses the marker}'
jevify pick --from tool 'render a terminal demo from a tape file'
```

Quote the whole marker argument. Inspect dry runs, never `eval` them. `pick --from` gives
only the handle. Unchecked command substitution discards its exit status; use `fill` when
that handle becomes an argument. `JEVIFY_STATUS_FILE` distinguishes resolution failure from
the executed command's own exit status.

## Small, composable operations

`pick` selects records, `filter` keeps matching and unsure records, `label` prefixes labels,
and `is` returns a predicate's exit code. `add` stages selected tracked hunks only with caller
authorization. Ordinary `sort`, `awk`, `jq` and `grep` do counting, arithmetic and literal work.

Human output carries records and handles; diagnostics go to stderr. `pick` and `filter`
preserve bytes and input order. Records are lines, paragraphs with `--para`, or NUL-separated
with `-0`. Machine output is one JSON envelope on one line; exit codes are stable.
`jevify capabilities` describes the installed interface and `jevify init agents` gives agents
a short instruction block.

## Bound the work and show the limits

Use one process per question, not one per record. Cheap deterministic tools narrow the input
first. Selection takes at most two rounds of parallel requests, with concurrency bounded by
the backend. Finalists meet in one comparison; probabilities from separate pools are not
compared as absolute ranks. Commit, file and directory selection needs evidence beyond names.

20,000 distinct records is an input ceiling. Two-round selection gives tighter keyless
candidate ceilings. Ordered listings report omitted older candidates; unordered overflow,
lister failure and deadlines are errors. Incomplete evidence cannot authorize a whole-input
verdict or a larger action. An unsure flag cannot silently disappear.

The free backend has a dollar budget: $0.50 per IP per UTC day, subject to $100 per day across
everyone and four concurrent requests. TypeSafe uses the caller's credits. Spent budgets are
`quota_exhausted` (exit 4); a per-request spending limit is `input_too_large` (exit 6).

## Prove the useful cases

On 2026-09-28, TypeSafe puts the root-cause line first in 30 of 34 failed GitHub Actions runs
from 30 public repositories, and points at a wrong line in four. The
[benchmark](../benchmarks/why-ci.md) defines the sample, labels and baselines. Its output token
saving measures the payload an agent reads, not agent task success or total inference cost.

On the same date, TypeSafe resolves bat PR #4018 among 1,000 open and closed PRs in 6.9 seconds,
with 424× fewer bytes than reading the listing. These measurements support particular tasks;
they do not establish correctness or savings for every verb, backend or workflow.

Real end-to-end cases prove selection behavior. Contract tests protect execution safety,
redaction, byte preservation, limits, HTTP handling, deadlines, exit codes and the envelope.
The Claude Code failure hook and GitHub Action put `why` where an agent encounters a long
failure. They preserve abstention and leave the caller in charge of what happens next.
