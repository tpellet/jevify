# Triage: classifying a failed CI run from the line `why` points at

Question: once `why` has pointed at the line, can `fill`'s `@{one:…}` marker put the run into a
class such as `compile | test | flaky | infra` without a new verb? The GitHub Action's `classes`
input runs exactly the mechanism measured here.

Measurement: **2026-10-02**, TypeSafe `jev-1.13.0`, release build `jevify 0.14.2`, threshold
0.5, `JEVIFY_NO_CACHE=1`, the 34 scored logs of [why-ci.md](why-ci.md) (raw logs in the external
cache; the OpenTofu case stays out as there).

## Classes and labels

One labeller assigns one class per log before any run, from the gold diagnostic of
[cases.jsonl](../evals/why-ci/cases.jsonl); the labels and a one-line reason are in
[why-triage-labels.tsv](why-triage-labels.tsv).

- **compile**: the code or its dependencies fail to build, type-check, lint or resolve, so no
  test runs (6 logs).
- **test**: a test runs and fails deterministically on an assertion, exception or output
  mismatch (6).
- **flaky**: a timeout, retry limit or resource bound fails the run and a rerun could pass (8).
- **infra**: the CI environment fails: credentials, permissions, network, downloads, workflow or
  bot configuration, a security scanner (14).

Two labels are judgment calls: Redis (a memory bound measured against a limit: flaky, though it
is written as an assertion) and pytest-dev (NumPy failing to import on the PyPy runner: infra,
though nothing in CI configuration changed). Both count against jevify below.

## Method

Each log goes through the Action's path, one pass, no retries:

```sh
jevify why --no-save < run.log > why.txt          # the human output: the line plus 3 lines each side
jevify fill --dry-run --json --context why.txt -- echo '@{one:compile|test|flaky|infra:QUESTION}'
```

Two questions. *Short*: `the kind of failure in this CI log`. *Defined*: the four class
definitions above in one sentence (`compile: the code or its dependencies fail to build,
type-check, lint or resolve. test: … flaky: … infra: …`), which is the default
`classes-question` of the Action. A third configuration passes the whole raw log as `--context`
instead of `why`'s output.

Correct means the chosen option equals the label. Abstain is exit 3 (`no_match`, `ambiguous`,
`insufficient_evidence`). Wrong is any other option. Backend errors are NOT RUN inside the scored
total and listed as such.

## Totals

| Context | Question | N | Correct | Abstain | Wrong | NOT RUN | fill wall p50 / p95 (s) |
|---|---|---:|---:|---:|---:|---:|---:|
| `why` output (line + 3 lines each side) | defined | 34 | **26 (76.5%)** | 4 (11.8%) | 4 (11.8%) | 0 | 0.22 / 0.28 |
| `why` output | short | 34 | 11 (32.4%) | 11 (32.4%) | 12 (35.3%) | 0 | 0.22 / 0.31 |
| whole raw log | defined | 34 | 11 (32.4%) | 20 (58.8%) | 1 (2.9%) | 2 | 0.24 / 0.46 |

The `why` call itself adds a median 0.50 s (max 2.17 s) on the same machine; the Action pays it
anyway.

The whole-log configuration abstains with `insufficient_evidence` on the 19 logs over the
32,000-character evidence budget and errors (`api_protocol`) on two more; it answers only
small logs. The short question collapses to `test` or `no_match`: every security-scanner log
(Helm, Prometheus, golangci-lint) and every timeout comes out wrong or unanswered. Definitions in
the question are what make the four classes work, which is why the Action's default question
carries them.

## Per-run results, `why` output + defined question

