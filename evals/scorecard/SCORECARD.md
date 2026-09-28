# Verb scorecard

Measured 2026-09-28 on a macOS dev machine with `jevify 0.13.0` (`~/.cargo/bin/jevify`, sha256 `9e2cfa88e867ce1a`), every call with `JEVIFY_NO_CACHE=1`. Keyless backend: classifier.dev, answering model `jev-1.13.0`. TypeSafe backend: answering model `jev-1.13.0`. Thin arm: `scripts/thinjev/jev` at `f34ee5a`, keyless, answering model `jev-1.13.0`.

The thin arm is one keyless call per question with the raw candidate lines as options and the top option as its answer: no lister, no excerpt or patch, no second round, no threshold. Its input is cut the way a shell user cuts it past the wrapper's 100 options: `tail -n 100` of a log, `head -n 100` of any other list, lines clipped to 200 characters and kept once. A lister case gives it `git ls-files`, the directories in it, or `git log --format='%h %s'`; a `route` case gives it the tool names. `thin+threshold` abstains where the top probability is below the threshold jevify reports for the same case. The thin arm has no TypeSafe column: the wrapper is keyless.

Sets: `evals/validation` (153 cases, both splits) and `evals/holdout` (103 cases) against their gold; `evals/variance` (the holdout subset, 5 cold reruns and 8 orders); `evals/commit-subjects` (20 commit descriptions whose subject lies). A `filter` or `label` case is one record; jevify answers a run of records in one call, the thin arm asks once per record. Latency is wall time per call, process start included; requests per case are HTTP requests to the backend.

**Columns.** accuracy on answered: right over decided. coverage: decided over scored. false actions: a decision the gold calls wrong or says not to make, over scored. abstained: no decision, with how many of those the gold calls right. right of all: right decisions over scored.

## Per verb, both sets

