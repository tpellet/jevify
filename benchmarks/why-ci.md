# jevify why on failed GitHub Actions runs

Measurement: **2026-09-28**, TypeSafe `jev-1.13.0`, release build `jevify 0.14.0`, threshold 0.5.

**On 34 failed CI runs from 30 public repositories, jevify why points at the root cause first in 30, abstains in 0, and points at a wrong line in 4; an agent reads 99.76% fewer tokens when consuming its JSON stdout instead of the complete failed-job logs. Median latency is 0.87 s on a loaded machine.**

“Root cause” means the first explanatory diagnostic block in the printed log, not a verified underlying software defect. Token reduction measures the output payload alone, not successful agent task completion or total inference cost.

## Method

The manifest contains 35 failed runs from 31 public repositories across six repository ecosystems. One ambiguous OpenTofu output-diff case is NOT RUN; 34 runs enter the scored totals. Repository ecosystem describes the project, not necessarily the failing job: for example, Zed's scheduled Python issue script belongs to the Rust repository group.

Selection is purposive, not random. Public repository visibility and default branch are checked with `gh repo view`; failed runs are queried with `gh run list -R OWNER/REPO -b BRANCH -s failure`, with push-event queries where branch-name searches return fork pull requests. The newest available eligible candidates in these queries supply the sample; additional recent failures in Helm, Prometheus, and Neovim fill ecosystem coverage. Run metadata verifies failure, the default-branch name, and that the head repository is the repository itself. Cancellations, fork PRs, runner shutdowns, disk exhaustion, service outages, and unavailable logs are excluded before inference. Workflow configuration errors, dependency resolution errors, permission failures, and test timeouts remain eligible.

The exact stdout of `gh run view ID --log-failed -R OWNER/REPO` is the input: all failed-job output, with job, step, and timestamp prefixes intact. Raw logs remain outside the repository. SHA-256 and line counts pin each input; a mismatched log or an unavailable/expired fetch is NOT RUN. An existing hash-matching cache remains usable without refetching. Line numbers are one-based physical lines, including blank lines.

One labeller assigns the gold diagnostic lines and rationale before any benchmark inference, without consulting pilot prediction files. Generic exit-code messages, test-count summaries, warnings from passing steps, and later repetitions receive no credit. Separate jobs can fail for different reasons; the first diagnostic in printed order is the target. The eligible Pydantic pilot log is byte-identical to a fresh fetch and supplies one case; the other pilot cases are non-default-branch runs or superseded by newer eligible failures. The pilot subset is therefore not a held-out test.

Each unambiguous case receives exactly these two invocations, without retries:

```sh
jevify why --json --no-cache
jevify why -n 3 --json --no-cache
```

The same raw bytes also pass through `tail -n 50` and `grep -n -iE "error|fail|panic" | tail -n 5`. The runner uses Python standard library subprocesses, not a shell, for these pipelines. All 68 jevify invocations report TypeSafe, model `jev-1.13.0`, threshold 0.5, and zero answer-cache hits. Saved-input storage is disabled. No tool errors enter the scored set. Labels and log bytes are fixed across development runs; this is a development-set measurement, not a held-out evaluation.

Hit@1 checks the first selected line. Hit@3 checks the first three selected lines from the separate `-n 3` invocation. The two invocations can rank differently. Wrong-first-line rate counts nonempty selections outside gold; abstention requires exit 3 and no causes. Usage, authentication, availability, malformed responses, and timeouts are NOT RUN, not abstentions. Ambiguous cases are excluded before inference.

For baselines, window hit asks whether **any** returned line intersects gold: all 50 tail lines or all five grep lines. Their first/first-three positions are chronological, not relevance-ranked; window hit is the useful retrieval comparison. Neither baseline identifies a root-cause line.

Tokens in and out are UTF-8 bytes divided by four. Output includes the **entire JSON envelope**, not only the cause text; baseline output includes exactly its printed bytes. Reduction is `1 - sum(output bytes) / sum(input bytes)`, without integer rounding before aggregation. Wall time covers a subprocess, stdin handling, inference, and output; fetching is excluded. Calls run serially. p50 is the median and p95 is the nearest-rank 95th percentile.

Latency is measured on a loaded machine with tests and other workers running concurrently. These timings are not an isolated latency comparison.

