# Configuration

jevify has no config file. Every setting is a flag or an environment variable. `jevify capabilities --json` prints the list below as data and is the source of truth.

## Environment variables

| Variable | Default | Meaning |
|:---|:---|:---|
| `TYPESAFE_API_KEY` | | The API key. Never printed, never logged. Setting it selects the `typesafe` backend. |
| `TYPESAFE_API_KEY_FILE` | | Path to a file holding the key; read only when a request needs a key. Use it to keep the key out of your environment and shell history: `TYPESAFE_API_KEY_FILE=/path/to/key`. |
| `JEVIFY_BACKEND` | `typesafe` with a key, `classifier` without one | `typesafe` or `classifier`: which API answers. See [Backends](#backends). |
| `JEVIFY_BASE_URL` | the active backend's own URL | HTTPS at `api.typesafe.ai` for TypeSafe or `classifier.dev` for classifier, on port 443. Local test endpoints are also accepted; see below. |
| `JEVIFY_MODEL` | `jev-1.13.0` on TypeSafe | TypeSafe model or alias; explicit overrides on classifier are usage errors because the service selects its model. `jev-latest` moves with TypeSafe releases. |
| `JEVIFY_THRESHOLD` | `0.5` | Decision threshold on backend yes/no scores; calibration is task- and backend-specific. |
| `JEVIFY_CONCURRENCY` | `8` on `typesafe`, `4` on `classifier` | Parallel requests within one round (the windows of a tournament). |
| `JEVIFY_DEADLINE` | `600` | The verb's overall budget in whole seconds. A retry wait that would end past it is not started, a request still queued or in flight at the deadline is cancelled, and the verb ends exit 4 naming the deadline. Zero is a usage error. |
| `JEVIFY_CACHE_DIR` | platform cache dir, `jevify` sub-directory | Where answers, the tool inventory, `sort`'s recovery journals and raw saved inputs in `outputs/` live. |
| `JEVIFY_CONFIG_DIR` | platform configuration dir, `jevify` sub-directory | Where the user's `kinds.jsonl` lives. It is read only for a marker kind that is neither coded nor shipped. See [Kinds](kinds.md). |
| `JEVIFY_NO_CACHE` | | Set to `1` to disable the answer cache, and `route`'s tool inventory file (answer entries are ignored after 7 days anyway, but their files stay). It does not disable saved inputs or the `tool` kind's inventory file, and it moves `sort --apply`'s recovery journal to the system temporary directory rather than suppressing it. |
| `JEVIFY_NO_SAVE` | | Set to `1` so `why` and `filter` save no raw input. It is the fleet-wide form of `--no-save`: export it once and no call site has to remember the flag. |
| `JEVIFY_PRICE_PER_MTOK` | `0.042` | Dollars per million input tokens, used for `meta.cost_usd`. Change it if your TypeSafe pricing differs. |
| `JEVIFY_INVENTORY_FILE` | | A JSON array of `{name, summary}` that replaces the PATH inventory for `route`. Used by the tests and the evals so every machine routes over the same tools. |
| `JEVIFY_CNF` | | Set to `1` to enable the command-not-found hook printed by `jevify init`. |

A flag beats its variable: `-t 0.7` wins over `JEVIFY_THRESHOLD=0.5`.

`JEVIFY_BASE_URL` accepts only the active backend's host, compared case-insensitively,
with HTTPS and no port or explicit port 443. Trailing dots, other hosts, other ports,
userinfo (`user:password@host`) and malformed URLs are configuration errors before any
request. An empty or blank value uses the backend's default URL. For local testing,
`localhost` and `127.0.0.1` accept any scheme and port, without userinfo.
Inference, prewarm and health requests never follow redirects.

## Backends

jevify asks one of two APIs. TypeSafe accepts a model selection; classifier controls its answering model.

| Backend | Selected when | Key | Cost |
|:---|:---|:---|:---|
| `classifier` | no key is set | none needed | free ([classifier.dev](https://classifier.dev) runs Jev and serves it free) |
| `typesafe` | `TYPESAFE_API_KEY` or `TYPESAFE_API_KEY_FILE` is set | yours | billed to your key |

`JEVIFY_BACKEND=typesafe|classifier` forces a backend. `typesafe` without a key is exit 5. `meta.backend` in the JSON output and `jevify health` both name the backend that answered, and `meta.model` names the build of Jev behind it.

The free service has tighter limits and maps TypeSafe Nouls to binary Choice questions. The same threshold is exposed, but its calibration and the resulting answers are not assumed equivalent across backends. Constructed fields and label counts are checked locally; unsupported requests fail without silently dropping candidates.

| | `typesafe` | `classifier` |
|:---|---:|---:|
| options per question | 255 | 100 |
| tournament window | 200 | 99 + NONE |
| input per request | 32,000 tokens | 32,000 UTF-16 code units |
| questions per request | bounded by request evidence | 20 dimensions (jevify splits bigger asks) |
| records per `filter` request | 20 sharing one state | up to 60, each judged alone (the keyless spending limit refuses 75 with HTTP 402) |
| rate limit | 1,200 requests/min | 3,000 classifications/min, 20,000/day, per IP; one classification is one record under one question |
| `meta.input_tokens`, `meta.cost_usd` | complete reported input tokens or null, estimated input cost or null | tokens may be null; cost is 0 at the default zero service price |

Classifier also limits each instruction to 4,000 UTF-16 code units, each label to 200, and each dimension name to 64. The compact JSON of all dimension definitions must fit 16,000 UTF-16 code units, including JSON escaping. An emoji outside the basic multilingual plane counts as two units. jevify checks the complete constructed request before sending it.

Routing and root-cause comparisons are in [evals/](../../evals/). Equal aggregate scores do not establish interchangeable probabilities. `meta.model` is a string; several reported answering models are joined with `", "`.

A `rate_limit_day` HTTP 429 returns exit 4, `daily quota of the free backend reached`, without retry.
Filter batches honour numeric `Retry-After` through 60 seconds and refuse longer delays. No request
is sent before a `Retry-After` ends, and the sum of the waits stays under `JEVIFY_DEADLINE`.
`meta.usage` reports the attempts, successes, waits, cache hits and tokens of the run.

Free calls a day on one IP, computed from the shape of each verb's requests. The per-request
costs of `is` and `filter` were measured on 2026-09-22 with `JEVIFY_CONCURRENCY=4`; the `pick`,
`why` and `route` runs of that day never completed (HTTP 502), so their costs are read from
the requests jevify builds ([benchmarks/results.md](../../benchmarks/results.md)). `is` costs
one classification per statement, so 6,600 calls of three statements to 20,000 of one;
`filter` and `label` cost one per distinct record, so 20,000 records in total, and jevify
sends at most 60 records per request, the largest batch tried (75 is refused with HTTP 402
`request_spending_limit`, reported as exit 4 `api_unavailable`; 61 to 74 were not tried);
`pick` costs two per window of 99 lines, plus two for the final round when there is more than
one window, so 830 calls of 1,000 lines to 10,000 of at most 99; `why` costs the same but
always runs its final round, so 830 calls of 1,000 lines to 5,000 of at most 99; `route`
costs two per window of 99 commands plus one per finalist, at most 12, so about 380 calls over
a PATH of 1,883 commands. `capabilities.backends` carries the same figures.

## Global flags

| Flag | Variable | Meaning |
|:---|:---|:---|
| `--json` (alias `--robot`) | | One JSON envelope on stdout, usage errors included |
| `--format human\|json\|jsonl\|toon` | | Output format; overrides `--json` |
| `-t, --threshold <0..1>` | `JEVIFY_THRESHOLD` | Decision threshold on backend yes/no scores |
| `--model <id>` | `JEVIFY_MODEL` | TypeSafe model or alias |
| `--no-cache` | `JEVIFY_NO_CACHE` | Skip the local answer cache |
| `--no-save` (`why`, `filter`) | `JEVIFY_NO_SAVE` | Save no raw input |
| `--verbose` | | Probabilities, request count, tokens, cost and timing on stderr; no short flag |
| `-V, --version` | | Print the version |

`filter -v` means inversion. Per-verb flags are in [Verbs](verbs.md).

## Limits

From `capabilities.limits` (the `typesafe` figures; `capabilities.backends` lists both backends):

| Limit | Value |
|:---|---:|
| options per question (`choice_options`) | 255 |
| tournament window (`window`) | 200 |
| model state tokens (`state_tokens`) | 32,000 |
| request tokens (`request_tokens`) | 64,000 |
| requests per minute (TypeSafe) | 1,200 |
| tokens per second (TypeSafe) | 250,000 |
| classifications per minute (classifier.dev, free, per IP) | 3,000 |
| stdin bytes (`stdin_bytes`) | 67,108,864 (64 MiB) |
| distinct records in `pick` and `filter` | 20,000 |

Past the byte or distinct-record ceiling jevify exits 6 before inference; the record ceiling uses
`error.kind=too_many`. The token limits are the API's. A request that exceeds them after jevify's
own budgeting comes back as `api_rejected_request`, exit 6.

## Where files live

- Base directory: `JEVIFY_CACHE_DIR`, otherwise the platform cache directory (`~/Library/Caches/jevify` on macOS, `$XDG_CACHE_HOME/jevify` or `~/.cache/jevify` on Linux).
- Answers live in `answers/<2 hex>/<blake3>.json`, one file per request, written by every verb that asks a question: `fill`, `pick`, `why`, `route`, `filter`, `label`, `is`, `add`, `sort`. The file name is the hash of the redacted request; the file itself holds the answer alone — the answering model's name, and per question a probability, the chosen option and a probability per option. No record text, no excerpt, no path and no question text; options are the opaque IDs the request used, except the labels you pass to `label` and `filter`'s three fixed phrases. Entries are ignored after seven days and `--no-cache` disables the cache, but expiry deletes no files.
- Only `why` and `filter` save raw input, secrets included, as `outputs/<blake3-16>.log` under the base directory. It is kept for seven days: each save deletes the store's own files past that age, and saving the same input again refreshes its file. Pruning stays inside `outputs`, follows no symlink and leaves files it did not write alone.
- `--no-save` disables saving for one call and `JEVIFY_NO_SAVE=1` for every call, both independently of `--no-cache`; a skipped or failed save sets `data.complete=false`. `data.complete` is about the run's own output, never about how many records were judged — that is `unsure` against `total`.
- `sort --apply` writes a unique JSONL recovery journal and prints its path (`data.undo_log`): absolute source and destination path bytes plus each file's device and inode, never file content, and never sent to a model. Nothing turns it off and nothing expires it; under `--no-cache` it is written to the system temporary directory (`TMPDIR`) instead of the base directory. Preserve journals needed for undo.
- The tool inventory, `inventory-<fingerprint>.json`, holds the names and man-page one-line summaries of the executables on your PATH. It is never expired, and a changed PATH writes a new file under a new fingerprint. `--no-cache` stops `route` from writing it; the `tool` kind of `fill` and `pick --from` reads `JEVIFY_CACHE_DIR` directly and writes the file even under `--no-cache`, and writes nothing at all when that variable is unset.
- Configuration directory: `JEVIFY_CONFIG_DIR`, otherwise the platform configuration directory (`~/Library/Application Support/jevify` on macOS, `$XDG_CONFIG_HOME/jevify` or `~/.config/jevify` on Linux). It holds `kinds.jsonl`, the user's own marker kinds; jevify reads no recipe from a repository.

## Shell integration

```sh
eval "$(jevify init zsh)"      # ~/.zshrc
eval "$(jevify init bash)"     # ~/.bashrc
```

The snippet defines `,` as an alias for `jevify route` (`noglob jevify route` in zsh). It also defines a command-not-found handler that passes unknown commands of three or more words to `jevify route`, but only when `JEVIFY_CNF=1` is exported and no handler exists already. Routing prints a tool and starts no user command.