| verb | arm | answering model | n | accuracy on answered | coverage | false actions | abstained | right of all | p50 ms | p95 ms | requests per case |
|:--|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| fill | jevify keyless | jev-1.13.0 | 33 | 92% (24/26) | 79% | 6% (2) | 21% (6 right) | 73% | 1089 | 3497 | 3.88 |
| fill | jevify TypeSafe | jev-1.13.0 | 33 | 92% (24/26) | 79% | 6% (2) | 21% (6 right) | 73% | 643 | 1027 | 2.76 |
| fill | thin | jev-1.13.0 | 33 | 58% (19/33) | 100% | 42% (14) | 0% (0 right) | 58% | 550 | 1268 | 1.00 |
| fill | thin+threshold | jev-1.13.0 | 33 | 90% (19/21) | 64% | 6% (2) | 36% (7 right) | 58% | 550 | 1268 | 1.00 |
| filter | jevify keyless | jev-1.13.0 | 60 | 98% (41/42) | 70% | 2% (1) | 30% (5 right) | 68% | 551 | 966 | 0.13 |
| filter | jevify TypeSafe | jev-1.13.0 | 60 | 98% (45/46) | 77% | 2% (1) | 23% (5 right) | 75% | 387 | 442 | 0.13 |
| filter | thin | jev-1.13.0 | 60 | 83% (50/60) | 100% | 17% (10) | 0% (0 right) | 83% | 506 | 972 | 1.00 |
| filter | thin+threshold | jev-1.13.0 | 60 | 83% (50/60) | 100% | 17% (10) | 0% (0 right) | 83% | 506 | 972 | 1.00 |
| is | jevify keyless | jev-1.13.0 | 15 | 87% (13/15) | 100% | 13% (2) | 0% (0 right) | 87% | 545 | 799 | 1.00 |
| is | jevify TypeSafe | jev-1.13.0 | 15 | 87% (13/15) | 100% | 13% (2) | 0% (0 right) | 87% | 358 | 384 | 1.00 |
| is | thin | jev-1.13.0 | 15 | 87% (13/15) | 100% | 13% (2) | 0% (0 right) | 87% | 504 | 694 | 1.00 |
| is | thin+threshold | jev-1.13.0 | 15 | 87% (13/15) | 100% | 13% (2) | 0% (0 right) | 87% | 504 | 694 | 1.00 |
| label | jevify keyless | jev-1.13.0 | 32 | 85% (23/27) | 84% | 12% (4) | 16% (5 right) | 72% | 1150 | 1236 | 0.12 |
| label | jevify TypeSafe | jev-1.13.0 | 32 | 85% (22/26) | 81% | 12% (4) | 19% (5 right) | 69% | 375 | 380 | 0.12 |
| label | thin | jev-1.13.0 | 32 | 72% (23/32) | 100% | 28% (9) | 0% (0 right) | 72% | 529 | 1155 | 1.00 |
| label | thin+threshold | jev-1.13.0 | 32 | 72% (23/32) | 100% | 28% (9) | 0% (0 right) | 72% | 529 | 1155 | 1.00 |
| pick | jevify keyless | jev-1.13.0 | 32 | 89% (24/27) | 84% | 9% (3) | 16% (5 right) | 75% | 571 | 938 | 1.00 |
| pick | jevify TypeSafe | jev-1.13.0 | 32 | 89% (24/27) | 84% | 9% (3) | 16% (5 right) | 75% | 382 | 517 | 1.00 |
| pick | thin | jev-1.13.0 | 32 | 72% (23/32) | 100% | 28% (9) | 0% (0 right) | 72% | 541 | 1198 | 1.00 |
| pick | thin+threshold | jev-1.13.0 | 32 | 77% (23/30) | 94% | 22% (7) | 6% (2 right) | 72% | 541 | 1198 | 1.00 |
| pick --from | jevify keyless | jev-1.13.0 | 9 | 100% (6/6) | 67% | 0% (0) | 33% (2 right) | 67% | 3055 | 4848 | 15.89 |
| pick --from | jevify TypeSafe | jev-1.13.0 | 9 | 100% (7/7) | 78% | 0% (0) | 22% (2 right) | 78% | 962 | 1286 | 8.78 |
| pick --from | thin | jev-1.13.0 | 9 | 44% (4/9) | 100% | 56% (5) | 0% (0 right) | 44% | 580 | 1194 | 1.00 |
| pick --from | thin+threshold | jev-1.13.0 | 9 | 67% (4/6) | 67% | 22% (2) | 33% (2 right) | 44% | 580 | 1194 | 1.00 |
| route | jevify keyless | jev-1.13.0 | 32 | 92% (22/24) | 75% | 6% (2) | 25% (4 right) | 69% | 1386 | 4786 | 9.12 |
| route | jevify TypeSafe | jev-1.13.0 | 32 | 91% (21/23) | 72% | 6% (2) | 28% (4 right) | 66% | 946 | 2708 | 5.38 |
| route | thin | jev-1.13.0 | 32 | 50% (16/32) | 100% | 50% (16) | 0% (0 right) | 50% | 516 | 1095 | 1.00 |
| route | thin+threshold | jev-1.13.0 | 32 | 73% (16/22) | 69% | 19% (6) | 31% (3 right) | 50% | 516 | 1095 | 1.00 |
| why | jevify keyless | jev-1.13.0 | 43 | 89% (34/38) | 88% | 9% (4) | 12% (5 right) | 79% | 1485 | 2082 | 2.81 |
| why | jevify TypeSafe | jev-1.13.0 | 43 | 92% (34/37) | 86% | 7% (3) | 14% (6 right) | 79% | 648 | 895 | 2.30 |
| why | thin | jev-1.13.0 | 43 | 47% (20/43) | 100% | 53% (23) | 0% (0 right) | 47% | 1061 | 1402 | 1.00 |
| why | thin+threshold | jev-1.13.0 | 43 | 46% (6/13) | 30% | 16% (7) | 70% (4 right) | 14% | 1061 | 1402 | 1.00 |
| **all verbs** | jevify keyless | jev-1.13.0 | 256 | 91% (187/205) | 80% | 7% (18) | 20% (32 right) | 73% | 1110 | 4401 | 2.90 |
| **all verbs** | jevify TypeSafe | jev-1.13.0 | 256 | 92% (190/207) | 81% | 7% (17) | 19% (33 right) | 74% | 634 | 2209 | 1.95 |
| **all verbs** | thin | jev-1.13.0 | 256 | 66% (168/256) | 100% | 34% (88) | 0% (0 right) | 66% | 537 | 1239 | 1.00 |
| **all verbs** | thin+threshold | jev-1.13.0 | 256 | 77% (154/199) | 78% | 18% (45) | 22% (18 right) | 60% | 537 | 1239 | 1.00 |