| Repository / run | Label | `why` line | Class | p | Result |
|---|---|---:|---|---:|---|
| starship/starship / 33383062728 | test | 4672 (hit) | test | 1.00 | correct |
| rust-lang/rust-analyzer / 36283748467 | compile | 361 (hit) | compile | 1.00 | correct |
| clap-rs/clap / 36287138011 | flaky | 770 (hit) | flaky | 0.96 | correct |
| serde-rs/serde / 35677316906 | test | 516 (wrong) | compile | 0.73 | wrong |
| rustls/rustls / 36343947828 | compile | 899 (hit) | compile | 1.00 | correct |
| zed-industries/zed / 36387759021 | infra | 172 (hit) | infra | 1.00 | correct |
| npm/cli / 35779328486 | infra | 336 (hit) | infra | 0.95 | correct |
| eslint/eslint / 33851901321 | flaky | 284 (hit) | flaky | 0.95 | correct |
| webpack/webpack / 34234731508 | compile | 1027 (hit) | compile | 0.99 | correct |
| vercel/next.js / 36391014017 | infra | 522 (hit) | infra | 0.98 | correct |
| sveltejs/svelte / 36087475151 | flaky | 350 (hit) | flaky | 0.92 | correct |
| pydantic/pydantic / 35342760119 | infra | 1056 (hit) | test | 0.74 | wrong |
| pytest-dev/pytest / 35831999592 | infra | 730 (hit) | — | — | abstain (ambiguous) |
| pallets/flask / 34252544596 | compile | 269 (wrong) | — | — | abstain (ambiguous) |
| scikit-learn/scikit-learn / 34077229123 | test | 3487 (hit) | test | 0.99 | correct |
| home-assistant/core / 34921188397 | compile | 453 (hit) | compile | 0.99 | correct |
| psf/requests / 32792099048 | infra | 45 (hit) | infra | 1.00 | correct |
| golangci/golangci-lint / 34167961970 | infra | 260 (hit) | infra | 0.99 | correct |
| caddyserver/caddy / 29171913482 | infra | 4632 (hit) | — | — | abstain (ambiguous) |
| grafana/loki / 36173147923 | flaky | 1227 (hit) | flaky | 0.97 | correct |
| tailscale/tailscale / 36173811309 | test | 385 (wrong) | flaky | 0.98 | wrong |
| helm/helm / 31059356441 | infra | 456 (hit) | infra | 0.98 | correct |
| helm/helm / 30963144784 | infra | 471 (block) | infra | 0.98 | correct |
| helm/helm / 30774399212 | infra | 459 (hit) | infra | 0.97 | correct |
| docker/compose / 34228656738 | test | 51842 (wrong) | test | 0.83 | correct |
| aquasecurity/trivy / 33768370418 | compile | 8980 (hit) | compile | 0.98 | correct |
| neovim/neovim / 33110233061 | test | 8982 (hit) | test | 1.00 | correct |
| neovim/neovim / 33084302624 | flaky | 10776 (hit) | — | — | abstain (ambiguous) |
| redis/redis / 34420266410 | flaky | 8631 (hit) | test | 0.95 | wrong |
| laravel/framework / 34135873264 | infra | 937 (hit) | infra | 0.80 | correct |
| ruby/ruby / 36397536870 | flaky | 3958 (hit) | flaky | 0.93 | correct |
| pnpm/pnpm / 34207109750 | flaky | 3292 (hit) | flaky | 0.88 | correct |
| prometheus/prometheus / 29892452615 | infra | 233 (hit) | infra | 0.96 | correct |
| prometheus/prometheus / 29868709147 | infra | 226 (hit) | infra | 0.95 | correct |

`why` hit or wrong follows the gold lines of why-ci.md; this run repeats the same four misses.
Two answers moved inside their diagnostic block since the 2026-09-28 run: Helm 471 (`Your code is
affected by 1 vulnerability`, two lines past the explicit gold set, counted as a hit here because
the class question sees the same govulncheck block) and Prometheus 233 (inside the gold set). On
the 30 runs where `why` hits: 25 correct, 3 abstain, 2 wrong.
On the 4 `why` misses: 1 correct (Compose: the wrong line is still a test assertion), 1 abstain,
2 wrong (Serde, Tailscale: the class follows the wrong line, a compiler error inside snapshot
output and a broken pipe).

## Misses

- Pydantic: `could not find task "generate_dev_jwks"` reads as a test failure; the task runner
  is CI configuration.
- Redis: a memory assertion against a bound; the label says flaky, the model says test. The
  label is the debatable one.
- Serde and Tailscale: inherited from `why`'s wrong line.
- Abstentions (pytest-dev, Flask, Caddy, Neovim TUI) are all `ambiguous`: two classes within the
  rival ratio, for instance infra against compile on a NumPy import failure under PyPy.

## Limits

One labeller, one pass, a convenience sample with repeated failures in three repositories, and
two labels that could go either way. The classes are four words with one-sentence definitions;
other class sets need their own question and their own measurement. The context is seven lines:
a class that needs the step name or an earlier line (which job, which runner) is out of reach,
which is where Pydantic and the abstentions sit. Fresh `why` answers can differ from the
2026-09-28 run in rank within a gold range. No held-out set; this is a development-set
measurement of the mechanism the Action runs.
