# Configuration

Settings come from flags and environment variables. A flag beats its variable.
`jevify capabilities --json` describes the installed limits and configuration.

## Environment variables

| Variable | Default | Meaning |
|:---|:---|:---|
| `TYPESAFE_API_KEY` | unset | TypeSafe key; setting it selects TypeSafe |
| `TYPESAFE_API_KEY_FILE` | unset | Key file, read only when needed; use `/path/to/key` |
| `JEVIFY_BACKEND` | `typesafe` with a key, `classifier` otherwise | Force either backend |
| `JEVIFY_BASE_URL` | active backend's URL | Pinned HTTPS host; see below |
| `JEVIFY_MODEL` | `jev-1.13.0` on TypeSafe | Model or alias; classifier chooses its own and rejects overrides |
| `JEVIFY_THRESHOLD` | `0.5` | Decision threshold, calibrated per backend and task |
| `JEVIFY_CONCURRENCY` | `8` TypeSafe, `4` classifier | Parallel requests per round |
| `JEVIFY_DEADLINE` | `600` | Overall deadline in whole seconds; zero is a usage error |
| `JEVIFY_CACHE_DIR` | platform cache directory, `jevify` subdirectory | Answer cache and saved input; explicit setting also enables tool inventory storage |
| `JEVIFY_CONFIG_DIR` | unset | Explicit directory for user `kinds.jsonl`; no cwd or platform config reads |
| `JEVIFY_NO_CACHE` | unset | `1` bypasses answer caching; does not disable saved inputs or tool inventory |
| `JEVIFY_NO_SAVE` | unset | `1` disables the raw input copy for `why` and `filter` |
| `JEVIFY_INVENTORY_FILE` | unset | JSON array of `{name, summary}` replacing the PATH inventory for the `tool` kind |
| `JEVIFY_STATUS_FILE` | unset | `fill` execution status JSON; an unwritable path prevents execution |
| `JEVIFY_DECISION` | unset | `round_one` adds tournament candidate details to machine output |

`JEVIFY_BASE_URL` accepts only HTTPS at `api.typesafe.ai` for TypeSafe or `classifier.dev`
for classifier, on port 443. Host comparison ignores case. Trailing dots, other hosts or
ports, userinfo and malformed URLs are configuration errors before any request. Blank uses
the default. Local tests can use `localhost` or `127.0.0.1` with any scheme and port, without
userinfo. Inference, prewarm and health requests never follow redirects.

## Backends

No key or account is required for classifier.dev. Its free budget is **$0.50 per IP per UTC
day**, subject to **$100 per day across everyone** and **four concurrent requests**. There is
no fixed number of free calls: spending depends on request size. TypeSafe uses your credits.
`JEVIFY_BACKEND=typesafe` without a key exits 5. `jevify health` makes a small uncached
classification and reports quota or credit exhaustion through errors; it consumes budget.

| Limit | TypeSafe | classifier.dev |
|:---|---:|---:|
| options per question | 255 | 100 |
| selection window | 200 plus NONE | 99 plus NONE |
| input per request | 32,000 state tokens | 32,000 UTF-16 code units |
| questions per request | bounded by evidence | 20 dimensions |
| records per `filter` request | 20 in one shared state | up to 60, each judged alone |
| `meta.input_tokens` | reported total, or null if incomplete | reported total, or null if incomplete |

Classifier instructions are capped at 4,000 UTF-16 units, labels at 200, dimension names at
64, and the compact JSON of dimension definitions at 16,000. Escaping counts; an emoji can
occupy two UTF-16 units. jevify validates the constructed request before sending it.
Scores and thresholds do not imply equivalent calibration across backends.

`quota_exhausted` (exit 4) means depleted TypeSafe credits or a spent free budget; it is never
retried. A per-request spending limit is `input_too_large` (exit 6): narrow the request.
Transient failures can be retried within `JEVIFY_DEADLINE`; no request is sent before an
accepted `Retry-After` ends. Filter batches honor numeric waits through 60 seconds and refuse
longer waits. Expiry of the overall deadline is `api_deadline` (exit 4).

## Global flags

| Flag | Meaning |
|:---|:---|
| `--json` (alias `--robot`) | One JSON envelope on one stdout line, including usage errors |
| `-t, --threshold <0..1>` | Decision threshold |
| `--model <id>` | TypeSafe model or alias |
| `--no-cache` | Bypass answer caching |
| `--verbose` | Scores, requests, tokens and timing on stderr |
| `-V, --version` | Version |

`--no-save` belongs to `why` and `filter`. `filter -v` means inversion.
[Verbs](verbs.md) lists the flags for each command.

## Limits

Stdin is bounded at 64 MiB. `pick` and `filter` accept at most 20,000 distinct records;
this is an input ceiling, not a daily quota. Selection also has to fit two rounds:
`pick` and `pick --from` accept 9,801 candidates keyless and 20,000 on TypeSafe;
`fill` accepts 3,267 and 13,200 per marker. Ordered kinds report any omitted older candidates;
unordered overflow is `too_many` (exit 6).

## Where files live

- Cache base: `JEVIFY_CACHE_DIR`, otherwise `~/Library/Caches/jevify` on macOS or
  `$XDG_CACHE_HOME/jevify` / `~/.cache/jevify` on Linux.
- Answers: `answers/<2 hex>/<blake3>.json`, holding model names and decisions, not evidence.
  Redacted requests determine the hash. Entries are ignored after seven days; files remain.
- Saved inputs: `outputs/<blake3-16>.log`, written only by `why` and `filter`, containing raw
  bytes including secrets. A save prunes the store's own files older than seven days.
  `--no-save` and `JEVIFY_NO_SAVE=1` control this independently of the answer cache.
- Tool inventory: `inventory-<fingerprint>.json` holds PATH executable names and man-page
  summaries. The `tool` kind writes it when `JEVIFY_CACHE_DIR` is explicitly set, even with
  `--no-cache`; otherwise it rebuilds the inventory. Files have no expiry.
- User kinds: `kinds.jsonl` under explicit `JEVIFY_CONFIG_DIR`. No cwd or platform configuration
  directory is read. User recipes cannot shadow shipped kinds.

[Privacy](../../PRIVACY.md) gives the retention, redaction and file-withholding rules.