## jevify minus thin, keyless

Same backend, same model, same items. Differences are in points (percentage of scored items); positive means jevify is higher.

| verb | n | right of all | coverage | false-action rate | vs thin+threshold: right of all | false-action rate |
|:--|--:|--:|--:|--:|--:|--:|
| fill | 33 | +15 | -21 | -36 | +15 | +0 |
| filter | 60 | -15 | -30 | -15 | -15 | -15 |
| is | 15 | +0 | +0 | +0 | +0 | +0 |
| label | 32 | +0 | -16 | -16 | +0 | -16 |
| pick | 32 | +3 | -16 | -19 | +3 | -12 |
| pick --from | 9 | +22 | -33 | -56 | +22 | -22 |
| route | 32 | +19 | -25 | -44 | +19 | -12 |
| why | 43 | +33 | -12 | -44 | +65 | -7 |

## Where the gap comes from

Every item where jevify keyless and thin reach a different outcome, grouped by the jevify mechanism the difference traces to. `+` is an item jevify gets right or correctly declines and thin does not; `-` the reverse; `~` both wrong in different ways. `(100/292)`: the thin arm's options, out of the input's 292 non-blank lines; `gold cut off`: the right answer was not among them.

**fill**

- listers + chunking (thin read a 100-line cut of the listing): 6 for jevify, 0 for thin, 0 both wrong — `+fill-cal-01 (100/2834, gold cut off)`, `+fill-del-01 (100/204, gold cut off)`, `+fill-del-02 (100/204, gold cut off)`, `+fill-del-03 (100/204, gold cut off)`, `+fill-del-04 (100/204, gold cut off)`, `+fill-val-03 (100/1687, gold cut off)`
- calibrated abstain: 6 for jevify, 0 for thin, 0 both wrong — `+fill-cal-04`, `+fill-cal-05`, `+fill-del-06 (100/204)`, `+fill-jst-04`, `+fill-val-06`, `+fill-zox-06`
- abstention (jevify declined an answer thin got right): 0 for jevify, 1 for thin, 0 both wrong — `-fill-zox-08`

**filter**

- abstention (jevify declined an answer thin got right): 0 for jevify, 12 for thin, 0 both wrong — `-filter-cal-cli-01`, `-filter-cal-cli-07`, `-filter-cal-cli-x-01`, `-filter-cal-cli-x-02`, `-filter-cal-cli-x-03`, `-filter-cal-cli-x-04`, `-filter-val-ruff-04`, `-filter-zox-02`, `-filter-zox-05`, `-filter-zox-06`, `-filter-zox-08`, `-filter-zox-10`
- calibrated abstain: 5 for jevify, 0 for thin, 1 both wrong — `+filter-val-dav-x-01`, `+filter-val-dav-x-02`, `+filter-val-dav-x-03`, `+filter-val-dav-x-04`, `+filter-zox-09`, `~filter-jst-06`
- question framing (same options, one request): 3 for jevify, 0 for thin, 0 both wrong — `+filter-val-ruff-05`, `+filter-val-ruff-06`, `+filter-zox-01`

**label**

- calibrated abstain: 5 for jevify, 0 for thin, 0 both wrong — `+label-cal-cli-01`, `+label-cal-cli-04`, `+label-cal-cli-05`, `+label-cal-pai-07`, `+label-val-ruff-08`

**pick**

- calibrated abstain: 5 for jevify, 0 for thin, 0 both wrong — `+pick-cal-03`, `+pick-del-06`, `+pick-jst-06`, `+pick-val-04`, `+pick-zox-07`
- question framing (same options, one request): 1 for jevify, 0 for thin, 0 both wrong — `+pick-zox-02`

**pick --from**

- listers + chunking (thin read a 100-line cut of the listing): 3 for jevify, 0 for thin, 0 both wrong — `+pickfrom-cal-01 (100/1406, gold cut off)`, `+pickfrom-cal-03 (100/2834, gold cut off)`, `+pickfrom-val-03 (100/1687, gold cut off)`
- calibrated abstain: 2 for jevify, 0 for thin, 0 both wrong — `+pickfrom-cal-04 (100/2834)`, `+pickfrom-val-05 (100/1687)`
- abstention (jevify declined an answer thin got right): 0 for jevify, 1 for thin, 0 both wrong — `-pickfrom-val-04 (100/264)`