## Totals

| Method | N | hit@1 | hit@3 | Window hit | Abstain | Wrong first line | Tokens in → out | Fewer tokens | Wall p50 / p95 (s) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| why | 34 | 30/34 (88.2%) | 30/34 (88.2%) | 30/34 (88.2%) | 0/34 (0.0%) | 4/34 (11.8%) | 12445602 → 30279 | 99.76% | 0.874 / 2.875 |
| why -n 3 | 34 | 30/34 (88.2%) | 31/34 (91.2%) | 31/34 (91.2%) | 0/34 (0.0%) | 4/34 (11.8%) | 12445602 → 45158 | 99.64% | 0.942 / 3.516 |
| tail -n 50 | 34 | 2/34 (5.9%) | 3/34 (8.8%) | 10/34 (29.4%) | 0/34 (0.0%) | 32/34 (94.1%) | 12445602 → 60108 | 99.52% | 0.007 / 0.124 |
| grep \| tail -n 5 | 34 | 8/34 (23.5%) | 11/34 (32.4%) | 11/34 (32.4%) | 0/34 (0.0%) | 26/34 (76.5%) | 12445602 → 6940 | 99.94% | 0.013 / 0.297 |

The top-1 headline is **88.2% hit, 0% abstain, 11.8% wrong**. Top-3 reaches **91.2%**. Tail and grep retain a gold diagnostic somewhere in their output in **29.4%** and **32.4%** of cases respectively.

Median per-case top-1 payload reduction is **97.75%**. The aggregate 99.76% figure is weighted toward large logs. An oracle calculation that adds the full input back for every wrong top-1 answer reduces the saving to **81.98%**; this assumes perfect error detection and is not a measured agent recovery workflow. Excluding the reused pilot gives 29/33 top-1 hits (87.9%).

The 68 CLI invocations make **896 backend requests**, which report **9,060,489 input tokens** and **1,536,443 output tokens** in total. The headline token figure describes what a downstream agent reads, not these backend tokens or their cost.

## Ecosystem split

| Repository ecosystem | Sampled | Scored | Top-1 hit | Abstain | Wrong |
|---|---:|---:|---:|---:|---:|
| Rust / cargo projects | 6 | 6 | 5 | 0 | 1 |
| JavaScript / npm / pnpm projects | 6 | 6 | 6 | 0 | 0 |
| Python projects | 6 | 6 | 5 | 0 | 1 |
| Go projects | 6 | 6 | 5 | 0 | 1 |
| Docker / infrastructure projects | 6 | 5 | 4 | 0 | 1 |
| Other: C, Lua, PHP, Ruby projects | 5 | 5 | 5 | 0 | 0 |
| **Total** | **35** | **34** | **30** | **0** | **4** |

## Per-run results

Gold ranges marked * abbreviate an explicit set of line numbers; only the numbers in [cases.jsonl](../evals/why-ci/cases.jsonl) count. Tail/grep columns are whole-window hits.

The selection columns list every returned cause. Multijob logs return one cause per failed job, so these lists can exceed three lines; hit@1 and hit@3 still score only the first one or three lines respectively.

