# Agents

Use `why` when a failure log is too long to read, and `fill` when a command needs a handle
you can describe but cannot name. Cheap tools narrow the input first. Use one process for
many records, rather than reading each file or asking about each line in a loop.

```sh
jevify init agents
jevify capabilities --json
```

`init agents` prints a short instruction block. `capabilities` describes the compact installed
interface. The [robot contract](../ROBOT_MODE.md) defines fields, telemetry and recovery.

## Situations

| Need | Command |
|:---|:---|
| Cause of a failed CI run | `gh run view <id> --log-failed \| jevify why` |
| Commit by what it did | `jevify fill --dry-run -- git show '@{commit:fixes retry backoff}'` |
| Branch by description | `jevify fill --dry-run -- git switch '@{branch:the auth refactor}'` |
| File by content | `git ls-files \| jevify pick --files 'where retries back off'` |
| Installed tool by task | `jevify pick --from tool 'render a terminal demo from a tape file'` |
| Only the handle | `jevify pick --from pr 'the Windows path fix'` |
| Relevant records | `gh issue list \| jevify filter 'reports a crash'` |
| Labels for many records | `gh issue list \| jevify label bug,feature,question` |
| Predicate for the next action | `jevify is 'asks for a refund' --context mail.txt` |

## Machine output

`--json` (alias `--robot`) prints one envelope on one line, including usage errors:

```text
{ok, command, version, exit_code, data, meta, error{kind,message,hint,example} | null}
```

Branch on `exit_code`, then inspect `data`. `ok` is true for no (1) and abstention (3) as well
as success (0). `fill` requires `--dry-run` with `--json`; a successful resolution supplies
`data.argv`. On abstention, `data.shortlist` where present contains candidates and evidence,
not selected handles. `fill` reports each marker's reason and its first failure in argv order.

| Code | Meaning |
|---:|:---|
| 0 | yes, found, done |
| 1 | `is`: any no; `filter`: kept none |
| 2 | usage error |
| 3 | no match, near tie or unsure |
| 4 | unavailable, exhausted quota, deadline or protocol error |
| 5 | missing or rejected TypeSafe key |
| 6 | empty, oversized, unreadable or invalid input |
| 130 | declined at interactive `add` confirmation |

Error kinds are stable. `quota_exhausted` means depleted credits or free budget and is never
retried. A per-request spending limit is `input_too_large` (6); narrow the request.
`api_deadline` (4) means the overall budget expired. Do not retry abstention until it agrees.
`health` performs a small uncached classification, including quota/credit checks through errors.

## Permissions

Allow `fill --dry-run` and authorize `fill` by the underlying command prefix. Quote whole
marker arguments; never `eval` a preview. Stdin has one role, candidates or context; use
`--candidates FILE` or `--context FILE` for the other. A failed marker runs nothing. An unsure
flag cannot silently disappear. A lister failure, overflow or deadline never yields a partial
list. User recipes require explicit `JEVIFY_CONFIG_DIR`, never the cwd or platform config dir.

After execution the child owns the exit code. `JEVIFY_STATUS_FILE=PATH` records `ran`, argv,
reasons and errors before the handoff. A dry run has `ran: false`. An unwritable status file
prevents execution; exec failure after the write is `cannot_run` (6).

Output verbs start no user command. `add` only stages selected tracked hunks; use `--yes` when
staging is authorized. A noninteractive call without it or `--dry-run` exits 2. Human decline
is 130. A high score grants no permission.

## Evidence and privacy

Check `why.considered` against `why.total`. `is` abstains on oversized context; `add` rejects
oversized hunks. `--files` uses excerpts, not complete file review. `pick` and `filter`
preserve record bytes and order; machine text can be lossy for non-UTF-8 input.

Only `why` and `filter` save raw input locally, including secrets, for seven days. Set
`JEVIFY_NO_SAVE=1` or pass `--no-save` to stop it; `--no-cache` controls answer caching
independently. Requests use best-effort redaction. [Privacy](../../PRIVACY.md) lists evidence
and retention; semantic judgments are not security gates.

## Failure integrations

The [Claude Code plugin](../../plugins/jevify/skills/jevify/SKILL.md) includes a Bash
`PostToolUseFailure` hook that supplies a `why` answer for long failures. Its input can be
truncated by Claude Code; pipe the full output when the missing middle matters.
The [GitHub Action](../../action.yml) writes the cause to the job summary:

```yaml
- uses: tpellet/jevify@<tag>
  if: failure()
  with:
    log: build.log
```

Choose a release tag containing the action. Save the earlier failing step with
`2>&1 | tee build.log` and `shell: bash` so the pipeline preserves failure.