**route**

- listers + chunking (thin read a 100-line cut of the listing): 6 for jevify, 0 for thin, 4 both wrong — `+route-cal-arch-01 (100/1910, gold cut off)`, `+route-cal-text-01 (100/1910, gold cut off)`, `+route-cal-text-02 (100/1910, gold cut off)`, `+route-cal-text-03 (100/1910, gold cut off)`, `+route-val-net-01 (100/1910, gold cut off)`, `+route-val-net-03 (100/1910, gold cut off)`, `~route-cal-arch-02 (100/1910, gold cut off)`, `~route-val-dev-01 (100/1910, gold cut off)`, `~route-val-dev-02 (100/1910, gold cut off)`, `~route-val-net-02 (100/1910, gold cut off)`
- calibrated abstain: 4 for jevify, 0 for thin, 0 both wrong — `+route-cal-arch-03 (100/1910)`, `+route-ctr-07`, `+route-dat-06`, `+route-med-07`

**why**

- two-round finalists with failure context: 13 for jevify, 1 for thin, 0 both wrong — `+why-c-compile-error`, `+why-c-link-undefined`, `+why-cal-cargo-03 (97/241)`, `+why-cal-docker-03 (97/234)`, `+why-cal-go-01 (87/170)`, `+why-cal-pytest-02 (99/292)`, `+why-py-module-not-found`, `+why-py-unittest-failure`, `+why-rust-index-panic`, `+why-rust-serde-derive-feature`, `+why-val-bevyengine-bevy-34039655631 (99/292)`, `+why-val-prometheus-prometheus-33817696493 (99/283)`, `+why-val-prometheus-prometheus-33824930195 (100/299)`, `-why-cal-npm-04 (78/240)`
- calibrated abstain: 5 for jevify, 0 for thin, 0 both wrong — `+why-c-warnings-only`, `+why-cal-ok-tokio-rs-tokio-32519536561 (100/150)`, `+why-rust-clean-build`, `+why-val-denoland-deno-35714383921 (100/300)`, `+why-val-ok-prometheus-prometheus-33830612264 (92/102)`
- chunking into windows (thin read the last 100 lines): 2 for jevify, 0 for thin, 0 both wrong — `+why-val-bevyengine-bevy-34095699482 (87/294, gold cut off)`, `+why-val-rust-lang-rust-analyzer-30087733489 (100/291, gold cut off)`

## Per set

