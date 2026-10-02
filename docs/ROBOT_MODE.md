# jevify — robot mode

Use jevify when a failure log is too long to read or a command needs a value you can describe
but cannot name. It selects existing evidence and generates no text. `jevify capabilities
--json` gives the compact installed interface; `jevify init agents` prints a short instruction
block. [Verbs](guide/verbs.md) and [Kinds](guide/kinds.md) give argument details.

## When to call it

| Need | Command |
|:---|:---|
| Cause in a failed CI log | `gh run view <id> --log-failed \| jevify why` |
| PR by description | `jevify fill --dry-run -- gh pr checkout '@{pr:the Windows path fix}'` |
| Branch or commit by description | `jevify fill --dry-run -- git show '@{commit:fixes retry backoff}'` |
| Handle alone | `jevify pick --from branch 'the auth refactor'` |
| Installed tool | `jevify pick --from tool 'keep my mac awake for an hour'` |
| Records matching a statement | `gh issue list \| jevify filter 'reports a crash'` |
| Bucket for each record | `gh issue list \| jevify filter --label bug,feature,question` |
| Condition for the next step | `jevify is 'asks for a refund' --context mail.txt` |

Cheap tools narrow the input first. One process handles many records; avoid loops of `is`
calls and one read per file. Use `--files` with `pick` or `filter` to judge path lists.
Write literal questions about evidence. Counting, arithmetic and date ordering belong to code.

### Other verbs

| Need | Command |
|:---|:---|
| Stage one authorized topic | `jevify add --dry-run 'the token expiry fix'` |

