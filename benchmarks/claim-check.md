# Claim check: can `is` verify an agent's claim against a log?

Question: does a `verify` verb (claim + evidence → supports / contradicts / unsupported) earn a
place, given that `jevify is "<claim>" --context <log>` already exists?

## Set

20 claims of the kind a coding agent writes after a run ("all tests pass", "the failure is in X",
"the build failed because Y"), each against one real log:

- 10 failed CI logs from `evals/why/` (cargo, docker, go, npm, pytest; sources in each `.expect`).
- One `cargo test --locked -- --test-threads=1` run of this repository (318 lines, all suites
  pass, e2e and transcript tests ignored).

Each claim carries a hand label written before any run: **S** supports (10), **C**
contradicts (6), **U** unsupported, i.e. the log neither shows nor rules it out (4).

| # | log | label | claim | `is` | p |
|---|-----|-------|-------|------|---|
| 1 | cargo test (this repo) | S | all tests pass | yes | 0.78 |
| 2 | cargo test (this repo) | C | at least one test failed | no | 0.02 |
| 3 | cargo test (this repo) | U | the release build is under 5 MB | no | 0.05 |
| 4 | cargo-01 | S | the docs build failed because the import IntoRawHandle cannot be resolved | yes | 0.98 |
| 5 | cargo-01 | C | all tests pass | no | 0.02 |
| 6 | cargo-01 | C | the failure is in tokio/src/runtime/scheduler.rs | no | 0.01 |
| 7 | cargo-03 | S | the build failed because of a clippy lint | yes | 0.98 |
| 8 | cargo-03 | C | the build failed because of a type mismatch error | no | 0.05 |
| 9 | docker-02 | S | the build failed because pulling the upx image was denied | yes | 0.97 |
| 10 | docker-02 | U | the failure is caused by an expired registry password secret | no | 0.04 |
| 11 | go-01 | S | the failure is in TestSqlUpdate | yes | 0.99 |
| 12 | go-01 | C | the failure is in TestSqlInsert | no | 0.01 |
| 13 | go-03 | S | TestPullHandlerForceBypassesFitCheck made more blob requests than expected | yes | 0.97 |
| 14 | go-03 | U | the test is flaky and passes on retry | no | 0.17 |
| 15 | npm-01 | S | the build failed because fileToUrl is imported twice in wasm.ts | yes | 0.95 |
| 16 | npm-01 | C | the build failed because a dependency could not be installed | no | 0.04 |
| 17 | npm-03 | S | the failure is that the name Temporal cannot be found in a type check | yes | 0.98 |
| 18 | pytest-01 | S | the failure is a deprecation warning raised as an error | yes | 0.95 |
| 19 | pytest-03 | U | the failure is caused by a change in Python 3.15 | no | 0.25 |
| 20 | docker-04 | S | the build failed because apt could not install packages with unmet dependencies | yes | 0.98 |

## Result (2026-10-01, jevify 0.14.1, TypeSafe jev-1.13.0, threshold 0.5, band 0.15, no cache)

Scoring: `yes` is correct for S; `no` is correct for C and U, since `is` answers "is the claim
established by the text", which is what an agent needs before acting.

| measure | value |
|---------|-------|
| answered | 20 of 20 (abstain rate 0%) |
| accuracy on answered | 20 of 20 |
| false "supports" (yes on C or U) | 0 of 10 |
| missed supports (no on S) | 0 of 10 |
| requests | 1 per claim, 20 total |
| backend time | median 230 ms, max 414 ms (14.3k input tokens, go-03) |
| wall time per call | 0.23–0.44 s |
| truncated input | none (largest 14.3k tokens) |

The lowest-margin answers sit where expected: "all tests pass" on a passing run at 0.78 (the log
also lists ignored tests), and the two speculative-cause claims (14, 19) at 0.17 and 0.25, inside
or near the unsure band but on the right side.

Limits of this probe: 20 claims, written by one person against logs whose cause is already
known; the false claims name a wrong file, test or cause rather than a subtly wrong one. Every
log fits in one request. The probe says nothing about logs over the input limit, claims that
mix a true part with a false part, or claims about counts ("3 tests fail"), which `is` refuses
by contract.

## Recommendation

Do not build `verify` next. On this set `is` already does the job a `verify` verb would do,
with zero false "supports", one request and about a quarter of a second per claim; the only
thing `verify` adds is splitting "no" into contradicts versus unsupported, and an agent acting
on a claim treats both the same (do not proceed). Document the pattern instead:
`jevify is "<claim>" --context <log>` in the guide and the skill, with "yes means the log
establishes it". Revisit `verify` only if a harder set shows it is needed; that set would need
logs over the input limit (where `verify` would have to window the log the way `why` does and
refuse a whole-input verdict on incomplete evidence), compound claims split into parts, and
near-miss false claims (right test, wrong assertion), measured against the same zero-false-yes
bar.