| verb | arm | answering model | n | accuracy on answered | coverage | false actions | abstained | right of all | p50 ms | p95 ms | requests per case |
|:--|:--|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| fill (validation) | jevify keyless | jev-1.13.0 | 12 | 89% (8/9) | 75% | 8% (1) | 25% (3 right) | 67% | 1141 | 3678 | 6.50 |
| filter (validation) | jevify keyless | jev-1.13.0 | 40 | 100% (29/29) | 72% | 0% (0) | 28% (4 right) | 72% | 531 | 966 | 0.15 |
| is (validation) | jevify keyless | jev-1.13.0 | 15 | 87% (13/15) | 100% | 13% (2) | 0% (0 right) | 87% | 545 | 799 | 1.00 |
| label (validation) | jevify keyless | jev-1.13.0 | 32 | 85% (23/27) | 84% | 12% (4) | 16% (5 right) | 72% | 1150 | 1236 | 0.12 |
| pick (validation) | jevify keyless | jev-1.13.0 | 12 | 90% (9/10) | 83% | 8% (1) | 17% (2 right) | 75% | 575 | 985 | 1.00 |
| pick --from (validation) | jevify keyless | jev-1.13.0 | 9 | 100% (6/6) | 67% | 0% (0) | 33% (2 right) | 67% | 3055 | 4848 | 15.89 |
| route (validation) | jevify keyless | jev-1.13.0 | 12 | 86% (6/7) | 58% | 8% (1) | 42% (1 right) | 50% | 4605 | 5374 | 21.00 |
| why (validation) | jevify keyless | jev-1.13.0 | 21 | 78% (14/18) | 86% | 19% (4) | 14% (3 right) | 67% | 1649 | 2112 | 3.67 |
| fill (validation) | jevify TypeSafe | jev-1.13.0 | 12 | 89% (8/9) | 75% | 8% (1) | 25% (3 right) | 67% | 535 | 1028 | 3.92 |
| filter (validation) | jevify TypeSafe | jev-1.13.0 | 40 | 100% (33/33) | 82% | 0% (0) | 18% (4 right) | 82% | 367 | 442 | 0.15 |
| is (validation) | jevify TypeSafe | jev-1.13.0 | 15 | 87% (13/15) | 100% | 13% (2) | 0% (0 right) | 87% | 358 | 384 | 1.00 |
| label (validation) | jevify TypeSafe | jev-1.13.0 | 32 | 85% (22/26) | 81% | 12% (4) | 19% (5 right) | 69% | 375 | 380 | 0.12 |
| pick (validation) | jevify TypeSafe | jev-1.13.0 | 12 | 90% (9/10) | 83% | 8% (1) | 17% (2 right) | 75% | 389 | 536 | 1.00 |
| pick --from (validation) | jevify TypeSafe | jev-1.13.0 | 9 | 100% (7/7) | 78% | 0% (0) | 22% (2 right) | 78% | 962 | 1286 | 8.78 |
| route (validation) | jevify TypeSafe | jev-1.13.0 | 12 | 86% (6/7) | 58% | 8% (1) | 42% (1 right) | 50% | 2282 | 2950 | 11.00 |
| why (validation) | jevify TypeSafe | jev-1.13.0 | 21 | 82% (14/17) | 81% | 14% (3) | 19% (4 right) | 67% | 665 | 718 | 2.62 |
| fill (validation) | thin | jev-1.13.0 | 12 | 50% (6/12) | 100% | 50% (6) | 0% (0 right) | 50% | 1124 | 1207 | 1.00 |
| filter (validation) | thin | jev-1.13.0 | 40 | 85% (34/40) | 100% | 15% (6) | 0% (0 right) | 85% | 499 | 1015 | 1.00 |
| is (validation) | thin | jev-1.13.0 | 15 | 87% (13/15) | 100% | 13% (2) | 0% (0 right) | 87% | 504 | 694 | 1.00 |
| label (validation) | thin | jev-1.13.0 | 32 | 72% (23/32) | 100% | 28% (9) | 0% (0 right) | 72% | 529 | 1155 | 1.00 |
| pick (validation) | thin | jev-1.13.0 | 12 | 75% (9/12) | 100% | 25% (3) | 0% (0 right) | 75% | 1109 | 1200 | 1.00 |
| pick --from (validation) | thin | jev-1.13.0 | 9 | 44% (4/9) | 100% | 56% (5) | 0% (0 right) | 44% | 580 | 1194 | 1.00 |
| route (validation) | thin | jev-1.13.0 | 12 | 0% (0/12) | 100% | 100% (12) | 0% (0 right) | 0% | 518 | 723 | 1.00 |
| why (validation) | thin | jev-1.13.0 | 21 | 29% (6/21) | 100% | 71% (15) | 0% (0 right) | 29% | 1014 | 1367 | 1.00 |
| fill (validation) | thin+threshold | jev-1.13.0 | 12 | 86% (6/7) | 58% | 8% (1) | 42% (4 right) | 50% | 1124 | 1207 | 1.00 |
| filter (validation) | thin+threshold | jev-1.13.0 | 40 | 85% (34/40) | 100% | 15% (6) | 0% (0 right) | 85% | 499 | 1015 | 1.00 |
| is (validation) | thin+threshold | jev-1.13.0 | 15 | 87% (13/15) | 100% | 13% (2) | 0% (0 right) | 87% | 504 | 694 | 1.00 |
| label (validation) | thin+threshold | jev-1.13.0 | 32 | 72% (23/32) | 100% | 28% (9) | 0% (0 right) | 72% | 529 | 1155 | 1.00 |
| pick (validation) | thin+threshold | jev-1.13.0 | 12 | 82% (9/11) | 92% | 17% (2) | 8% (1 right) | 75% | 1109 | 1200 | 1.00 |
| pick --from (validation) | thin+threshold | jev-1.13.0 | 9 | 67% (4/6) | 67% | 22% (2) | 33% (2 right) | 44% | 580 | 1194 | 1.00 |
| route (validation) | thin+threshold | jev-1.13.0 | 12 | 0% (0/3) | 25% | 25% (3) | 75% (2 right) | 0% | 518 | 723 | 1.00 |
| why (validation) | thin+threshold | jev-1.13.0 | 21 | 0% (0/2) | 10% | 10% (2) | 90% (3 right) | 0% | 1014 | 1367 | 1.00 |
| fill (holdout) | jevify keyless | jev-1.13.0 | 21 | 94% (16/17) | 81% | 5% (1) | 19% (3 right) | 76% | 1089 | 1596 | 2.38 |
| filter (holdout) | jevify keyless | jev-1.13.0 | 20 | 92% (12/13) | 65% | 5% (1) | 35% (1 right) | 60% | 530 | 762 | 0.10 |
| pick (holdout) | jevify keyless | jev-1.13.0 | 20 | 88% (15/17) | 85% | 10% (2) | 15% (3 right) | 75% | 571 | 695 | 1.00 |
| route (holdout) | jevify keyless | jev-1.13.0 | 20 | 94% (16/17) | 85% | 5% (1) | 15% (3 right) | 80% | 1202 | 1818 | 2.00 |
| why (holdout) | jevify keyless | jev-1.13.0 | 22 | 100% (20/20) | 91% | 0% (0) | 9% (2 right) | 91% | 1156 | 1724 | 2.00 |
| fill (holdout) | jevify TypeSafe | jev-1.13.0 | 21 | 94% (16/17) | 81% | 5% (1) | 19% (3 right) | 76% | 653 | 871 | 2.10 |
| filter (holdout) | jevify TypeSafe | jev-1.13.0 | 20 | 92% (12/13) | 65% | 5% (1) | 35% (1 right) | 60% | 339 | 399 | 0.10 |
| pick (holdout) | jevify TypeSafe | jev-1.13.0 | 20 | 88% (15/17) | 85% | 10% (2) | 15% (3 right) | 75% | 382 | 507 | 1.00 |
| route (holdout) | jevify TypeSafe | jev-1.13.0 | 20 | 94% (15/16) | 80% | 5% (1) | 20% (3 right) | 75% | 868 | 964 | 2.00 |
| why (holdout) | jevify TypeSafe | jev-1.13.0 | 22 | 100% (20/20) | 91% | 0% (0) | 9% (2 right) | 91% | 634 | 895 | 2.00 |
| fill (holdout) | thin | jev-1.13.0 | 21 | 62% (13/21) | 100% | 38% (8) | 0% (0 right) | 62% | 525 | 1302 | 1.00 |
| filter (holdout) | thin | jev-1.13.0 | 20 | 80% (16/20) | 100% | 20% (4) | 0% (0 right) | 80% | 518 | 730 | 1.00 |
| pick (holdout) | thin | jev-1.13.0 | 20 | 70% (14/20) | 100% | 30% (6) | 0% (0 right) | 70% | 522 | 867 | 1.00 |
| route (holdout) | thin | jev-1.13.0 | 20 | 80% (16/20) | 100% | 20% (4) | 0% (0 right) | 80% | 512 | 1343 | 1.00 |
| why (holdout) | thin | jev-1.13.0 | 22 | 64% (14/22) | 100% | 36% (8) | 0% (0 right) | 64% | 1094 | 1456 | 1.00 |
| fill (holdout) | thin+threshold | jev-1.13.0 | 21 | 93% (13/14) | 67% | 5% (1) | 33% (3 right) | 62% | 525 | 1302 | 1.00 |
| filter (holdout) | thin+threshold | jev-1.13.0 | 20 | 80% (16/20) | 100% | 20% (4) | 0% (0 right) | 80% | 518 | 730 | 1.00 |
| pick (holdout) | thin+threshold | jev-1.13.0 | 20 | 74% (14/19) | 95% | 25% (5) | 5% (1 right) | 70% | 522 | 867 | 1.00 |
| route (holdout) | thin+threshold | jev-1.13.0 | 20 | 84% (16/19) | 95% | 15% (3) | 5% (1 right) | 80% | 512 | 1343 | 1.00 |
| why (holdout) | thin+threshold | jev-1.13.0 | 22 | 55% (6/11) | 50% | 23% (5) | 50% (1 right) | 27% | 1094 | 1456 | 1.00 |

