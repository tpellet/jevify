# Run the test you can describe

`fill` with a supplied list runs the one test that matches a behaviour, out of a test runner's
own listing. The listing is the evidence; the handle is the test name; the command is the
runner's. Nothing runs on an abstention.

## The recipe

Cargo, from the repository root. `--list` prints `name: test` per test on stdout and the per-binary
`Running …` headers on stderr, so `sed` keeps only the names; `--exact` after the second `--` makes
the chosen name a whole-name filter in every test binary:

```sh
cargo test -- --list 2>/dev/null | sed -n 's/: test$//p' \
  | jevify fill -- cargo test '@{-:a 429 response is retried and the answer is cached}' -- --exact
```

The names are what `cargo test` accepts: unit tests keep their module path
(`marker::tests::literal_table_preserves_bytes_and_decodes_only_the_escape`), integration tests
are bare (`a_429_is_retried_and_the_answer_is_cached`). Ignored tests are listed too, so a
chosen `#[ignore]` test runs as `0 passed; 1 ignored` unless the command adds `--ignored`.

pytest, from the directory the suite is collected in. `--collect-only -q` prints one node id per
test followed by a blank line and a `N tests collected` summary; `sed` keeps the node ids, which
pytest accepts as arguments (`uvx pytest` on both sides when pytest is not on PATH):

```sh
pytest --collect-only -q | sed -n '/::/p' \
  | jevify fill -- pytest '@{-:the backoff is capped at the maximum}'
```

Go: `go test -list '.*' ./pkg/` prints one test name per line and a trailing `ok  pkg  0.01s`
line; `sed -n '/^Test/p'` keeps the names, and the command is
`go test -run '^@{-:…}$' ./pkg/`.

Each marker is one request on TypeSafe for a listing under 13,200 names (3,267 keyless); the
listing is not ordered, so a longer one is `too_many` (exit 6). The `-- --exact` tail reaches
`cargo` untouched: `fill` reads the command after its own first `--`.

## Gold set

`test-by-behaviour-gold.tsv`: 50 behaviours, each written before the run with the test it
names, or `NONE` when no test covers it.

- 40 on this repository (`cargo test -- --list`, 190 tests, jevify 0.14.2 at `fef44ad`):
  36 with one gold test across every verb, the backend client, redaction, the release script
  and the transcripts check; 4 behaviours with no test (`NONE`).
- 10 on `sharkdp/hyperfine` at `f12f3d9`, the clone `tests/e2e.rs` pins (`cargo test -- --list`,
  102 tests): 9 with one gold test; 1 with no test.

A behaviour is a sentence about what the test checks, in different words from the test name
where a paraphrase exists (`a doubled @@ keeps its bytes` for `literal_table_preserves_bytes…`),
never the name itself.

## Method

Each row runs

```sh
JEVIFY_NO_CACHE=1 jevify fill --dry-run --json --candidates <listing> -- cargo test '@{-:<behaviour>}' -- --exact
```

in the repository the row names, so every case is one live request and no `cargo test` time
enters the latency. The score reads the envelope: exit 0 with `data.markers[0].handle` equal to
the gold is a hit; exit 3 on a `NONE` row is a correct abstention; exit 3 on a row with a gold is
an abstention miss; exit 0 with any other handle is wrong and would run that test. Latency is
`meta.elapsed_ms`, requests `meta.requests`.

## Results

| | Count |
|:---|---:|
| Hit (the gold test, would run) | 43 |
| Abstain, correct (`NONE` row, nothing runs) | 5 of 5 |
| Abstain, miss (a gold exists, nothing runs) | 2 |
| Wrong (another test chosen) | 0 |
| Wrong and would run | 0 |

43 of 45 behaviours with a test run that test; every behaviour without a test runs nothing. The
two misses are abstentions, not wrong runs:

- `j14` "label prints a question mark for unsure records, ties and none": the gold
  `unsure_records_ties_and_none_are_question_marks` scores 0.38 against `none` 0.57. The
  behaviour reads as the verb's whole contract, and the listing holds three `label` tests.
- `h08` "the default shell on this platform": `options::test_default_shell` scores 0.78, but the
  any-match gate reads 0.44, under the threshold. A behaviour stated as a topic rather than a
  check reads as not being on the list.

Both answer to the status line's hint: describe what the test checks, in its own words. The
nearest names appear in the `jevify fill:` status line on stderr (`nearest (not chosen)`), not
in `data.shortlist`, which `fill` leaves null on exit 3.

