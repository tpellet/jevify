# AGENTS.md — jevify

A fast Rust CLI that points at the right thing among real things — a line, a tool, a hunk, a
file — using TypeSafe's Jev model, with confidence scores and an honest "nothing fits".

- Vision: `docs/VISION.md` (decides direction: `fill` on the input side of a command, the
  stdin verbs on the output side; a plan that disagrees with it is out of date)
- Spec: `docs/superpowers/specs/2026-09-18-hunch-v0-design.md`
- Plan: `docs/superpowers/plans/2026-09-21-jevify-two-sides.md` (source of truth for tasks; beads
  transcribe it). `docs/superpowers/plans/2026-09-18-hunch-v0.md` is the plan of the verbs that
  exist.

Principles: point, never generate · at most two rounds of parallel Jev calls per verb ·
one decision threshold, with calibration
claims scoped to backend and task · Unix-first human output, one machine envelope · safe by
default. Incomplete evidence cannot authorize whole-input verdicts or larger actions.
Output verbs start no user command; callers authorize staging and file moves.

---

## Toolchain: Rust & Cargo

- Cargo only. Edition 2024, stable toolchain. `rust-version = "1.87"` is the supported floor;
  the local toolchain is pinned to 1.93.
- `#![deny(unsafe_code)]` in `src/lib.rs`. `unsafe { std::env::set_var(..) }` only inside
  `tests/` (edition 2024 marks it unsafe).
- Async: tokio current-thread runtime only; no second executor. Blocking local work (stdin,
  inventory, `man`) goes through `tokio::task::spawn_blocking` or `std::thread::scope` so the
  runtime keeps driving in-flight HTTP.
- Dependencies: explicit versions, minimal set, prefer std. Adding a crate needs a one-line
  justification in the commit body. `toon-format` must stay `default-features = false`.
- Verify third-party APIs against docs.rs or the downloaded crate source, not memory.
- macOS and Linux only for v0.

---

## Quality Gate (before every commit, all must pass)

```bash
cargo fmt --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
ubs <changed files>          # exit 0 required; verify findings, fix root causes
```

Run the gate and every `git commit`/`git push` with the sandbox disabled: wiremock binds
127.0.0.1 and SSH signing needs `~/.ssh`, both sandbox-denied. Never `#[ignore]`, weaken or
delete a test to get past a sandbox failure.

End-to-end tests (`tests/e2e.rs`, all `#[ignore]`: the real binary, the live backend, real
repositories and logs, gold answers):

```bash
TYPESAFE_API_KEY_FILE=$HOME/.ssh/typesafe-ai-key cargo test --test e2e -- --ignored --test-threads=1
```

Without a key they print `SKIPPED: set TYPESAFE_API_KEY_FILE=...` and return; a hand-off lists
them as NOT RUN, never as passed. The keyless half runs only with `JEVIFY_E2E_KEYLESS=1`, so no
default run spends classifier.dev's free per-IP budget. No release ships unless this suite
passes on the TypeSafe backend, known misses excepted and named in the release notes.

The transcripts of `README.md` and `docs/guide/getting-started.md` have their own live check
(`tests/transcripts.rs`, all `#[ignore]`), which runs every documented command and compares the
decision and the chosen item, never the probability:

```bash
TYPESAFE_API_KEY_FILE=$HOME/.ssh/typesafe-ai-key \
    cargo test --test transcripts -- --ignored --test-threads=1
```

Run it after editing a transcript and after changing what a verb selects; a full run spends
about 31 keyless classifications and 22 on TypeSafe. The expectations are the table in that
file, each row citing the page and line it came from, so a page and its check move together.
The scheduled `transcripts` workflow runs the keyless half weekly.

---

## Testing

- Owner decision 2026-09-28: end-to-end tests with real data prove behaviour; unit tests are the
  exception. `tests/e2e.rs` runs the real binary against the live backend on real repositories
  and logs, with gold answers, and asserts the decision and the chosen item, never a
  probability. Run it after any change to what a verb selects (command below).
- A unit or contract test earns its place only when it guards a contract that an end-to-end run
  cannot check cheaply: exit codes, the `--json` envelope and error kinds, marker parsing, `fill`
  execution safety, redaction and host pinning, byte-exact records, backend request limits,
  HTTP status mapping, the deadline. The list is `docs/superpowers/plans/2026-09-28-refocus.md`,
  "Keep list". No test pins a tuning constant (window sizes, request counts, finalist counts).