`add` changes the git index and nothing else; the caller authorizes staging with `--yes`.
[Verbs](guide/verbs.md#add) has its contract.

## Input and execution

`pick`, `filter` and `add` join bare words into one description; `is` takes one statement per
argument. Quote each statement. A leading `-` and `pick --from -` name stdin. A verb's options
may precede the verb. `-C DIR` (`--repo DIR`) scopes `fill` or `pick`; a wrapped `git -C DIR`
also scopes its listers. On `why`, `-C N` means context lines.

`fill` takes a literal command after `--`, substitutes its markers, then executes without a
shell. Quote whole marker arguments, including prefixes and suffixes. Inspect `--dry-run`,
never `eval` it. Machine output requires `--dry-run`. Authorize execution by command prefix,
exactly as the underlying command is authorized.

Markers select listed things, supplied records (`-`) or caller-written options (`one`, `flag`).
Escapes are `\}`, `\:` and `\|`; `@@{` writes a literal `@{`. `@{u}`, `HEAD@{2}` and
`user@host:path` pass through. Unknown kinds, unterminated markers, markers in argv[0] and no
marker are usage errors. A flag marker occupies a whole argument: yes keeps it, no drops it,
unsure runs nothing. A handle beginning with `-` gets a `./` prefix.

Stdin cannot supply both candidates and option context. Use `--candidates FILE` or
`--context FILE` for one role. Consumed stdin becomes empty for the child; otherwise it is
inherited. Every marker resolves against one snapshot, and any failure prevents execution.
Never use unchecked `"$(jevify pick …)"` as an argument: the shell discards abstention's status.

User recipes require an explicit `JEVIFY_CONFIG_DIR` and cannot shadow coded or shipped kinds.
The cwd and platform configuration directory supply no recipes. Listers run with stdin null,
prompts disabled, color disabled and a deadline. Failure, overflow or timeout is an error,
never a partial list. Ordered candidate limits report omitted older items; unordered overflow
is `too_many` (6). `tool` uses PATH names, summaries and finalist man-page evidence.

## One envelope

`--json` (alias `--robot`) emits exactly one JSON envelope on one line, including usage errors.
Pipes do not implicitly enable it. Human stdout carries handles and records, stderr diagnostics.

```text
{ok, command, version, exit_code, data,
 meta{backend, model, elapsed_ms, requests, cache_hits, input_tokens,
      threshold, request_id, usage, telemetry, decision},
 error{kind, message, hint, example} | null}
```

Branch on `exit_code`, which equals process status, then inspect `data`. `ok` only means jevify
completed without its own error: it is true on no (1) and abstention (3). Error kinds are stable;
messages are for people. `error.example` gives a corrected command to inspect before running.

| Verb | Data |
|:---|:---|
| `fill --dry-run` | `argv` on success, `reason`, `markers[{arg,kind,reason,handle,p,candidates,total,omitted}]` |
| `pick` | `matches[{line,text,ordinal,p,lossy?}]`, `any`, `source` |
| `pick --from` | matches without `line`; `reason`, `candidates`, `total`, `omitted`, `windows`, `finalists_per_window` |
| `why` | `causes[{line,text,p,context[]}]`, `any`, `considered`, `total`, `hint`, `saved_input`, `complete` |
| `filter` | `records[{text,ordinal,p,verdict,lossy?,unreadable?}]`, `kept`, `total`, `unsure`, `saved_input`, `complete`, `excerpts_withheld` |
| `filter --label` | `records[{label,text,ordinal,p,lossy?,unreadable?}]`, `labelled`, `total`, `unsure`, `complete`, `excerpts_withheld` |
| `is` | `p`, `verdict`, `truncated`; several statements add `statements[{statement,verdict,p}]`; oversized context adds `reason` |
| `add` | `hunks[{file,header,p,staged}]` |
| `capabilities` | commands, flags, exits, error kinds, environment, kinds and safety |
| `health` | `backend`, `base_url`, `key`, `api`, `latency_ms`, `model` |
| `init agents` | `script` |

An abstaining selection can provide `data.shortlist`: candidates with evidence and scores,
not chosen answers. Exit 3 never authorizes use of a shortlisted handle. `fill` gives
`data.reason` for the first failed marker in argv order and `data.markers[].reason` per marker.
`no_match`, `ambiguous`, `unsure_flag` and `insufficient_evidence` are reasons, not error kinds;
`error` is null on abstention.

## Exit codes and recovery

| Code | Meaning and next step |
|---:|:---|
| 0 | yes, found or successful operation |
| 1 | `is`: any no; `filter`: kept none |
| 2 | usage; inspect `error.example` |
| 3 | no match, near tie or unsure; inspect evidence, narrow the question or choose explicitly |
| 4 | unavailable, deadline, protocol failure or exhausted quota; branch on error kind |
| 5 | missing or rejected TypeSafe key |
| 6 | empty, oversized, unreadable or rejected input; narrow or repair it |
| 130 | person declined `add` confirmation |

`&&` acts only on 0. Under `git bisect run`, map 3 to 125 (skip), and handle operational errors
separately. Do not retry uncertainty until it agrees.

- `quota_exhausted` (4): TypeSafe credits or the free budget are spent. No automatic retry.
  classifier.dev allows $0.50 per IP per UTC day, subject to $100 per day across everyone and
  four concurrent requests; no fixed daily classification count is promised.
- `input_too_large` (6): includes a per-request spending limit. Reduce request size.
- `api_unavailable` (4): transport or service failure; retries remain bounded by the deadline.
- `api_deadline` (4): the overall `JEVIFY_DEADLINE` budget expired; split work or raise it.
- `api_protocol` (4): a response jevify cannot interpret; report it.
- `api_rejected_request` (6): HTTP 413/422; inspect the error for size or malformed-body details.
- `too_many`, `lister_failed`, `recipe_invalid` (6): narrow candidates, run the lister, or fix
  the named recipe line. No failed lister's partial output reaches selection.

`health` sends a small uncached classification to check usability, including credit or quota
exhaustion. It consumes backend budget and reports errors through the same envelope.

## The exec status

After `fill` executes, the command owns its exit code, including codes 2–6. Exec mode emits no
envelope. `JEVIFY_STATUS_FILE=PATH` records the decision before execution:

```text
{command, version, exit_code, ran, argv, reason, markers, error}
```

`ran: false`, or no file, means nothing ran. A successful dry run has exit 0 and `ran: false`.
`ran: true` records the attempted handoff; if exec itself fails after the write, `fill` reports
exit 6 `cannot_run` on stderr. An unwritable status path is `status_file_unwritable` (6) and
prevents execution. The child inherits the variable; unset it before a nested `fill`.

## Evidence and records

`pick` and `filter` preserve selected bytes and input order; `filter --label a,b,c` prints
`LABEL<TAB>RECORD`, `?` when unsure, and takes no statement, `-v`, `-c` or `--strict`.
Ordinals are 1-based. Non-UTF-8 machine text is lossy and marked `lossy: true`. `-0` and
`--para` change record splitting and conflict. Identical records are judged once.
`filter` keeps unsure records unless strict; with or without `--label`, it exits 3 when all
are unsure. A late error can leave a prefix on human stdout.

`--files` withholds hidden, secret-looking and symlink excerpts. Unreadable files are named on
stderr, and remain unsure in `filter` without a model request. The path can still
be a candidate even when its content is withheld. [Privacy](../PRIVACY.md) states the checks.

Compare `why.considered` with `why.total`; incomplete evidence cannot prove a whole-log verdict.

### `why --hook HOST`: the tool hook

`jevify why --hook claude` (or `codex`) is the body of an agent's tool hook, in place of `--json`.
It reads the host's `PostToolUseFailure` or `PostToolUse` payload for the `Bash` tool on stdin,
takes the failed command's output from `error` (behind its `Exit code N` line) or from
`tool_response{stdout,stderr,exit_code}`, and prints one object,
`{hookSpecificOutput: {hookEventName, additionalContext}}`, where `additionalContext` names the
pointed line, the output's line count, the saved-output path and the cause's context block. It
prints nothing when the output has fewer than `--min-lines` lines (default 80,
`JEVIFY_HOOK_MIN_LINES`), the command succeeded or was interrupted, the payload is not that
shape, `why` abstains, the backend fails, or 20 seconds pass. It always exits 0, never blocks the
tool call, and takes the usual `-C` and `--no-save`. The plugin's `hooks/why-on-fail.sh` is this
one command.
`is` abstains on oversized context. `add` rejects oversized hunks and stages only selected
tracked hunks. Noninteractive `add` without `--yes` or `--dry-run` exits 2; interactive decline
is 130. A failed `git apply --cached` stages nothing. Authorization belongs to the caller.

## Telemetry

`meta.model` is the answering model, multiple names joined by `", "`. Missing is unknown.
`fill` requires Jev, including dry runs. `meta.request_id` identifies the last reported
TypeSafe inference request. Unknown token counts are null, never measured zero.

`meta.requests` counts inference POST attempts, including failures and retries. `meta.telemetry`
separates inference POSTs, health GETs, prewarm GETs and semantic calls. Transport counters obey
`attempted = succeeded + failed + cancelled + in_flight`. A send starts an attempt, not waiting
for a concurrency permit. HTTP success can still fail semantic validation. Cache hits make no
inference attempt. `logical_rounds` is null because HTTP accounting cannot infer stages.

`meta.usage` reports `attempted`, `succeeded`, `waited{count,total_ms}`, `cache_hits` and
`tokens{input,output}`. Retry waits count elapsed completed or interrupted sleeps, not HTTP or
semaphore waits. Detailed token accounting records reported subtotals and unknown attempts;
`reported_attempts + unknown_attempts = inference_posts.attempted`.

`meta.decision` records the gates used: `best`, `next`, `none`, `any`, `fails`, with unused
scores null. `JEVIFY_DECISION=round_one` adds windows and finalists; ordinary output omits that
larger evidence structure. Relative probabilities from different candidate pools are not
absolute ranks. A high score needs task-specific calibration and cannot grant permission.

Connections have a five-second timeout and responses a sixty-second timeout. The overall
`JEVIFY_DEADLINE` defaults to 600 seconds and bounds requests, queues and retries. An accepted
`Retry-After` completes before the next send; filter batches refuse waits longer than 60 seconds.

## Privacy and retention

Redaction is best effort; text can influence its judge, so do not use semantics as a security
gate. Requests stay pinned to the backend host and never follow redirects with evidence or keys.
Only `why` and `filter` save raw input, secrets included, for seven days. `--no-save` or
`JEVIFY_NO_SAVE=1` disables that copy; `--no-cache` controls answers independently.
`complete` describes output completion and saving, not the proportion of evidence judged.

Answer-cache values contain model names and decisions, not evidence; keys hash redacted
requests. Entries are ignored after seven days, without deleting files. Tool inventory files
hold PATH names and summaries without expiry, and are written under an explicit
`JEVIFY_CACHE_DIR` even with `--no-cache`. Neither diagnostics nor telemetry prints credentials.

## The MCP server

`jevify mcp` serves `why`, `is` and `pick` as MCP tools over stdio: JSON-RPC 2.0, one message
per line, nothing but protocol messages on stdout, diagnostics on stderr. It exits when stdin
closes. It answers the `initialize` handshake of protocol revision 2025-11-25 (earlier known
revisions are echoed), `server/discover` and the per-request `_meta` of revision 2026-07-28,
`ping`, `tools/list` and `tools/call`; an unsupported `_meta` version is error `-32022` with the
supported list. Every call loads its configuration from the environment the client gave the
process (`TYPESAFE_API_KEY_FILE`, `JEVIFY_*`) and reports its own `meta`.

| Tool | Arguments | Runs |
|:---|:---|:---|
| `why` | `path` or `text`, one of the two | `why -C 3 -n 1` on the log; saves raw input by the verb's rules |
| `is` | `statement`; `context` or `context_path`, one of the two | `is` with the default band |
| `pick` | `description`; `items` (strings) or `from_kind` (`commit`, `branch`, `file`, `tool`, `pr`, `run`) with an optional `cwd` | `pick` on the items, or `pick --from` in `cwd`; selects only, starts no command |

A result carries the [envelope](#one-envelope) as `structuredContent` and one short text
(`line 155: …`, `yes (p 0.91)`, the chosen item). Branch on `structuredContent.exit_code`
exactly as on the process exit code: 0 found or yes, 1 no, 3 nothing fits or unsure, with
`data.shortlist` naming the nearest candidates, which are not answers. An abstention is a
plain result, never `isError`. A jevify error (`usage`, `bad_api_key`, `quota_exhausted`,
`empty_input`, …) is a tool execution error: `isError: true`, the text `kind: message; hint`,
and the envelope with `error.kind` and `exit_code`. Unknown tools, missing or malformed
parameters and unknown methods are JSON-RPC errors (`-32602`, `-32601`); malformed JSON is
`-32700`.

Claude Code, through the plugin's `mcpServers` or by hand:

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

Claude Desktop installs the bundle built from `packaging/mcpb/manifest.json` with the
`jevify` binary at `server/jevify` inside it (`mcpb pack`); its one setting is the key file
path, blank for keyless use. A desktop client launches the server in a directory of its own,
so `pick` with `from_kind` names the repository in `cwd`.