## Commits whose subject lies (`fill` commit kind)

`evals/commit-subjects`: the scratch repository (21 commits, 10 cases) and ripgrep at `3fce3b5` (2,287 commits, 10 cases). Right means the code commit that holds the change; the thin arm reads `git log --format='%h %s'`, the newest 100 lines on ripgrep.

| half | arm | answering model | n | right | wrong | lying subject won | abstained | requests per case | median s |
|:--|:--|:--|--:|--:|--:|--:|--:|--:|--:|
| scratch | jevify keyless | jev-1.13.0 | 10 | 10 | 0 | 0 | 0 | 2.1 | 2.13 |
| scratch | jevify TypeSafe | jev-1.13.0 | 10 | 10 | 0 | 0 | 0 | 2.0 | 1.70 |
| scratch | thin | jev-1.13.0 | 10 | 0 | 10 | 10 | 0 | 1.0 | 0.53 |
| ripgrep | jevify keyless | jev-1.13.0 | 10 | 6 | 1 | 0 | 3 | 25.7 | 5.80 |
| ripgrep | jevify TypeSafe | jev-1.13.0 | 10 | 8 | 1 | 0 | 1 | 13.0 | 2.35 |
| ripgrep | thin | jev-1.13.0 | 10 | 1 | 9 | 0 | 0 | 1.0 | 0.54 |