| Repository / run | Ecosystem | Log lines | Gold lines | why → result | -n 3 → hit@3 | Tail hit | Grep hit |
|---|---|---:|---|---|---|---|---|
| [starship/starship / 33383062728](https://github.com/starship/starship/actions/runs/33383062728) | rust | 4,735 | 4671–4674* | 4672 → hit | 4672, 4673, 4671 → hit | no | no |
| [rust-lang/rust-analyzer / 36283748467](https://github.com/rust-lang/rust-analyzer/actions/runs/36283748467) | rust | 420 | 361–369* | 361 → hit | 361 → hit | no | no |
| [clap-rs/clap / 36287138011](https://github.com/clap-rs/clap/actions/runs/36287138011) | rust | 791 | 770–771* | 770 → hit | 770, 769 → hit | yes | yes |
| [serde-rs/serde / 35677316906](https://github.com/serde-rs/serde/actions/runs/35677316906) | rust | 980 | 512 | 516 → wrong | 516, 512, 524 → hit | no | no |
| [rustls/rustls / 36343947828](https://github.com/rustls/rustls/actions/runs/36343947828) | rust | 962 | 899 | 899 → hit | 899, 903, 905 → hit | no | no |
| [zed-industries/zed / 36387759021](https://github.com/zed-industries/zed/actions/runs/36387759021) | rust | 209 | 172 | 172 → hit | 172 → hit | yes | yes |
| [npm/cli / 35779328486](https://github.com/npm/cli/actions/runs/35779328486) | js | 93,759 | 336 | 336, 661, 54318, 93135, 93252, 93381, 93511, 93626, 93754 → hit | 336, 661, 54318, 93135, 93252, 93381, 93511, 93626, 93754 → hit | no | no |
| [eslint/eslint / 33851901321](https://github.com/eslint/eslint/actions/runs/33851901321) | js | 370 | 284 | 284 → hit | 284, 186, 283 → hit | no | yes |
| [webpack/webpack / 34234731508](https://github.com/webpack/webpack/actions/runs/34234731508) | js | 1,068 | 1027–1030* | 1027 → hit | 1027 → hit | yes | no |
| [vercel/next.js / 36391014017](https://github.com/vercel/next.js/actions/runs/36391014017) | js | 1,199 | 522–524* | 522, 1121 → hit | 522, 1121 → hit | no | no |
| [sveltejs/svelte / 36087475151](https://github.com/sveltejs/svelte/actions/runs/36087475151) | js | 405 | 350 | 350 → hit | 350, 344, 349 → hit | no | yes |
| [pydantic/pydantic / 35342760119](https://github.com/pydantic/pydantic/actions/runs/35342760119) | python | 10,252 | 1056 | 1056, 2267, 3476, 4694, 5534, 6883, 8231, 9592 → hit | 1056, 2267, 3476, 4694, 5534, 6883, 8231, 9592 → hit | no | no |
| [pytest-dev/pytest / 35831999592](https://github.com/pytest-dev/pytest/actions/runs/35831999592) | python | 844 | 730 | 730 → hit | 730 → hit | no | no |
| [pallets/flask / 34252544596](https://github.com/pallets/flask/actions/runs/34252544596) | python | 298 | 278 | 269 → wrong | 269, 173, 175 → miss | yes | no |
| [scikit-learn/scikit-learn / 34077229123](https://github.com/scikit-learn/scikit-learn/actions/runs/34077229123) | python | 14,063 | 3487 | 3487 → hit | 3487, 14060 → hit | no | no |
| [home-assistant/core / 34921188397](https://github.com/home-assistant/core/actions/runs/34921188397) | python | 550 | 453–461* | 453 → hit | 453, 454, 459 → hit | no | no |
| [psf/requests / 32792099048](https://github.com/psf/requests/actions/runs/32792099048) | python | 46 | 45 | 45 → hit | 45, 32 → hit | yes | yes |
| [golangci/golangci-lint / 34167961970](https://github.com/golangci/golangci-lint/actions/runs/34167961970) | go | 281 | 260 | 260 → hit | 260 → hit | yes | no |
| [caddyserver/caddy / 29171913482](https://github.com/caddyserver/caddy/actions/runs/29171913482) | go | 5,640 | 4632 | 4632 → hit | 4632, 5625 → hit | no | no |
| [grafana/loki / 36173147923](https://github.com/grafana/loki/actions/runs/36173147923) | go | 5,655 | 1227–1230* | 1227 → hit | 1227 → hit | no | no |
| [tailscale/tailscale / 36173811309](https://github.com/tailscale/tailscale/actions/runs/36173811309) | go | 453 | 406 | 385 → wrong | 385, 386 → miss | yes | no |
| [helm/helm / 31059356441](https://github.com/helm/helm/actions/runs/31059356441) | infra | 529 | 456–466* | 456 → hit | 456, 468, 457 → hit | no | yes |
| [helm/helm / 30963144784](https://github.com/helm/helm/actions/runs/30963144784) | infra | 532 | 459–469* | 459 → hit | 459, 471, 460 → hit | no | yes |
| [helm/helm / 30774399212](https://github.com/helm/helm/actions/runs/30774399212) | infra | 532 | 459–469* | 459 → hit | 459, 471, 460 → hit | no | yes |
| [docker/compose / 34228656738](https://github.com/docker/compose/actions/runs/34228656738) | infra | 56,516 | 54784 | 51842 → wrong | 51842, 56020, 56182 → miss | no | no |
| [aquasecurity/trivy / 33768370418](https://github.com/aquasecurity/trivy/actions/runs/33768370418) | infra | 9,774 | 8980 | 8980 → hit | 8980, 8976, 9758 → hit | no | no |
| [neovim/neovim / 33110233061](https://github.com/neovim/neovim/actions/runs/33110233061) | other | 13,065 | 8982–8986* | 8982 → hit | 8982, 12959, 8981 → hit | no | no |
| [neovim/neovim / 33084302624](https://github.com/neovim/neovim/actions/runs/33084302624) | other | 14,161 | 10775–10786* | 10776 → hit | 10776, 10774, 14072 → hit | no | no |
| [redis/redis / 34420266410](https://github.com/redis/redis/actions/runs/34420266410) | other | 31,479 | 8631 | 8631, 10787, 30550 → hit | 8631, 10787, 30550 → hit | no | no |
| [laravel/framework / 34135873264](https://github.com/laravel/framework/actions/runs/34135873264) | other | 973 | 937 | 937 → hit | 937, 190 → hit | yes | yes |
| [ruby/ruby / 36397536870](https://github.com/ruby/ruby/actions/runs/36397536870) | other | 4,507 | 3958–3959* | 3958 → hit | 3958, 3967, 557 → hit | no | no |
| [pnpm/pnpm / 34207109750](https://github.com/pnpm/pnpm/actions/runs/34207109750) | js | 3,565 | 3292 | 3292 → hit | 3292 → hit | no | no |
| [opentofu/opentofu / 32139669958](https://github.com/opentofu/opentofu/actions/runs/32139669958) | infra | 653 | 430 | NOT RUN: ambiguous | — | — | — |
| [prometheus/prometheus / 29892452615](https://github.com/prometheus/prometheus/actions/runs/29892452615) | go | 277 | 226–236* | 226 → hit | 226, 233, 227 → hit | yes | yes |
| [prometheus/prometheus / 29868709147](https://github.com/prometheus/prometheus/actions/runs/29868709147) | go | 277 | 226–236* | 226 → hit | 226, 233, 227 → hit | yes | yes |

## Misses and limits

- Serde selects a compiler error printed inside the expected UI-test output, rather than the diagnostic-snapshot mismatch. Top-3 includes the gold mismatch line.
- Flask selects the traceback's import-error header rather than the fatal ImmutableDict deprecation diagnostic.
- Tailscale selects a broken-pipe message rather than the peer-relay assertion diagnostic.
- Docker Compose selects an image-store diagnostic rather than the container-state assertion.

One labeller, one pass per configuration, a convenience sample, repeated failures within three repositories, and a previously exercised pilot limit generalization. There is no inter-rater agreement study, randomized holdout, repeated-run stability estimate, or measured end-to-end agent task success. The sample includes workflow maintenance and configuration failures, not just compiler and test failures. Defaults and backend calibration are specific to the measured binary and TypeSafe model. A zero observed abstention rate does not establish reliable abstention on other logs.

Exact-first-block scoring rejects semantically related later evidence. Multijob logs use printed order, not global timestamp order. The OpenTofu case has an elided output diff and remains ambiguous. GitHub retention, reruns, and changes in gh rendering can prevent reproduction; the runner reports NOT RUN instead of silently replacing pinned bytes. The newest-run query is time-sensitive and the sample is not an exhaustive census of public Actions failures.

## Reproduction and verification

Use [the runner instructions](../evals/why-ci/README.md). The manifest supplies run IDs, workflows, failed-job names, branch/event provenance, fetch date, raw hashes, line counts, gold text, rationales, and ambiguity flags. Raw logs, complete response envelopes, timings, and collection metadata stay in the external cache.

Measured implementation: `3582380` (`why` CI-log selection), release build reporting `jevify 0.14.0`. The measurement metadata does not record a binary SHA-256.
Manifest SHA-256: `5a09e4218efa342ec062710fea0da3d4f400842a6dce020a2aa98dd29ff6a9a3`.

The runner validates raw hashes and physical line counts before inference. The 68 saved response envelopes report the expected backend, model, threshold and cache state. Raw logs and response envelopes remain in the external cache.
