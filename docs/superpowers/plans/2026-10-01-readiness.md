# Readiness pass (0.14.1 → advertisable)

Goal: prove the shipped product and fix only reproduced defects. No new verbs.
Source: Fable product review of 0.14.1 (2026-09-28), critiqued by gpt-6-astra and gpt-6.1-sol
(2026-09-30). Owner approved 2026-10-01.

## Tasks

R1. Reproduce the keyless 402. Capture the real classifier.dev body when the per-IP daily budget
    is spent. Fix the mapping in `src/jev/client.rs` only if that body reaches the
    `request_spending_limit` arm and reports `input_too_large`. Contract test with the captured
    body (redacted). Done: captured body in the bead notes; test passes; or the bead notes show
    the bug does not reproduce.
R2. Honest e2e gate (`tests/e2e.rs`). Keep gold assertions. Known misses are named and reported
    separately, never counted as passes. A TypeSafe run without a key fails loudly instead of
    returning success. Done: gate output lists passes, named misses and failures separately.
R3. Surface sweep. Remove `route` and `sort` from `.claude-plugin/marketplace.json`. Write help
    text for the eight blank `fill` flags in `src/cli.rs`. Leave "windows 13" (1,212 / 99 = 13).
R4. Key positioning. README, getting-started and `action.yml` say: free tier to try, a TypeSafe
    key for CI and sustained use. A key is not mandatory.
R5. Latency measurement. Uncached runs per backend, with retries, failures and accuracy recorded,
    p50/p95 across runs, in `benchmarks/latency.md`. Change code only for a demonstrated cause,
    in a follow-up bead.
R6. `fill` sample. 20 real tasks on pinned public repos, with gold answers, in
    `benchmarks/fill-sample.md`: hit, abstain, wrong, wrong-and-would-execute.
R7. Claim-check probe with existing `is`. 20 claims from real agent transcripts or CI runs
    against their logs: supports / contradicts / unsure vs a hand label, in
    `benchmarks/claim-check.md`. Decides whether `verify` enters the next plan.
R8. Gate and release. Full quality gate, live e2e, transcripts. Release only if green.
    Advertise only measured results.

## Cut

Triage fields on `why`, `verify`, hedged requests, 4 s timeout, `health` change, crate rename
(separate decision).

## Changelog

- 2026-10-01: plan written from the merged astra and sol reviews; owner said execute.