## Rerun and order sensitivity

`evals/variance` subset of the holdout set. A question has changed when its runs do not all give the same answer; a change to another confident answer is the kind a caller cannot see. Order applies to candidates that arrive as a list, so `why` has no order row.


**Rerun (5 cold runs)**

| verb | arm | questions | answer changed | changed to another confident answer | right in every run |
|:--|:--|--:|--:|--:|--:|
| fill | jevify keyless | 4 | 0 | 0 | 4 |
| filter | jevify keyless | 20 | 1 | 0 | 12 |
| pick | jevify keyless | 7 | 0 | 0 | 5 |
| route | jevify keyless | 6 | 0 | 0 | 6 |
| why | jevify keyless | 6 | 1 | 1 | 6 |
| fill | jevify TypeSafe | 4 | 0 | 0 | 4 |
| filter | jevify TypeSafe | 20 | 3 | 0 | 11 |
| pick | jevify TypeSafe | 7 | 0 | 0 | 5 |
| route | jevify TypeSafe | 6 | 0 | 0 | 6 |
| why | jevify TypeSafe | 6 | 1 | 1 | 6 |
| fill | thin | 4 | 0 | 0 | 3 |
| filter | thin | 20 | 1 | 1 | 15 |
| pick | thin | 7 | 0 | 0 | 5 |
| route | thin | 6 | 1 | 1 | 4 |
| why | thin | 6 | 1 | 1 | 3 |

**Order (8 orders)**

| verb | arm | questions | answer changed | changed to another confident answer | right in every run |
|:--|:--|--:|--:|--:|--:|
| fill | jevify keyless | 2 | 0 | 0 | 2 |
| filter | jevify keyless | 20 | 2 | 0 | 11 |
| pick | jevify keyless | 4 | 1 | 1 | 2 |
| route | jevify keyless | 3 | 1 | 0 | 2 |
| fill | jevify TypeSafe | 2 | 0 | 0 | 2 |
| filter | jevify TypeSafe | 20 | 7 | 0 | 8 |
| pick | jevify TypeSafe | 4 | 1 | 1 | 2 |
| route | jevify TypeSafe | 3 | 1 | 0 | 2 |
| fill | thin | 2 | 0 | 0 | 2 |
| filter | thin | 20 | 0 | 0 | 15 |
| pick | thin | 4 | 1 | 1 | 2 |
| route | thin | 3 | 1 | 1 | 3 |

## Cost

Classifications: 3814 keyless (jevify and thin together), 2349 on TypeSafe. Not run: jevify keyless 0, jevify TypeSafe 0, thin 0, thin+threshold 0.

Regenerate: `bash evals/scorecard/run.sh` (TypeSafe arms run when `TYPESAFE_API_KEY_FILE` is set in its environment).
