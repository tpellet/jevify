# Latency

Uncached wall time per call on TypeSafe (model `jev-1.13.0`), jevify 0.14.1 release build, a
typical macOS dev machine on a home connection, measured 2026-10-01 between 12:00 and 12:30 UTC.
Every run sets `JEVIFY_NO_CACHE=1` and `JEVIFY_NO_SAVE=1`. Requests, retries and failures come
from `meta.requests` and `meta.telemetry` of the `--json` envelope; wall time is the whole
process, from spawn to exit, stdin included.

## Calls

Direct to `https://api.typesafe.ai`, 90 runs:

| Call | Runs | p50 s | p95 s | max s | Requests | Retries | Failed | Correct |
|:---|---:|---:|---:|---:|---:|---:|---:|---:|
| `why < docs/demo/build.log` | 20 | 0.51 | 0.60 | 0.81 | 8 | 0 | 0 | 20/20 |
| `why < evals/why/go-01.log` | 10 | 0.42 | 0.53 | 0.53 | 2 | 0 | 0 | 10/10 |
| `why < evals/why/cargo-01.log` | 10 | 0.41 | 0.50 | 0.50 | 3 | 0 | 0 | 10/10 |
| `why < evals/why/pytest-01.log` | 10 | 0.45 | 0.55 | 0.55 | 3 | 0 | 0 | 10/10 |
| `is 'asks for a refund' < docs/demo/mail.txt` | 20 | 0.23 | 0.28 | 0.30 | 1 | 0 | 0 | 20/20 |
| `fill --dry-run` commit (`docs/demo/examples.sh`) | 10 | 0.24 | 0.31 | 0.31 | 1 | 0 | 0 | 10/10 |
| `pick --files` (`docs/demo/examples.sh`) | 10 | 0.48 | 0.52 | 0.52 | 4 | 0 | 0 | 10/10 |

Correct means: `why` names the error line on `build.log` and a line inside the gold range of
each eval case; `is` exits 0; `fill` resolves `317cbf7`; `pick --files` returns `src/cli.rs`
first. The request count is fixed per input.

## Requests

The same calls through a local timing proxy (`JEVIFY_BASE_URL=http://127.0.0.1:<port>`), which
logs each backend request's duration, plus a soak of 60 more `why` and 60 more `is` runs:

| Call | Runs | p50 s | p95 s | max s | Requests | Request p50 s | p95 s | max s |
|:---|---:|---:|---:|---:|---:|---:|---:|---:|
| `why < build.log` | 20 + 60 | 0.50 | 0.62 | 0.74 | 640 | 0.26 | 0.32 | 0.49 |
| `why` eval cases | 30 | 0.41 | 0.50 | 0.50 | 80 | 0.20 | 0.28 | 0.34 |
| `is` | 20 + 60 | 0.19 | 0.25 | 0.40 | 80 | 0.18 | 0.24 | 0.39 |
| `fill --dry-run` | 10 | 0.19 | 0.21 | 0.21 | 10 | 0.18 | 0.20 | 0.20 |
| `pick --files` | 10 | 0.41 | 0.51 | 0.51 | 40 | 0.20 | 0.24 | 0.25 |

850 backend requests, all HTTP 200, none retried; the slowest took 0.49 s. A call costs about
two request times (its rounds) plus 0.1-0.2 s of process start and input reading.

## Slow runs

None in 330 runs: the slowest call took 0.81 s. The 10-22 s `why` runs and the 9.7 s `is` run
a reviewer reported, all with zero retries, did not recur, so their cause is unattributed.
With zero retries and a sixty-second response timeout, such a run is one or more backend
requests that answered slowly, not a retry loop. Measuring it needs per-request timing at the
moment it happens; `meta` reports counts but no per-request durations.

Keyless (classifier.dev) runs were not made, to keep its free per-IP budget.
