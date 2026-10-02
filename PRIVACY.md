# What leaves your machine

jevify sends evidence to the selected backend:

| Backend | Selected when | Destination | Authentication |
|:---|:---|:---|:---|
| `typesafe` | a key is set, or `JEVIFY_BACKEND=typesafe` | `https://api.typesafe.ai` | your key |
| `classifier` | no key is set, or `JEVIFY_BACKEND=classifier` | `https://classifier.dev` | none |

`JEVIFY_BASE_URL` accepts only HTTPS at the active backend's host on port 443. Host comparison
ignores case; trailing dots, other hosts or ports, userinfo and malformed URLs are rejected.
Blank uses the default. Local testing accepts `localhost` and `127.0.0.1` with any scheme and
port, without userinfo; TypeSafe test endpoints receive the bearer key too. Inference, prewarm
and health requests never follow redirects. The TypeSafe key never goes to classifier.dev.

## Evidence per verb

| Verb | Sent | Not sent |
|:---|:---|:---|
| `fill` | descriptions, option context and candidate evidence: branch names and subjects; commit subjects and finalist bodies, paths, diffstat and patch excerpts; file paths and finalist excerpts; tool names, summaries and finalist man-page evidence; recipe listing records | literal command arguments outside markers; withheld file content; the command executes locally |
| `pick --from` | the same evidence as `fill` for the kind | no user command executes |
| `pick` | description and distinct, bounded selection records | evidence beyond the per-item budget |
| `pick --files` | description, paths and eligible finalist excerpts | withheld file content and paths outside the candidate list |
| `filter` | statement, or the labels of `--label`, and distinct record evidence; with `--files`, paths and eligible excerpts | content beyond excerpts; unreadable files remain unsure (`?` under `--label`) without a request |
| `why` | bounded failure-log selection evidence | lines filtered out locally |
| `is` | statements and complete supported context | oversized context abstains before inference |
| `add` | topic and complete unstaged hunks of tracked files | untracked files and content outside the diff; oversized hunks are rejected |
| `mcp` | per tool call, what the verb sends: `why` the log named by `path` or given as `text`, `is` the statement and the context, `pick` the description and the supplied items or the listed kind's evidence | nothing between calls; no tool starts a user command; the `why` tool saves raw input by the verb's rules |
| `health` | a small fixed classification probe and TypeSafe key when selected | user text |
| `capabilities`, `init agents` | nothing | all local data |

PDF excerpts use text from the first two pages when `pdftotext` is installed, clipped to
2,000 characters. Excerpts do not establish whole-document verdicts.

`fill --dry-run` sends evidence but starts no user command. Execution starts only the command
the caller wrote. Listers run Git or the owning tool with stdin null, `GH_PROMPT_DISABLED=1`,
`GIT_TERMINAL_PROMPT=0`, `NO_COLOR=1` and a deadline. `tool` reads PATH and man-page evidence.
`capabilities.kinds` lists exact lister argv. User recipes require `JEVIFY_CONFIG_DIR` and
cannot shadow shipped kinds; the cwd and platform configuration directory supply no recipes.
A lister failure, output overflow or deadline never supplies partial candidates.

## Redaction and withheld excerpts

Before reading a file excerpt, jevify withholds paths whose written components:

- start with `.` (except navigation `.` and `..`) or `id_`;
- end with `.pem` or `.key`;
- contain `credentials` or `secret`.

Checks are case-sensitive. Symlink file entries also receive no excerpt. The path remains a
candidate and can leave the machine as a name. `excerpts withheld: N` also counts unreadable
files, reported individually on stderr. This is not a filesystem sandbox.

Before requests, obvious secrets (`token=…`, `Bearer …`, `sk-…`, `ghp_…`, `AKIA…`, JWTs)
are masked as `[REDACTED]` in semantic state and questions. Opaque option IDs stay stable.
Masking is best effort, not a guarantee. jevify never prints or logs your API key;
`TYPESAFE_API_KEY_FILE=/path/to/key` keeps it out of command text.

## Local stores

The cache base is `JEVIFY_CACHE_DIR`, otherwise `~/Library/Caches/jevify` on macOS or
`$XDG_CACHE_HOME/jevify` / `~/.cache/jevify` on Linux.

| Store | Contents | Retention | Disable |
|:---|:---|:---|:---|
| Answers, `answers/<2 hex>/<blake3>.json` | answering model and decisions: probabilities and chosen options; no record text, excerpts, paths or question text. Options are opaque IDs except caller-supplied labels and fixed verdict phrases. The request survives as a redacted-request hash | entries ignored after seven days; files remain | `--no-cache` or `JEVIFY_NO_CACHE=1` |
| Saved inputs, `outputs/<blake3-16>.log` | full raw bytes, including secrets; written only by `why` and `filter` | seven days; each save prunes the store's own older files | `--no-save` or `JEVIFY_NO_SAVE=1` |
| Tool inventory, `inventory-<fingerprint>.json` | PATH executable names and one-line man-page summaries, written by the `tool` kind when `JEVIFY_CACHE_DIR` is explicit | no expiry; changed PATH produces another fingerprint | leave `JEVIFY_CACHE_DIR` unset; `--no-cache` does not stop inventory storage |

Every inference verb can cache answers. A cache hit sends no request: `meta.cache_hits`
increases, `meta.requests` does not. Cache identity includes backend, endpoint, model and
decision-contract version. `--no-cache` and `--no-save` control different stores.

Only `why` and `filter` save raw input, before inference, with directory mode 0700 and file
mode 0600. Saving identical input refreshes its retention. The path appears on stderr and in
`data.saved_input`; a failed or skipped save sets `data.complete=false`. `filter --label`
saves no raw input, though its answers can be cached.

Pruning only visits `outputs`, follows no symlink, descends into no subdirectory and deletes
only regular files with the store's own names (`<blake3-16>.log` and stranded temporary names).
If `outputs` is a symlink, pruning does nothing. Files with unrelated names remain.

`complete` concerns the run's output, not its evidence coverage: on `why` it records whether
the raw input was saved; on `filter` it also requires that processing completed. Read `unsure`
against `total`, and `why.considered` against `why.total`, to assess what was judged.

## Service limits and integrations

classifier.dev's free budget is $0.50 per IP per UTC day, subject to $100 per day across
everyone and four concurrent requests. Spent free budget or TypeSafe credits produce
`quota_exhausted` (exit 4), never retried. A per-request spending limit produces
`input_too_large` (exit 6). A late failure can leave a prefix on human stdout.

The MCP server (`jevify mcp`) runs the same verbs in process for the client that launched it:
each call sends its own evidence, loads its own configuration from the environment, and the
`why` tool saves raw input by the verb's rules. Only JSON-RPC leaves on stdout; diagnostics go
to stderr, which the client may show or discard. The Claude Code failure hook sends the failed
Bash output it receives through `why` and can save the raw input by the same rules. Claude Code can truncate that output. The GitHub Action
sends the specified log through `why --no-save` and writes the chosen context to the job
summary; that summary has the workflow's visibility. `health` consumes a small classification
to verify usability, so it can detect quota or credit exhaustion.

Service policies: [TypeSafe](https://docs.typesafe.ai/legal),
[classifier.dev privacy](https://classifier.dev/privacy) and
[terms](https://classifier.dev/terms). Free requests need no account, but the service sees
your IP and the `jevify/<version>` user agent. Both backends receive evidence off the machine.
