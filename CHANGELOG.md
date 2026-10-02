# Changelog

## Unreleased

## 0.15.0 - 2026-10-02

Added:

- `jevify mcp`: a stdio MCP server with the tools `why`, `is` and `pick` (select only), for Claude
  Code, Codex and Claude Desktop (`packaging/mcpb/manifest.json`). An abstention is a result with
  `data.shortlist`, not an error.
- `why --hook claude|codex [--min-lines N]` reads the agent's tool-failure payload and prints the
  hook JSON itself; the plugin hook no longer needs `jq`. A hook run always exits 0.
- The GitHub Action's `classes:` input names the kind of failure next to the line.
- `jevify init agents` and the plugin skill carry the "run the test you mean" recipe for cargo
  and pytest.
- Every `quota_exhausted`, missing-key and bad-key hint, `health` (`data.key_url`) and the
  Action's exit-4 summary point at https://console.typesafe.ai/keys.

Changed:

- `label` is `filter --label A,B,C`. The `label` verb is removed.
- `fill` lists `data.shortlist` (and `markers[].shortlist`) on every exit 3; the list is empty,
  never null, when nothing was scored.
- The README leads with `why` and the test recipe; `add` moves to "Other verbs".

Measured:

- On TypeSafe, 2026-10-01 and 2026-10-02: failure class on 34 real CI runs, 26 right, 4 abstain,
  4 wrong (`benchmarks/why-triage.md`); a test chosen by behaviour, 43 of 45, 5 of 5 "none fits"
  abstained, 0 wrong (`benchmarks/test-by-behaviour.md`).

## 0.14.2 - 2026-10-01

Fixed:

- A spent classifier.dev per-IP daily budget reports `quota_exhausted` (exit 4). classifier.dev
  answers it with `request_spending_limit` and a `limitUsd` below $0.01; jevify reported
  `input_too_large` (exit 6).

Changed:

- Every flag of `fill`, `filter` and `init` has help text. The plugin listing names the current
  verbs.
- The README, the getting-started guide and the GitHub Action say the keyless tier is for trying
  jevify; CI and team use need a TypeSafe key.
- The end-to-end gate reports passes, named known misses and failures apart, and a TypeSafe run
  without a key fails instead of passing.

Measured:

- On 2026-10-01 with TypeSafe: 330 uncached runs, slowest 0.81 s (`benchmarks/latency.md`);
  `fill` on 20 agent requests, 17 hits, 2 abstentions, 0 wrong (`benchmarks/fill-sample.md`);
  `is` as a claim checker on 20 real logs, 20 of 20, no false "supports"
  (`benchmarks/claim-check.md`).

## 0.14.1 - 2026-09-28

Changed:

- `why` strips CI prefixes and ANSI escapes from selection evidence, preserves original output
  lines, and selects one cause per failed job. It recognizes TAP `not ok`, follows a selected
  summary with its diagnostic block, and exposes `data.shortlist` on exit 3.

Measured:

- On 2026-09-28 with TypeSafe, `why` root-cause-first hits rise from 28 to 30 of 34 failed
  GitHub Actions runs from 30 public repositories; wrong first lines fall from six to four,
  with zero abstentions. Top-three hits rise from 30 to 31 of 34. JSON stdout contains 99.76%
  fewer estimated tokens than full logs; p50 latency is 0.87 s on a loaded machine.
  Tail finds a gold line in 10/34, grep in 11/34. These are development-set payload and
  retrieval measurements, not total inference cost or agent task success (`benchmarks/why-ci.md`).

## 0.14.0 - 2026-09-28

Changed:

- The call shapes coding agents write work. A description is one quoted argument or several
  bare words (`jevify pick --from commit names the borrow`), for `pick`, `filter`, `add` and
  `route`; `is` keeps one statement per argument and warns when three or more are single words.
  A leading `-` and `--from -` name stdin, and a verb's options may come before the verb. In the
  adoption study, agent calls failed on argument shape or directory before any model was asked
  (benchmarks/agents/ADOPTION.md, section 5).
- `-C DIR` (`--repo`) runs `fill` or `pick` as if started in DIR, and `fill` lists where a
  `git -C DIR` command points instead of failing with `not a git repository`.
- `pick --files` with nothing on stdin ranks the files under the current directory and says so,
  where it was an input error.
- Every error names a command to run next, the caller's own corrected when jevify can, in the
  `try:` line and `error.example`, instead of a pointer to `capabilities`. A lister run outside a
  repository names the directory it ran in and the repositories below it. An unknown verb names
  the nearest verb, the installed version and `cargo install jevify`.
- An abstention names its nearest candidates as `nearest (not chosen)`, with scores and a hint,
  in `pick`, `why`, `add` and `fill`; `pick`'s envelope adds `shortlist` and `hint`. An agent had
  answered with a 0.27 candidate from the old `closest:` line.
- A bare `@{branch:...}` in a git subcommand that reads a revision (`log`, `show`, `diff`, ...)
  becomes the remote ref of a branch that exists only on a remote, which git resolves; `switch`
  and `checkout` keep the short name.
- `add` without `--yes` and without a terminal declines (exit 130) before any classification.
- `sort` accepts one file, sorted alone among the folders beside it or under `--into`.

Removed:

- `sort`, `route` (use `pick --from tool`), the `robot-docs` command, `init zsh|bash`,
  the comma alias and command-not-found hook, `--format`, cost-estimate fields,
  `JEVIFY_PRICE_PER_MTOK`, and reserved exit 7. `docs/ROBOT_MODE.md` remains the handbook.

Changed:

- `why` and `fill` lead the README, with the binary installer first and no Rust requirement.
- Selection uses one abstention rule, including stdin `pick` near ties. `pick --from` reads
  evidence for commit, file and directory finalists; finalist evidence budget follows rank.
- `--json` emits one line. `capabilities` is compact and `init agents` fits one screen.
  Noninteractive `add` without `--yes` or `--dry-run` is usage error 2, not decline 130.
- Spent TypeSafe credits or classifier.dev budget produce `quota_exhausted` (exit 4), never
  retried. A per-request spending limit is `input_too_large` (exit 6). The free service budget
  is $0.50 per IP per UTC day, subject to $100 per day across everyone and four concurrent
  requests; 20,000 remains an input record ceiling, not a daily quota.

Added:

- Claude Code Bash `PostToolUseFailure` hook: `why` supplies a cause for long failed output.
- GitHub Action: `uses: tpellet/jevify@<tag>` writes a `why` answer to the job summary.
- Abstention shortlists expose candidates with scores and evidence without selecting them.
  `health` makes a small uncached classification to detect quota or credit exhaustion.

Measured:

- On 2026-09-28, TypeSafe puts the root-cause line first in 28 of 34 failed GitHub Actions
  runs from 30 public repositories, and in the top three in 30. It points at a wrong line in
  six first-line runs. Tail finds a gold line in 10, grep in 11; JSON output has 99.7% fewer
  estimated tokens than the full logs, with median latency 0.84 s (`benchmarks/why-ci.md`).
  Token reduction describes downstream payload, not total inference cost or agent task success.
- On the same date and backend, `fill` resolves bat PR #4018 among 1,000 open and closed PRs
  in 6.9 s, with 424× fewer bytes than reading the listing.

Documents:

- What each store in the cache directory keeps is written down, and which switch stops it.
  The answer cache holds only the model name, the probabilities and the chosen option: no record
  text, excerpt, path or question reaches disk, and the request survives only as the hash in the
  file name. A marker planted in every input across all nine caching verbs appeared in the two
  saved-input files and in none of the thirty answer files. `capabilities` gains `answer_cache`.
  Two switch facts the documents had wrong: `--no-cache` relocates `sort --apply`'s recovery
  journal to the system temporary directory rather than suppressing it, and the `tool` kind's
  inventory bypasses the switch entirely (`evals/cache-retention/`).

Measured:

- Whether an agent reaches for jevify, measured apart from whether it helps. Over eight tasks a
  script proved lexical search cannot answer, an agent given jevify and told about it once reached
  for it in 11 of 16 runs, against none of six on an easier set; where it was told to use the tool
  and the tool named the answer, the agent adopted that answer 15 times of 15 and never overrode
  it. On correctness the three arms are indistinguishable, 15 of 16, 14 of 16 and 16 of 16 with
  intervals overlapping over most of their range, so no correctness gain is claimed at this sample
  size, and the cheaper resource profile of the choose-for-yourself arm is not claimed as an effect
  because one run of sixteen moves its total by a third (`benchmarks/agents/ADOPTION.md`).

## 0.13.0 - 2026-09-24

Changed:

- A drifted transcript now fails a check instead of shipping. `tests/transcripts.rs`, opt-in and
  never in the ordinary gate, runs every transcript of the README and the getting-started guide
  against both backends and compares the decision and the chosen item, never the probability,
  which moves with the model. A second check needs no network and asserts that every expectation
  still matches the page it cites, so the table and the pages cannot drift apart either. Three
  demonstrations had already stopped being true before it existed; it caught a fourth on the day
  it was written.

- The demonstrations show what the binary does. The commit demo lists a range between two release
  tags, so the thirty commits a reader pipes in are the ones the page was recorded against, and
  cannot fall out of a window as the history grows. The `pick` demo asked the model to compare
  month names against today's date, which the vision reserves for code and which made its answer
  right only during September; it now asks about meaning and resolves at 0.97 to 1.00 on both
  backends. The `ci-run` example in the kinds guide asked for a position an ordered lister had
  already decided, and the empty-input error showed a question a literal `grep` would answer.
  Both are replaced, and the figures carry the date they were recorded rather than a version
  number that churns weekly.


Changed:

- The overall deadline's message is its own sentence: `the overall deadline of N s passed before
  the answer was ready; JEVIFY_DEADLINE sets it`. It no longer reads as an outage of the API. The
  exit code stays 4 and the error kind stays `api_deadline`.

- The envelope's `ok` keeps its meaning, "jevify reached the end without an error of its own",
  which is true on a no and on an abstention; `capabilities.envelope` now says so and names
  `exit_code` as the field to branch on. A harness trial branched on `ok`, read an abstention as
  success and then failed on a `data` key that was not there.

## 0.12.0 - 2026-09-24

Measured:

- Both backends serve the whole `fill` capacity they advertise. A sweep of 151 attempts at seven
  candidate counts and three concurrency settings answered every request at or below the limit:
  3,267 candidates keyless in 6.3 seconds and 13,200 on TypeSafe in 3.5. The limit a keyless
  caller meets first is the per-IP allowance, 3,000 classifications a minute and 20,000 a day, of
  which one full marker spends 68; the documents now name it, and the hint on an unavailable API
  no longer blames concurrency (`evals/capacity/`).

- How much an answer varies is measured and written down. Five cold reruns of identical bytes
  move 1.6 percent of answers; eight orders of the same candidates move 5.0 percent, and one
  question in four does not survive all eight. Of the degradations, 86 percent become
  abstentions, which a caller can retry or escalate; the rest choose a different handle at
  unchanged confidence, identically on both backends for the same shuffle. The gate scores bound
  none of it and the seven-day answer cache hides it (`evals/variance/`).

Changed:

- Raw input saving can be turned off from the environment, and the store it writes to has a
  bound. `why` and `filter` save their whole stdin, secrets and all, and until now the only way
  to stop them was `--no-save` on each call: a fleet had to remember the flag at every call site,
  and `JEVIFY_NO_CACHE`, which many operators reached for first, governs answers only. Setting
  `JEVIFY_NO_SAVE=1` now stops the saving everywhere it applies, exactly as the flag does for one
  call. The saved inputs are also kept for seven days instead of forever, the same retention as
  the answer cache: a save deletes the store's own files past that age, and saving the same input
  again refreshes its file. The pruning reads the store's `outputs` directory and nothing else —
  no sub-directory, no symlink, and only files named the way the store names its own — so a file
  you put there, or anywhere else, is never deleted. `data.complete` on `why` and `filter` keeps
  its name and its meaning, the completeness of the run's own output, and is now documented as
  such wherever it appears: judgment coverage is `unsure` against `total`, and `why`'s selection
  coverage is `considered` against `total`.

- `fill` can now report whether it started the command. Setting `JEVIFY_STATUS_FILE` to a path
  makes it write one JSON object there before the command replaces it:
  `{command, version, exit_code, ran, argv, reason, markers, error}`. `ran` is true when the
  command started, so the exit code the caller sees is the command's own, and false, or absent,
  when nothing ran. Before this, an abstention and a command that exited 3 were the same exit
  code with nothing but a stderr prefix to tell them apart, which is why a careful integrator
  turned exec mode off and ran `--dry-run --json` instead. The command's exit code is unchanged,
  and so is every other exit code; without the variable nothing is written. A status file that
  cannot be written is exit 6 with the kind `status_file_unwritable`, and the command does not
  start.

- Every `error.kind` is now enumerated in `capabilities`, under `error_kinds` and per exit code,
  so the branches a machine needs are readable from the machine interface. One table in
  `src/exit.rs` feeds all three lists, a library test proves it is exactly what `kind()`
  produces, and a contract test scans the sources for `kind:` literals outside it.

- The overall deadline passing is its own kind, `api_deadline`, apart from the transport failure
  `api_unavailable` on the same exit code 4; a caller that recognized a deadline by its message
  now reads the kind. A request the API rejects points at the token budget only when the
  service's message names a size limit, and otherwise says the request is malformed. The
  per-request connect and read timeouts, 5 and 60 seconds, are published alongside the other
  limits.

## 0.11.0 - 2026-09-24

Changed:

- A `commit` finalist carries its diffstat and the first 1,000 characters of its patch, and the
  `commit` kind no longer lets the subjects round decide alone. A subject line is a claim about a
  change, and the commit making the claim need not be the commit holding it. On twenty adversarial
  cases over two foreign repositories, where a docs-only commit announces a change that a dull
  subject actually holds, the kind answered wrong at 0.82 to 1.00 on twenty of twenty; it now
  answers all twenty right on both backends. On honest history it gains too: the
  `evals/commit-attractor` set moves from 9 to 12 of 27 keyless and from 14 to 22 of 27 on
  TypeSafe, with one fewer wrong answer on each. A single-window `commit` finals now also admits
  the candidates the subjects round scored 0.00, since that score reflects the claim rather than
  the change. The patch costs one extra request only where a single window holds the whole
  history, nothing on a real pool, and all 24 finalists stay inside the keyless budget at an
  average of 1,184 characters each (`evals/commit-subjects/`, `benchmarks/results.md`).

- The quickstart `filter` example passes `--strict` and prints the output it produces. The
  three-way verdict keeps a record that says nothing either way, so the old example returned
  every line it appeared to filter. The rule is now stated on each user-facing page and in the
  capability strings instead of only in `--help`. Default behaviour is unchanged.

## 0.10.0 - 2026-09-22

Changed:

- `fill` always runs the finals of a `file` or `dir` marker. Before, a decisive names round
  with the runner-up out of play decided alone; on the 33 held-out content phrases of
  `evals/fill/finals/` that path chose a file named for the concept and holding something
  else once per twelve to fifteen fires on each backend, and `fill` is the verb whose choice
  reaches a command. The set costs 66 requests instead of 51 to 54 (`benchmarks/results.md`).
  `branch` and `commit` keep the shortcut, since no held-out evidence exists for them; the
  kinds guide and capabilities say which kinds it applies to.

- `meta.decision.round_one` is opt-in: `JEVIFY_DECISION=round_one` adds it to the envelope of a
  verb that ran a tournament, and an ordinary `--json` envelope carries no such field. It held
  every candidate of every window on every run since 0.9.1, so a 1,000-line `why` or a
  1,900-command `route` paid for it whether or not anyone read it. Its `finalists` now lists the
  items the finals request actually held, in its order: `fill` records them after widening a
  single window of names to every candidate with p > 0, `why` after adding the panic lines a
  finalist brings along, `route` after capping the pool at twelve; one window that decided alone
  leaves it empty. Before, it listed the shortlist's picks taken before those steps, so "whether
  it reached the finals" could not be read from it. Capabilities mark the field `round_one?` and
  list `JEVIFY_DECISION`. Exit codes and every other field name are unchanged.

- `filter` gates carry `fails`, P(the record says the statement does not hold), next to `any`
  (P(holds)) and `none` (P(does not say)), so a dropped record's "no" can be reconstructed
  from `meta.decision.gates`. `fails` is null on every other verb. The guides state the
  one-sided verdict rule as the code applies it: yes at P(holds) ≥ threshold + 0.15, no at
  P(does not hold) ≥ the same mark (0.65 by default), unsure otherwise; there is no lower
  bound at threshold minus 0.15.
- benchmarks/results.md labels the three-way `filter` figures as development-set: the three
  wordings were chosen on the 40 cases of both splits, so the validation split is spent for
  `filter`. The 0.9.0 entry below gives the rate as 3 to 6 of the 40 cases.

Documents:

- Every measured figure in the guides is re-derived from the file that produced it, or deleted
  where no file holds it. The routing table reads 34 of 39 author-written requests and 34 of 120
  NL2Bash, with one abstention, dated to the run and the version that produced it; the root-cause
  table reads 18 of 21 at rank one and none abstained. Per-verb token estimates and a
  routing-plus-root-cause dollar total are gone: no file in `evals/` or `benchmarks/` held them.
  The measured PATH is 1,883 commands, not 1,900.

- Three README demonstrations showed answers the binary no longer gives. The commit demo named a
  commit that did not match its description; it is replaced by a piped-candidate `fill` over
  thirty commits that resolves at 0.99. The two-candidate demo abstained about two runs in three;
  a longer description resolves it on ten runs of ten across both backends. The nothing-fits
  demonstration returned a commit on TypeSafe in four runs of four; it now asks for something a
  command-line program cannot hold, and NONE reaches 1.00. `docs/img/fill.svg` is redrawn to the
  command that ships.

- What the threshold decides is stated once, in `docs/guide/how-it-works.md`, and linked rather
  than repeated. It gates P(anything matches); the winner is chosen by a factor-of-two ratio
  against the runner-up and P(NONE), so a marker can resolve on a candidate whose own probability
  sits below the threshold. Three documents and one capability string said otherwise.

Measured:

- `evals/holdout/` holds 103 source-held-out cases over `fill`, `pick`, `why`, `filter` and
  `route`, from repositories, logs and inventories used nowhere else, with the gold apart from
  what a run reads. Both labelling passes are one reader, which the set states rather than
  claiming two annotators.

- The `commit` kind, measured over this repository's 248 commits with descriptions written from
  each target's diff: 11 of 27 right first keyless, 14 of 27 on TypeSafe, against a `branch`
  control of 12 of 14 on both. No commit is an attractor. The failures cluster in the oldest
  window, 0 of 5 on both backends, and the keyless failure mode is abstention with the right
  commit already in the finals.

## 0.9.3

Changed:

- `route` abstains on a near tie. When the runner-up is above the threshold and within 0.10 of
  the best, `route` exits 3, prints no tool, sets `data.tool` to null and names the tied commands
  in `data.ties[{tool,fit}]` (the best first) and on stderr (`too close to tell apart: ...`),
  as VISION promises for two candidates that are too close. The margin was 0.05, below the
  measured 0.06 jitter between identical uncached requests, and a tie still exited 0 with the
  best on stdout. `data.ties` is empty on every exit 0.
- README: the `is` example that printed a message on `||` is gone; it printed "build is fine"
  on exit 3 (unsure) and exit 4 (backend unavailable). Every README example acts on exit 0.

## 0.9.2

Fixed:

- `fill`: a literal prefix scopes the `branch` kind. `'origin/@{branch:x}'` lists that remote's
  refs and substitutes the qualified ref (`origin/ticket/TPE-791`) that `git log`, `git rev-parse`
  and every other revision-taking command resolve; the bare marker keeps substituting the short
  name (`ticket/TPE-791`) that `git switch` takes for a remote-only branch. No one spelling
  satisfies both commands, and the caller's literal is the only signal jevify uses. Capabilities
  state both forms under `kinds[branch].forms`.
- `filter`, `label`, `pick --files`: a file jevify cannot read is never judged by its name. An
  unreadable excerpt (a missing path, a directory in a file's place, a denied file or a denied
  ancestor directory) is missing evidence, not a policy withholding: `filter` and `label` never
  ask about the record, print it unsure with p 0, add `unreadable: REASON` to its `--json` record
  and name it on stderr (`excerpt unreadable: PATH: REASON`); all records unreadable exits 3 with
  no request. `pick` counts and names an unreadable finalist and lets it compete on its name.
  Both kinds share `excerpts_withheld`, so the envelope keeps its shape.

## 0.9.1

Added:

- `meta.decision.round_one` in the envelope: one entry per tournament, in decision order, with
  every window of round one (each candidate by rank, its P(NONE) and Noul), the finalist
  indices the shortlist kept and `n` per window; no extra request. `why`, `pick`, `fill` and
  `route` record it. `index` is the verb's own 1-based number.
- `route` names a near-tie: a tool above the threshold and within 0.05 of the best goes to
  `data.ties[{tool,fit}]` and to stderr (`also fits: ...`); stdout still names the best and a
  tie never abstains. Capabilities and ROBOT_MODE state the rule.

## 0.9.0

Changed:

- `filter` judges each record three ways: the record says the statement holds, says it does not
  hold, or does not say. A record that says nothing either way is unsure and kept, not silently
  dropped. On the 40 adjudicated `filter` cases this takes false actions to zero and keeps
  every record the gold keeps; 3 (TypeSafe) to 6 (classifier.dev) of the 19 gold drops, 16%
  to 32% of them, move from dropped to unsure. The three wordings were compared on both
  splits, so these are development-set figures, not a held-out validation. The threshold and
  the band do not change, and `meta.decision` carries the probability that the record does not
  say under `none`.
- `why` points at the line that carries a panic's message, not at the `panicked at` header: the
  finals judge a panic header next to its message. The adjudicated `why` cases go from 0.80 to
  0.89 accuracy with one false answer fewer.
- `pick` over a plain listing prefers the entry that is or does what the description names over
  a page that documents it, a test of it or a recording of it, unless the description asks for
  one of those. On the 36-case pilot set (`benchmarks/agents/PILOT.md`, 30 of 36 right before
  the rule), four of the six losses are a documentation page chosen over the file that does the
  work; the rule addresses those four. The set is not re-measured after the rule.

## 0.8.3

Added:

- `meta.decision` in the envelope: `verb`, `backend`, `model{requested, answering}` (the
  requested and the answering model kept apart, `unknown` when the service names none),
  `threshold` and `gates[]`, the scores at the gate of every decision (`best`, `next`, `none`,
  `any`, as the verb uses them; one entry per marker, statement, record, hunk, file or pick).
  Existing fields keep their names; no threshold or calibration change.

Changed:

- The `sort` section of the verbs guide holds the filesystem failure matrix for macOS and Linux,
  per condition and per operation, and the tests cover a crash at each journal step, an ENOSPC
  intent write and an undo over the journal a crashed apply leaves behind.
- Opt-in live tests cover `fill` per kind, `label` and `filter` on both backends, from recorded
  fixtures, and report SKIPPED when a key is missing or the free backend answers with another
  model.

## 0.8.2

Fixed:

- `capabilities.selection_limits.finalists_per_window` states the `fill` finalist rule as the
  code applies it: three names per window in the shortlist round, and with one window up to 24
  in the finals for `branch`, `commit`, `file` and `dir` when the names leave the pick
  undecided. `capabilities.exit_codes` names exit 7 `reserved`, never returned, instead of
  `child_failed`. The `--dry-run` example prints `would run`, the `git switch` example names
  `ticket/TPE-791`, and the keyless batch figure is 60 records.
- `fill`: a decisive names round on a `branch`, `commit`, `file` or `dir` marker goes on to the
  finals when its winner does not beat the rest of the field by the winner ratio, so a phrase
  that describes a file's content is decided by the excerpts; a one-sided names round keeps the
  single request, and a finalist whose excerpt is withheld never competes on its name alone.

Changed:

- The README opens with what the reader cannot name and reaches a working command on the first
  screen; `docs/demo/examples.sh` reproduces every example. `scripts/eval_why.py` scores the
  corpus listed in `evals/why/corpus.jsonl`, with provenance and a content hash per case.

## 0.8.1

Fixed:

- `fill --dry-run` ends its stderr with `would run …` instead of `exec …`; only a real run
  prints `exec`.
- `fill` and `pick --from branch` name a branch that exists only on one remote by its short name
  (`ticket/TPE-791`), which `git switch` accepts, instead of `origin/ticket/TPE-791`; the ref
  stays in the evidence.
- `why` shows each finalist next to the nearest failure statement and asks for the failing
  step's own line rather than the first loud one; `data.any` is taken over both rounds.
- A `file` or `dir` marker whose names round is undecided sends every name not ruled out to the
  finals (up to 24), not the top three, so a content phrase whose file ranks low on its name
  reaches the excerpt round instead of losing to a related sibling.

## 0.8.0

Added:

- One overall deadline per verb, `JEVIFY_DEADLINE` seconds (600 by default): a retry wait that
  would end past it is not started, a request still queued or in flight at the deadline is
  cancelled, and the verb ends exit 4 `api_unavailable` naming the deadline. No request is sent
  before a server's `Retry-After` ends.
- `meta.usage{attempted, succeeded, waited{count, total_ms}, cache_hits, tokens{input, output}}`
  under `--json`: a cache hit is a hit and not a request, and a token count left unknown by any
  attempt is `null`, never zero. `telemetry.retry_waits` counts the retry waits started.

Fixed:

- `man` (route's synopsis and descriptions) and `pdftotext` (sort's PDF excerpts) run under a
  5 s deadline with stdin at `/dev/null` and bounded output, through the same poll-and-kill
  runner as the man index; a hung converter leaves the verb to go on without its text.
  `pdftotext` reads the file by path, after the regular-file check, instead of from stdin.
- `filter` and `label` send at most 60 records per classifier.dev request, the largest keyless
  batch tried (`benchmarks/results.md`): the service refuses 75 with HTTP 402 `request_spending_limit` before
  judging anything, so a keyless run over about 70 records ended exit 4 `api_protocol` with the
  raw body. A 402 is reported as exit 4 `api_unavailable` naming the service code, without
  retry. `capabilities.limits.records_per_request.classifier` is 60.
- A marker in the command position (`argv[0]`) is exit 2 with its own message: the command
  must be literal, and `jevify pick --from tool '<description>'` finds it first.
- `fill`'s abstention line names the rival that decided it, `none` included with its
  probability, and never prints an empty field. An empty listing says `no KIND to choose from`
  (reason `no_match`, exit 3, no request).

Changed:

- `dir` finalists carry the names of their first 24 children as round-two evidence; a
  withheld or symlinked directory carries none and counts as withheld.
- `why` and stdin `pick` print their line, candidate and window counts on stderr before the
  first request, like `filter` and `label`.
- `jevify init agents` pairs every situation with a complete command and lists the kinds; the
  skill and the agents guide do the same.
- The keyless quota is measured per verb against the service's own accounting: one
  classification is one record under one question, 3,000 a minute and 20,000 a day per IP.
  `is` costs one per statement, `filter` and `label` one per distinct record, `pick` and `why`
  two per window of 99 lines plus two for the final round, `route` two per window of 99 commands
  plus one per finalist. README, the configuration guide, the FAQ and
  `capabilities.backends` state the free calls a day per verb; `benchmarks/results.md` holds the
  dated measurement. Routing over a PATH of 1,883 commands costs about 52 classifications, not
  one per command.

## 0.7.0

Added:

- `label a,b,c` tags each stdin record with one of the caller's labels and prints
  `LABEL<TAB>RECORD` in input order, `?` for an unsure record. Labels are comma-separated, at
  least two, distinct, at most 99 keyless or 200 on TypeSafe. It shares the record limits and
  batching of `filter`, takes `-0`, `--para` and `--files`, and saves nothing. Capabilities,
  the agent block, the skill and the privacy table name it.
- Marker kinds `commit`, `file`, `dir` and `tool`, coded, and the shipped recipes `pr`, `issue`,
  `ci-run`, `stash`, `process`, `container` and `pod`, read from `src/kinds.jsonl`. `fill` and
  `pick --from` resolve every kind. A recipe is one JSON line (`kind`, `list`, `field` or `key`,
  `ordered`); user recipes come from `kinds.jsonl` under `JEVIFY_CONFIG_DIR` or the platform
  configuration directory, never from a repository, and cannot replace a shipped kind.
- Every lister runs under one 20 s deadline with prompts disabled; a missing or unauthenticated
  tool is `lister_failed` with its own text, and a bad `kinds.jsonl` is `recipe_invalid` with
  its line number. A literal prefix ending in `/` narrows `file` and `dir`. Above the limit an
  ordered kind keeps its newest part and reports `candidates N of M, newest first`; any other
  is `too_many`.
- `capabilities` lists every kind with its lister argv and origin (`coded`, `shipped`, `user`),
  the recipe fields and rules, the withheld path patterns and `JEVIFY_CONFIG_DIR`. The guide
  gains `docs/guide/kinds.md`.

Changed:

- File excerpts drop imports, blank lines and license headers, and keep doc comments. For a
  kind with tier-two evidence, a no-match over names alone gets a second round with excerpts.

Known limits:

- `file` resolves phrases that match a path, and abstains on phrases that describe only a
  file's content (measured 2026-09-22 on jev-1.13.0: "stages hunks" does not reach
  `src/cmd/add.rs`). With 0.8.1, which sends every name not ruled out to the finals, the same
  phrase reaches `src/cmd/add.rs` at 0.68 (measured 2026-09-22 on jev-1.13.0).

## 0.6.0

Added:

- `fill` resolves quoted argument markers and becomes the caller-written command. `branch`
  lists refs, `-` selects from supplied records, and `one` and `flag` judge caller-written
  options against context. `--dry-run` previews argv; machine output requires it.
- All-or-nothing resolution, separate exit-3 marker reasons, and a Jev-only execution guard.
  Missing answering-model names are `unknown` and refuse execution. The command owns its
  output, signals and exit code; consumed stdin becomes empty for it.
- `pick --from branch` returns handles without running a user command; plain `pick` reads stdin.
- Capabilities list marker kinds, exact lister argv, backend capacities, input error kinds
  and abstention reasons. Agent instructions teach quoting, recovery and command permissions.

Changed:

- Selection finalists follow rank within each window: three, two or one as capacity permits.
  `fill` keeps three, accepting 3,267 candidates keyless and 13,200 on TypeSafe; `pick` accepts
  9,801 keyless and 20,000 on TypeSafe. Ordered kinds report retained coverage.

## 0.5.0

Added:

- `filter` selects matching stdin records, preserving bytes and input order. `-v` inverts,
  `-c` counts, and unsure records remain unless `--strict`. Batches hold up to 1,000 records
  on classifier.dev or 20 on TypeSafe; the distinct-record ceiling is 20,000 (`too_many`).
- `is` accepts several statements and `--context FILE`. One statement prints nothing;
  several print verdict-tab-statement lines, with `data.statements` in machine output.
- `pick` and `filter` accept `-0`, `--para` and boolean `--files` reading paths from stdin.
  Hidden and secret-looking file excerpts are withheld. Non-UTF-8 machine records carry
  replacement text, `lossy: true` and `ordinal`.
- `why` and `filter` save full raw inputs under the cache directory's `outputs/`, with
  content-addressed names and private permissions. These files include secrets and are never
  pruned. `--no-save` is separate from `--no-cache`; skipped or failed saves mark incomplete data.
- `init agents` prints at most 25 lines derived from the capabilities command and exit tables.

Changed:

- `route` prints an installed tool, summary and synopsis without starting the user's command.
  Shell integration calls `route`. Capabilities, help and agent documents share this contract.
- `why` reads stdin only, keeps numbered context and accepts no record split option.
- `meta.model` remains a string, joining several answering models with comma and space.
- A `rate_limit_day` HTTP 429 returns exit 4 with no retry. Classifier record batches honour
  numeric `Retry-After` through 60 seconds and refuse longer waits.

Removed:

- `why -- CMD`, `run`, its `--yes`, `--exec`, `--dry-run` and `--no-args` flags, and its
  argument-selection pass. No compatibility aliases are provided.
- `pick --files DIR`; supply paths on stdin instead.
- Global `-v`; use `--verbose` for diagnostics. `filter -v` means inversion.
- The README's obsolete terminal recording reference; the recording files remain available.

Fixed:

- Bind `JEVIFY_BASE_URL` to the active backend's HTTPS host on port 443, with local test endpoints excepted. Reject userinfo and disable redirects for inference, prewarm and health requests so credentials and evidence cannot follow an override to another service.

## 0.4.0 — 2026-09-20

Added:

- `pick --files <DIR> "<intent>"`: choose among the files under a directory by what they are about, and print the path. The first round ranks the path names; the second reads the first 2,000 masked characters of at most 24 finalist files. Inside a git work tree `.gitignore` applies. Hidden entries, symlinks and names that are not UTF-8 are never candidates. `data.source` reports `stdin` or `files`.

Changed:

- The crate, the library, the binary, the `JEVIFY_*` environment variables, the cache directory, the user agent, the agent skill and the repository are named `jevify`. Install with `cargo install jevify --locked`.

## 0.3.4 — 2026-09-20

Changed:

- Releases select a committed version with successful branch CI, scan its changes, and push a signed tag. The shared working tree is never packaged for publication.
- CI verifies the crates.io package and generated cargo-dist workflow. GitHub artifact publication precedes crates.io Trusted Publishing from the same release commit.

## 0.3.3 — 2026-09-20

Fixed:

- Sort apply and undo use atomic no-replace operations. Unique, durable JSONL journals preserve absolute filename bytes and file identity, recover interrupted moves, and report partial failures. Symlink entries are excluded; old TSV journals are rejected. Concurrent source replacement remains unsupported.
- Malformed API decisions return protocol errors instead of panics or false abstentions. Decision-bearing fields are validated before caching and consumption; cache identity includes endpoint and decision-contract version.
- Classifier requests check constructed input, instruction, label and dimension limits locally. Oversized requests fail explicitly rather than silently losing evidence. Clipping includes its marker within the requested limit. Explicit model overrides on classifier are rejected.
- Outbound semantic state and question text share a redaction boundary. Non-secret token-related identifiers retain their meaning. Request telemetry counts attempted inference POSTs, including retries and failures.
- Command displays quote argv for POSIX shells. Closed stdout pipes exit normally instead of panicking.

Changed:

- `run` executes only exact no-argument `true`, `false`, `pwd`, and `ls` forms under the existing confirmation policy. Other grammar stays a proposal with `complete:false` and a blocked reason; trusted PATH is required.
- Oversized `is` input abstains without an API call, with `p:null` and an evidence reason. `add` rejects oversized hunks or batches before classification or staging.
- Documentation and capabilities scope calibration to backend and task. The governing plan separates these repairs from candidate-survival, calibration, recipe, recovery, deadline and record-mode work.

## 0.3.2 — 2026-09-20

Changed:

- Documentation only. The README opens with what jevify is ("`grep` for meaning"), a table of the six commands and a links row, credits Jev from TypeSafe AI and classifier.dev, and the README and guide describe the tool without meta or retrospective language. The package description and keywords match.

## 0.3.1 — 2026-09-20

Changed:

- Help an agent can use without guessing. Bare `jevify` prints a ten-line quick-start card (still exit 2, on stderr) where it printed the full help. Each verb's `--help` has examples, its exit codes and its `--json` fields, and the free-text arguments say how to phrase them. `capabilities` gains `use_when`, `output`, `phrasing`, more `workflows`, and a `when` and an `example` per verb. The handbook (`jevify robot-docs`) opens with when to call jevify, how to phrase, and patterns.
- The Claude Code plugin lives in `plugins/jevify/`, so an install copies the skill and its manifest, not the repository. The skill file is now `plugins/jevify/skills/jevify/SKILL.md`.
- README and guide: plain descriptions of the six verbs, examples run on the keyless backend, animations of `run` and `sort`, and an agent section backed by `benchmarks/agents/`.

## 0.3.0 — 2026-09-19

Added:

- **No key needed.** With no TypeSafe key, jevify asks [classifier.dev](https://classifier.dev), which runs the same Jev model and serves it free, with no key and no account. Same verbs, same calibrated probabilities, same exit codes, same JSON envelope; `meta.backend` and `jevify health` name the backend that answered, and `capabilities.backends` lists both with their limits. `JEVIFY_BACKEND=typesafe|classifier` forces either. A key still gets you your own TypeSafe quota, and is what `typesafe` requires.
- Measured, not assumed: on the two accuracy evals the backends score the same. Routing, hand-written set: 34/39 top-1 on both. NL2Bash held-out: 34/120 on classifier.dev, 33/120 on TypeSafe. Root cause: 14/20 hit@1 and 15/20 hit@3 on both.
- On classifier.dev the free service's own limits apply: a question takes at most 100 options, so the tournament windows at 99 plus NONE; an input takes 32,000 characters; a request takes 20 questions, and jevify splits bigger asks. `meta.input_tokens` and `meta.cost_usd` are `0` there, because nothing is charged. Default concurrency is 4 rather than 8, the user agent is `jevify/<version>`, and `Retry-After` is honoured.

## 0.2.0 — 2026-09-19

Wave 2: two verbs that act on real things, each safe by default. macOS and Linux.

Verbs:

- `add "<topic>"`: score each unstaged hunk of tracked files against a topic and stage the ones about it; `--dry-run` only scores, `--yes` skips the question; works from any subdirectory of the repo; exit 3 when no hunk is about the topic, exit 6 when there are no unstaged changes.
- `sort <dir>`: propose a home among the existing folders under `dir` (or `--into <root>`), up to two levels deep, for each file directly in `dir`; `--apply` moves the files and writes an undo log, `--undo <log>` moves them back; exit 3 when nothing can be placed, exit 6 when there are no folders to sort into.

Safety:

- `add` touches the index only: it never commits, never stages untracked files or binary changes, and in machine mode stages only with `--yes` (otherwise exit 130 and nothing is staged).
- `sort` is a dry run unless `--apply`: it never overwrites a file, never deletes one, moves within one volume only, and `--undo` restores every file whose original path is still free.

Agent surface: `capabilities`, `robot-docs`, README and PRIVACY.md list both verbs; PRIVACY.md says what each sends (`add`: the topic and each unstaged hunk, clipped; `sort`: file names, the first 2,000 characters of each text file or of a PDF's first two pages, and the folder names under the root).

Speed: `run` opens its API connection while it reads the tool inventory, about 200 ms at p50 on `run cold full` (ABBA A/B in `benchmarks/README.md`); the other verbs have no local work to overlap and stay without prewarm.

## 0.1.0

First release. macOS and Linux.

Verbs:

- `pick "<intent>"`: print the stdin line(s) that match an intent; `-n N`, `--index`; exit 3 when nothing fits.
- `why`: point at the root-cause line in failing output, from stdin or by running the command (`why -- <cmd>`); `-C N` context, `-n N` causes.
- `run "<intent>"`: route to an installed tool, point at flags from its man page, confirm, run; `--dry-run`, `--yes`, `--no-args`, `--exec` (machine mode).
- `is "<condition>"`: exit 0 yes, 1 no, 3 unsure; `--band` (default 0.15).

Agent surface:

- `capabilities` (`--json`): commands, flags, exit codes, env, limits, safety rules.
- `robot-docs [guide|commands|exit-codes|examples|privacy]`: the agent handbook.
- `health`: key and API reachability.
- `init zsh|bash`: the `,` alias for `jevify run`, plus an opt-in command-not-found hook (`JEVIFY_CNF=1`).
- `--json` / `--robot` / `--format json|jsonl|toon`: exactly one envelope on stdout (`ok, command, version, exit_code, data, meta, error`), usage errors included.
- Exit codes: 0 ok, 1 no, 2 usage, 3 abstain, 4 unavailable, 5 auth, 6 input, 7 child failed, 130 declined.

Safety:

- `run` executes only after a TTY confirmation or `--yes`; in machine mode only with `--exec --yes`, the child's stdout redirected to stderr.
- Never-execute list, by tool name: `rm`, `dd`, `mkfs*`, `sudo`, `kill`, `shutdown` and the rest of the destructive set, the shell and process wrappers (`sh`, `bash`, `env`, `xargs`, `find`, `timeout`, ...) and the script interpreters (`python*`, `perl*`, `ruby*`, `node*`, `php*`, `lua*`). These are shown, never run.
- Commands run via argv, never a shell; flags come from man pages, no binary is probed with `--help`.
- Obvious secrets are masked before text leaves the machine (best effort; see PRIVACY.md).

Model and answers:

- Default model pinned to `jev-1.13.0`; the 0.5 threshold was calibrated on it. `--model jev-latest` / `JEVIFY_MODEL` allowed and documented as moving.
- One threshold (`-t`, `JEVIFY_THRESHOLD`, default 0.5) on absolute yes/no answers; "which one" answers must beat NONE.
- Tournament past 255 options (windows of 200 + NONE, 3 finalists per window, one finals round), 60,000-character window budget, at most 2 rounds per verb (3 for `run`).
- Disk cache of answers keyed by request hash, 7-day TTL; `--no-cache`, `JEVIFY_NO_CACHE`.
- Retries on 408/429/5xx/timeouts up to 3 times, honouring `retry-after`; 413/422 reported as input errors (`api_rejected_request`, exit 6).

Benchmarks (`benchmarks/`): per-verb p50/p95 with conditions; the connection prewarm was measured (17 ms at p50 on `pick`) and removed.