- Contract tests use `tests/common::FakeJev` (wiremock). No live network in the default test run.
- Tests run single-threaded (`--test-threads=1`): integration tests spawn the binary with
  per-process env; one thread keeps wiremock ports and stdin handling deterministic.

---

## jevify — This Project

**Release:** review the outgoing release diff, then run `bash scripts/release.sh <full commit SHA>`
for a successful `main` CI commit. The script reads the committed version and pushes its signed
tag; cargo-dist builds GitHub assets and calls `publish-crates.yml` to publish from a clean
checkout. Never publish the shared local working tree. Crates.io trusts the calling workflow
`release.yml`, not the reusable workflow filename. If publishing fails, check whether the registry
accepted the version before rerunning failed jobs. Never replace an existing tag. If only the
tag push failed, inspect the retained signed tag and push that exact tag after the usual scan.
CI validates the package with `cargo publish --dry-run --locked` and checks generated workflow
consistency with the pinned cargo-dist `dist plan`; it keeps cancelling superseded branch runs.

**`main` only.** No branches, no worktrees, no scratch clones, no PRs. One shared checkout.
Parallel agents all work in it and coordinate through Agent Mail: each registers, reserves its
exact write paths before its first edit, and posts in the bead's thread (global rule:
`~/.claude/AGENTS.md` RULE NUMBER 2). Tasks with a real dependency stay
serial. If Agent Mail is down, run serially; never fall back to a worktree. Stage and commit only
your own paths (`git commit -- <paths>`); the index is shared. Commits are SSH-signed; never bypass
signing. Commit subject: semantic prefix + bead ID, e.g. `feat: add jevify pick (hunch-abc)`.

**Unattended runs: never trigger a confirmation prompt.** Agents run while Thomas is away; a
prompt nobody answers stalls the whole run. If a destructive or mutating step seems needed
(`rm`, `git reset --hard`, `git stash`, force-push, `mv` over an existing file, anything `dcg`
blocks), skip it, record it in the bead notes or your report, and continue. Leave scratch files
in place. Never use `rm` in any form (`rm -f`, `rm -rf`), `rmdir`, `unlink`, `git rm`,
`find -delete`, `cargo clean`, `git restore`, `git worktree remove` or `git branch -D`, even on
your own scratch files: overwrite with the Write/Edit tool, write a new filename, or create a new
directory instead. Use non-interactive flags (`-y`, `--yes`) for every command. `dcg` blocking a command is not a puzzle to route around.

**Public repo: scan before every push.** `tpellet/jevify` is public; a push is publication. Before
every `git push`: `gitleaks git . --log-opts="origin/main..HEAD" --redact -v` (no leaks; the repo's
`.gitleaks.toml` applies), then read `git diff origin/main..HEAD` for keys, tokens, pasted API
responses, absolute local paths, session URLs, personal details, other people's data. On a hit:
fix forward in a new commit, never rewrite history, never force-push. Never commit communications
drafts (tweet, Show HN, announcements): they live in `~/Projects/jevify-launch/`, outside the repo,
and never go into bead fields or commit messages. User-facing docs say
`TYPESAFE_API_KEY_FILE=/path/to/key` and "a typical macOS dev machine", never the real key path
or personal tooling. Commit bodies stay technical: no `Co-Authored-By`, no `Claude-Session:`
trailer. Global rule: `~/.claude/AGENTS.md` "Public Repositories — Push With Care".

**Docs state what jevify does, in the present tense, as if it was always so.** No meta language
(sentences about the document or how its content was made: "captured on", "excerpts", "this guide
is") and no retrospective language ("now", "since 0.x", "new in", "ships", "release state"). Dates
and conditions belong only next to a measurement; history belongs in `CHANGELOG.md`.

**Live API key.** Point jevify at the key with
`TYPESAFE_API_KEY_FILE=$HOME/.ssh/typesafe-ai-key` in the command's environment — jevify reads
it, you never do. Never echo the variable or the key. Live commands need the sandbox disabled
because `~/.ssh` is sandbox-denied.

### Agent-facing contract (do not break without updating capabilities + docs + tests)

Exit codes 0 ok · 1 no · 2 usage · 3 abstain · 4 unavailable · 5 auth · 6 input · 7 reserved
(no verb reports a child command's failure) · 130 declined (`add` confirmation; `sort` has no
confirmation prompt). Machine envelope: `{ok, command, version, exit_code, data, meta, error}`
with `error{kind, message, hint, example}`. Error `kind` strings are stable identifiers.
New verbs update `cli.rs`, dispatch, `capabilities()`, `docs/ROBOT_MODE.md`, README, a
PRIVACY.md row, and tests.