Hits score 0.78 to 1.0, median 0.99. Every row is one request: 190 or 102 candidates fit one
TypeSafe window.

| Latency, `meta.elapsed_ms` | n | min | p50 | p95 | max | mean |
|:---|---:|---:|---:|---:|---:|---:|
| this repository (190 candidates) | 40 | 202 | 252 | 346 | 381 | 259 |
| hyperfine (102 candidates) | 10 | 191 | 236 | 354 | 354 | 242 |
| all | 50 | 191 | 252 | 350 | 381 | 255 |

p50/p95 are order statistics over the rows (nearest rank). `cargo test -- --list` itself takes
about 0.3 s on a built tree in this repository and a full build on a cold one; the recipe spends
that before jevify starts.

## Conditions

| | |
|:---|:---|
| Date | 2026-10-02 |
| Backend | TypeSafe, `jev-1.13.0`, `JEVIFY_NO_CACHE=1`, `--dry-run` |
| jevify | 0.14.2, `cargo build --release`, rustc 1.93.1 |
| Machine | a typical macOS dev machine: Apple M4 Pro, macOS 26.6.2, consumer Wi-Fi |
| Listings | this repository: 190 names; hyperfine `f12f3d9`: 102 names |

## Per case

Chosen is the handle `fill` substitutes; `(nothing)` is exit 3.

| Case | Behaviour | Chosen | p | ms | Score |
|:---|:---|:---|---:|---:|:---|
| h01 | benchmarks run one after another, never interleaved | `benchmarks_are_executed_sequentially` | 0.98 | 196 | hit |
| h02 | the reference command runs before the other commands | `reference_is_executed_first` | 0.99 | 236 | hit |
| h03 | a failing command passes with the ignore-failure option | `can_run_failing_commands_with_ignore_failure_option` | 0.98 | 212 | hit |
| h04 | outliers are detected when the MAD is zero | `outlier_detection::test_detect_outliers_if_mad_becomes_0` | 0.89 | 238 | hit |
| h05 | results export as CSV | `export::csv::test_csv` | 0.87 | 219 | hit |
| h06 | the shell spawning time counts when computing the number of runs | `takes_shell_spawning_time_into_account_for_computing_number_of_runs` | 1.0 | 294 | hit |
| h07 | a parameter scan over decimal values | `command::test_parameter_scan_commands_decimal` | 0.88 | 354 | hit |
| h08 | the default shell on this platform | `(nothing)` | - | 191 | abstain, miss |
| h09 | a missing stdin data file is an error | `fails_if_invalid_stdin_data_file_provided` | 0.93 | 195 | hit |
| h10 | results export as a PNG chart | `(nothing)` | - | 239 | abstain, correct |
| j01 | a secret file's content never leaves the machine when fill reads file excerpts | `file_finals_read_excerpts_and_secret_files_never_leave_the_machine` | 0.78 | 283 | hit |
| j02 | a 429 response is retried and the answer is cached | `a_429_is_retried_and_the_answer_is_cached` | 1.0 | 266 | hit |
| j03 | a redirect never receives the key or the evidence | `redirects_never_receive_key_or_evidence` | 1.0 | 241 | hit |
| j04 | the TypeSafe key is never sent to the keyless backend | `the_typesafe_key_never_reaches_the_keyless_backend` | 0.99 | 269 | hit |
| j05 | each HTTP status maps to an exit code and an error kind | `http_statuses_map_to_exit_codes_and_kinds` | 1.0 | 297 | hit |
| j06 | an unsure flag marker runs nothing | `a_flag_is_kept_or_removed_and_an_unsure_flag_runs_nothing` | 0.99 | 206 | hit |
| j07 | the status file records whether the command ran | `the_status_file_says_whether_the_command_ran` | 1.0 | 220 | hit |
| j08 | a commit handle is the full oid and a long history keeps the newest commits | `a_commit_is_a_full_oid_and_a_long_history_keeps_the_newest` | 1.0 | 269 | hit |
| j09 | git -C inside the wrapped command decides where the listers run | `git_dash_c_in_the_command_is_where_the_listers_run` | 0.87 | 233 | hit |
| j10 | why points at the root cause and prints its context lines | `points_at_the_root_cause_with_context` | 0.92 | 252 | hit |
| j11 | why on a log without any failure signal exits 3 with a hint | `no_signal_on_stdin_is_exit_3_with_a_hint` | 0.86 | 229 | hit |
| j12 | pick abstains on a near tie between two stdin records | `stdin_near_tie_abstains_without_printing_a_record` | 0.88 | 264 | hit |
| j13 | pick --from branch prints the local handle when a remote twin exists | `from_branch_remote_twin_uses_local_handle` | 1.0 | 252 | hit |
| j14 | label prints a question mark for unsure records, ties and none | `(nothing)` | - | 245 | abstain, miss |
| j15 | label refuses more labels than the window holds before any request | `label_count_is_checked_against_the_window_before_any_request` | 0.94 | 228 | hit |
| j16 | filter returns each line byte-exact after the first tab | `lines_come_back_byte_exact_after_the_first_tab` | 0.94 | 246 | hit |
| j17 | is keeps several statements in order and aggregates them into one verdict | `statements_keep_their_order_and_aggregate_the_verdict` | 0.79 | 220 | hit |
| j18 | is with --context FILE replaces stdin and a bad file makes no request | `context_file_replaces_stdin_and_its_errors_make_no_requests` | 0.99 | 270 | hit |
| j19 | add stages only the hunks that match the topic | `stages_only_matching_hunks` | 0.93 | 312 | hit |
| j20 | add on a clean tree is an input error with a hint | `clean_tree_is_an_input_error_with_a_hint` | 0.96 | 381 | hit |
| j21 | a person declining at the add prompt stages nothing | `terminal_decline_and_apply_rejection_stage_nothing` | 0.97 | 271 | hit |
| j22 | health exits 5 without a key | `health_is_5_without_a_key_and_0_after_uncached_classification` | 1.0 | 226 | hit |
| j23 | health reports billing, quota and auth errors | `health_reports_billing_quota_and_auth_errors` | 1.0 | 252 | hit |
| j24 | when the daily quota runs out the already answered prefix is kept | `daily_quota_keeps_the_answered_prefix` | 1.0 | 350 | hit |
| j25 | the deadline cancels a request that is still in flight | `the_deadline_cancels_a_request_in_flight` | 0.99 | 242 | hit |
| j26 | a usage error is exit 2 with one envelope and a corrected example | `usage_errors_are_exit_2_and_one_envelope_with_a_corrected_example` | 1.0 | 249 | hit |
| j27 | verb options written before the verb still belong to the verb | `verb_options_before_the_verb_are_read_as_the_verbs` | 0.99 | 346 | hit |
| j28 | clap errors keep non-UTF-8 arguments intact | `clap_errors_preserve_non_utf8_args_and_stop_format_scanning_at_double_dash` | 1.0 | 220 | hit |
| j29 | the binary never reads the platform configuration directory | `bin_never_reads_the_platform_configuration_directory` | 1.0 | 220 | hit |
| j30 | a tool is chosen from the inventory and man pages without running it | `tool_kind_uses_inventory_and_man_evidence_without_running_the_tool` | 1.0 | 256 | hit |
| j31 | a man process that sleeps is killed at the deadline | `manpage::tests::a_sleeping_man_is_killed_at_the_deadline_and_a_quick_one_is_read` | 1.0 | 257 | hit |
| j32 | secrets are redacted while ordinary tokens survive | `input::tests::secrets_are_redacted_and_ordinary_tokens_survive` | 1.0 | 202 | hit |
| j33 | a doubled @@ keeps its bytes and only the escape is decoded | `marker::tests::literal_table_preserves_bytes_and_decodes_only_the_escape` | 0.96 | 252 | hit |
| j34 | the release script refuses a version that is already tagged | `release_refuses_unverified_or_existing_versions_before_tagging` | 1.0 | 281 | hit |
| j35 | the documented transcripts still answer as the pages show on TypeSafe | `transcripts_still_answer_as_the_pages_show_on_typesafe` | 0.98 | 257 | hit |
| j36 | pruning old saved inputs never removes the outputs directory itself | `save::tests::pruning_never_leaves_the_outputs_directory` | 0.89 | 228 | hit |
| j37 | jevify refuses to run when the system clock is behind | `(nothing)` | - | 306 | abstain, correct |
| j38 | a Windows path with a drive letter is normalized | `(nothing)` | - | 258 | abstain, correct |
| j39 | an HTTPS_PROXY variable routes requests through the proxy | `(nothing)` | - | 280 | abstain, correct |
| j40 | the answer cache is encrypted at rest | `(nothing)` | - | 258 | abstain, correct |
