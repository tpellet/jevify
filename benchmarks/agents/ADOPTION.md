# Adoption, measured apart from efficacy, on tasks a literal search cannot answer

`benchmarks/agents/STUDY.md` records a harness and twelve runs that validate it.
Those twelve runs found two faults in the study's *design*, not in the harness:
both arms answered 12 of 12, so no result was possible; and the arm that had
jevify called it zero times, so the two arms were the same experiment run twice.
This is the study rebuilt around both faults, and its headline number is the one
the twelve runs could not produce.

**An agent given jevify and told about it once reached for it in 11 of 16 runs,
69 %, 95 % Wilson [0.44, 0.86].** Zero of six became eleven of sixteen when the
tasks stopped being ones `git log --grep` answers.

Measured 2026-09-27 on a macOS dev machine, jevify 0.13.0 (`~/.cargo/bin/jevify`,
sha256 `9e2cfa88e867ce1a…`), keyless against classifier.dev, answering model
`jev-1.13.0`. Agent model `claude-haiku-4-5`, one agent process per cell, study
tree `~/jevify-study-d`. 48 cells: 8 tasks x 3 arms x n = 2.

## 1. The task set, and the proof that it is not lexically reachable

A question `git log --grep` answers is a question jevify cannot improve. The old
set was written to avoid keyword overlap and then never checked, and it turned out
to be answerable by an agent reading a list of 120 commit subjects. So the rule
here is stronger and it is enforced by a script rather than by intention:

> Before a task enters the set, run the searches a competent engineer would try,
> from the question's own words, and record what each one returns. A search that
> returns the gold among at most three candidates disqualifies the task.

`scripts/study/baseline.py` is that gate. Per task it derives every content word
of the question (dropping the harness's framing and ordinary English) and runs
each one through `git log --grep`, `git log -S`, `git log --all --grep`,
`git grep -il` and `git ls-files`; for a branch task it also reads the subject of
every remote ref and maps every commit a `--grep` returns back to the branches
that contain it, the way an engineer would. On top of that it runs a handful of
identifiers a reader of the question might guess at, written down per task by hand
and kept in the record whether they succeed or fail — `git log -S'#[default]'`,
`git grep -il Replacer`, `git log -S'math.MaxInt32'`. Each probe is scored:

- **miss** — returned nothing, or nothing naming the gold
- **buried** — the gold is in the output, among more than three candidates
- **HIT** — the gold is in the output and there are at most three candidates

**Fourteen candidate tasks were built; four were disqualified.** The full record,
every probe with its command and its count, is in
`benchmarks/agents/lexical-baseline.json`.

| task | probes | miss | buried | HIT | the tightest probe that saw the gold at all | verdict |
|:---|---:|---:|---:|---:|:---|:---|
| D1 | 29 | 23 | 6 | 0 | 25 candidates, `git log --all -i --grep=nan` | kept |
| D2 | 31 | 23 | 8 | 0 | 13 candidates, `git log --all -i --grep=iteration` | kept |
| D3 | 56 | 53 | 2 | **1** | **1 candidate, `git log -S'#[default]'`** | dropped |
| D4 | 43 | 39 | 4 | 0 | 4 candidates, `git log -S sort_order` | kept |
| D5 | 59 | 57 | 2 | 0 | 11 candidates, `git log -S 'map(\|r\|'` | kept |
| D6 | 19 | 12 | 5 | **2** | **1 candidate, `git grep -il machinery`** | dropped |
| D7 | 23 | 22 | 0 | **1** | **2 candidates, `git grep -lw Core`** | dropped |
| D8 | 25 | 24 | 1 | 0 | 14 candidates, `git grep -il Slab` | kept |
| D9 | 55 | 53 | 2 | 0 | 17 candidates, `git log -S math.MaxInt32` | kept, then dropped for cost (§3) |
| D10 | 25 | 16 | 9 | 0 | 7 candidates, `git log --all -i --grep=comparison-based` | kept |
| E1 | 51 | 49 | 2 | 0 | 10 candidates, `git log -- src/benchmark/relative_speed.rs` | kept |
| E2 | 59 | 57 | 2 | 0 | 8 candidates, `git log -S into_iter` | kept, surplus, not run |
| E3 | 28 | 21 | 6 | **1** | **1 candidate, `git branch -r --list '*ci*'`** | dropped |
| E4 | 35 | 29 | 6 | 0 | 4 candidates, `git grep -il deserializ` | kept |

D6 is the instructive failure: the question said "buffer machinery" and
`git grep -il machinery` returned the answer and nothing else, because the word
was in the file. The gate caught a leak the author did not see.

### The eight tasks that were run

Every gold answer is a commit hash, a repository-relative path or a branch name at
a pinned sha, so a third party scores it without knowing which arm produced it.
The recorded lexical failure for each is the row above; what makes the task hard
is in the last column.

| task | repository at its pin | kind | gold | why no label answers it |
|:---|:---|:---|:---|:---|
| D1 | hyperfine `f12f3d9f` | branch | `fix-319` | 28 remote branches; this one's name is an issue number. The tip subject is "Fix nan/inf output for very fast commands" and the question says "meaningless figures … too quickly to measure". |
| D2 | hyperfine `f12f3d9f` | branch | `fix-771` | same, name is an issue number; tip "Add iteration information to failure error message". |
| D4 | hyperfine `f12f3d9f` | commit | `038f369` | subject is "SortOrder field": two words, no verb, no module. |
| D5 | ripgrep `3fce3b5b` | commit | `5e2d32f` | subject is "printer: slightly simplify code": the crate and nothing else. |
| D8 | fzf `b1be3a8b` | path | `src/util/slab.go` | 12 lines, no comments at all, so no word of the question can occur in it; 59 non-test Go files. |
| D10 | fzf `b1be3a8b` | branch | `perf` | the name states a goal, not the work; tip "Use LSD radix sort for Result sorting in matcher". |
| E1 | hyperfine `f12f3d9f` | commit | `98d262c` | subject is "Fix warnings". |
| E4 | ripgrep `3fce3b5b` | path | `crates/printer/src/jsont.rs` | the name is a contraction; the neighbour `json.rs` is the decoy and two probes returned it instead. |

## 2. Three arms

Adoption and efficacy are different questions and the two-arm design conflated
them. `available` and `required` share one Seatbelt profile; the prompt is the
only difference between them.

| arm | jevify | the one paragraph that differs | what it answers |
|:---|:---|:---|:---|
| `control` | denied exec | "The tool jevify is not installed on this machine." | the floor |
| `available` | allowed exec | where the binary is, plus the output of `jevify init agents` | does an agent reach for it |
| `required` | allowed exec | the same, plus "For this task you must use jevify for the step that chooses among candidates. Run it at least once and let what it returns decide your answer." | does it help when used |

The `available` paragraph is what a real integrator writes: the tool exists, here
is the path, here is its own documentation. It does not argue for the tool, does
not say the task suits it and does not mention the words of the question. Nudging
it would have made the adoption figure meaningless, which is the whole reason the
arm exists.

Every arm gets `Bash`, `Read`, `Grep` and `Glob` and may run whatever it likes.
The three are reported below separately and never averaged.

## 3. What was run, and what it cost

**n = 2, not 5 and not 3.** The harder task set costs materially more per run than
the projection in `STUDY.md` assumed, and the overrun is not uniform: it is one
task. The pricing there was a median of 0.028 USD for a with-arm run and 0.036 for
a control run. Measured here, the median run costs 0.027 (`available`), 0.044
(`control`) and 0.055 (`required`), and the tail is what breaks the budget:
`E1-control-r1` spent 56 turns, 2.4 M input tokens and 0.45 USD; `D5-required-r1`
spent 44 turns and 0.24 USD.

**D9 was dropped for cost after its first three cells**, not for reachability — its
baseline is clean. `D9-control-r1` ran 110 turns and exhausted the whole 1 USD
per-cell budget without answering. `D9-available-r1` ran 92 turns, made 23 jevify
calls and 892 keyless requests, 1,784 classifications, 9 % of one IP's daily
allowance in a single run. Those three cells cost about 1.8 USD between them.
Dropping a task after seeing a result is a real hazard, so this is on the record:
D9 was dropped on its resource profile, before any of its cells were scored, and
its three completed cells are excluded from every figure below by task id.

Spend, against an authorization of roughly 4 USD and 8 % of the keyless daily
allowance for 120 runs:

| | agent inference | keyless classifications |
|:---|---:|---:|
| the 48 scored cells | 3.18 USD | 548 (2.7 % of a day) |
| the three abandoned D9 cells and 6 pilot cells | 1.96 USD | 2,034 (10.2 %) |
| **everything this study spent** | **5.14 USD** | **2,582 (12.9 %)** |

That is 29 % over the dollar authorization and 61 % over the classification
authorization, and D9 is 80 % of the overrun. 120 runs was never reachable: at the
measured medians it would have cost about 8 USD before any D9-shaped tail.
n = 3 over the eight kept tasks would have cost about 5 USD for the cells alone,
7 USD in total. n = 2 buys 16 runs per arm, which is enough for an arm-level
interval and not enough for a cell-level one.

## 4. Results

48 cells, every one scored, none refused for a leak. `used` is whether jevify
named the gold *and* the agent answered it; `mtime` is the smoke-test count
discussed in §7 and is not a count of writes.

```
rid                        status        ok  turns tools   in_tok  wall_s   agent$  jev  req  used  mtime
D1-control-r1              scored        1       4     3    42705    13.5   0.0173    0    0     -      0
D1-control-r2              scored        1       5     4    53705    14.6   0.0174    0    0     -      0
D1-available-r1            scored        1       3     2    34003    13.3   0.0149    1    1   yes      2
D1-available-r2            scored        1       3     2    33751    12.3   0.0140    1    1   yes      0
D1-required-r1             scored        1      10     9   132969    33.4   0.0394    5    4   yes      0
D1-required-r2             scored        1       3     2    34208    13.2   0.0170    1    1   yes      1
D2-control-r1              scored        1      21    20   332408    47.1   0.0858    0    0     -      1
D2-control-r2              scored        1      10     9   113321    23.8   0.0308    0    0     -      0
D2-available-r1            scored        1       6     5    72461    18.3   0.0246    1    1   yes      0
D2-available-r2            scored        1       7     6    86269    18.9   0.0263    0    0     -      1
D2-required-r1             scored        1       4     3    45939    15.7   0.0169    1    2   yes      0
D2-required-r2             scored        1       7     6    85572    21.8   0.0266    1    1   yes      0
D4-control-r1              scored        1       5     4    56607    17.7   0.0212    0    0     -      0
D4-control-r2              scored        1       7     6    83612    16.8   0.0276    0    0     -      0
D4-available-r1            scored        1       5     4    71448    24.2   0.0281    1   12   yes      0
D4-available-r2            scored        0       7     6    87832    31.4   0.0312    2   12    no      0
D4-required-r1             scored        1      17    16   273953    46.5   0.0632    2   24   yes      4
D4-required-r2             scored        1       9     8   126350    30.7   0.0400    1   12   yes      0
D5-control-r1              scored        1      12    11   218379    39.2   0.0666    0    0     -      0
D5-control-r2              scored        1      15    14   243850    43.0   0.0659    0    0     -      0
D5-available-r1            scored        1      11    10   223310    49.5   0.0766    1   25    no      2
D5-available-r2            scored        1       4     3    46638    20.3   0.0195    1   25   yes      0
D5-required-r1             scored        1      44    43  1197818   131.2   0.2383    2   50    no      3
D5-required-r2             scored        1      26    25   553323    84.0   0.1305    2    2   yes      0
D8-control-r1              scored        1       8     7   164445    26.0   0.0699    0    0     -      1
D8-control-r2              scored        1       6     5   113949    19.1   0.0562    0    0     -      0
D8-available-r1            scored        1       7     6   124956    21.2   0.0631    0    0     -      0
D8-available-r2            scored        1       9     8   161282    26.8   0.0683    0    0     -      0
D8-required-r1             scored        1      22    21   771435    74.5   0.1872    3    8   yes      1
D8-required-r2             scored        1      14    13   206695    41.9   0.0559    4    6   yes      0
D10-control-r1             scored        1       3     2    30876    10.0   0.0115    0    0     -      0
D10-control-r2             scored        1       4     3    44953    15.4   0.0203    0    0     -      0
D10-available-r1           scored        1       4     3    45798    14.0   0.0133    1    1   yes      0
D10-available-r2           scored        1       4     3    45782    14.0   0.0179    2    1   yes      0
D10-required-r1            scored        1       3     2    33703    10.8   0.0109    1    0   yes      2
D10-required-r2            scored        1       5     4    58993    15.7   0.0219    1    1   yes      0
E1-control-r1              scored        0      56    55  2418484   220.9   0.4452    0    0     -      4
E1-control-r2              scored        1      33    32   824314    90.1   0.1753    0    0     -     47
E1-available-r1            scored        0      35    34  1131196   138.0   0.2455    3    0    no      2
E1-available-r2            scored        1      21    20   397928    58.1   0.0953    0    0     -      0
E1-required-r1             scored        1      12    11   191071    52.7   0.0544    2   24   yes      0
E1-required-r2             scored        1      24    23   507246    90.8   0.1168    3   36   yes      0
E4-control-r1              scored        1       8     7   131871    29.2   0.0658    0    0     -      0
E4-control-r2              scored        1       5     4    58807    18.2   0.0269    0    0     -      0
E4-available-r1            scored        1       5     4    64354    16.1   0.0251    0    0     -      0
E4-available-r2            scored        1       8     7   112199    28.0   0.0420    2    3    no      0
E4-required-r1             scored        1      10     9   202519    37.9   0.0729    3    6   yes      0
E4-required-r2             scored        1      10     9   222223    35.3   0.0789    1    2   yes      0
```

### Correctness per cell

At n = 2 a cell's interval is [0.34, 1.00] for 2 of 2 and [0.09, 0.91] for 1 of 2.
No two cells separate; the table is here because the count without the interval
would invite a reader to think they do.

| cell | k/n | 95 % Wilson | | cell | k/n | 95 % Wilson |
|:---|:---:|:---|---|:---|:---:|:---|
| D1 control / available / required | 2/2 each | [0.34, 1.00] | | D8 control / available / required | 2/2 each | [0.34, 1.00] |
| D2 control / available / required | 2/2 each | [0.34, 1.00] | | D10 control / available / required | 2/2 each | [0.34, 1.00] |
| D4 control | 2/2 | [0.34, 1.00] | | E1 control | 1/2 | [0.09, 0.91] |
| D4 available | 1/2 | [0.09, 0.91] | | E1 available | 1/2 | [0.09, 0.91] |
| D4 required | 2/2 | [0.34, 1.00] | | E1 required | 2/2 | [0.34, 1.00] |
| D5 control / available / required | 2/2 each | [0.34, 1.00] | | E4 control / available / required | 2/2 each | [0.34, 1.00] |

### Correctness and resources per arm

16 runs each, 8 tasks x 2 repetitions. Sums over the arm, medians in brackets.

| arm | k/n | 95 % Wilson | turns | input tokens | wall | agent USD |
|:---|:---:|:---|---:|---:|---:|---:|
| `control` | 15/16 | [0.72, 0.99] | 202 [8] | 4,932,286 [113,635] | 644.6 s [21.5] | 1.2036 [0.0435] |
| `available` | 14/16 | [0.64, 0.97] | 139 [6] | 2,739,207 [79,365] | 504.4 s [20.8] | 0.8058 [0.0272] |
| `required` | 16/16 | [0.81, 1.00] | 220 [10] | 4,644,017 [196,795] | 736.1 s [36.6] | 1.1707 [0.0551] |

Keyless backend usage, which is jevify's own cost and not the agent's: `control`
0 calls, `available` 16 calls / 82 requests / 164 classifications, `required` 33
calls / 179 requests / 384 classifications.

### Adoption, which is the number this study exists to get

**`available`: 11 of 16 runs called jevify at least once. 69 %, 95 % Wilson
[0.44, 0.86].**

| task | D1 | D2 | D4 | D5 | D8 | D10 | E1 | E4 |
|:---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| runs that called it | 2/2 | 1/2 | 2/2 | 2/2 | **0/2** | 2/2 | 1/2 | 1/2 |

D8 is the only task where the agent never reached for it in either run, and it is
the task with the smallest candidate set: one file among fifteen in `src/util`.
Both runs answered correctly with `Glob` and a `Read`. That is the shape of
non-adoption on this evidence — not refusal, but a candidate set small enough that
reading it is obviously cheaper.

### The `required` arm: called, and believed

| | count |
|:---|:---|
| runs that called jevify | 16/16 |
| runs where jevify named the gold and the agent answered it | 15/16 |
| runs where jevify named the gold and the agent answered otherwise | 0/16 |
| runs where no call of jevify ever named the gold | 1/16, `D5-required-r1` |

No run in the `required` arm was handed the right answer by jevify and then threw
it away. When jevify named the gold the agent used it; the single wrong-looking row
is a run whose first call named something else and whose later work recovered.

## 5. Every jevify call that abstained or did not name the gold

33 calls in `required`, 16 in `available`. Exit codes: `required` 16 x 0, 11 x 3
(abstain), 1 x 2 (usage), 1 x 6 (input), 4 x 128 (interrupted); `available` 5 x 0,
2 x 3, 6 x 6, 3 x 128. The 6s and 128s are the agent mis-driving the CLI, not the
model abstaining, and they are worth as much as the abstentions: an agent that
cannot get the argument shape right is an agent that will not adopt the tool.

Quoted in full, as `unblind.py` prints them:

```
D1-required-r1 call 2: exit 2, did not name the gold
    argv   pick - stops meaningless figures for commands that finish too quickly to measure
    printed (nothing)
    gold   fix-319   agent answered fix-319
D1-required-r1 call 3: abstained
    argv   pick stops meaningless figures for commands that finish too quickly to measure
    printed (nothing)
    gold   fix-319   agent answered fix-319
D1-required-r1 call 4: abstained
    argv   pick nan inf very fast commands
    printed (nothing)
    gold   fix-319   agent answered fix-319
D8-required-r2 call 2: abstained
    argv   pick --files defines pre-allocated integer buffers the scoring code borrows
    printed (nothing)
    gold   src/util/slab.go   agent answered fzf/src/util/slab.go
D8-required-r2 call 3: exit 6, did not name the gold
    argv   pick --files defines pre-allocated pair of integer buffers I16 and I32 for scoring
    printed (nothing)
    gold   src/util/slab.go   agent answered fzf/src/util/slab.go
E1-available-r1 call 1: exit 6, did not name the gold
    argv   pick --from commit names the borrow in return type of entry points for slowness calculation
    printed (nothing)
    gold   98d262c   agent answered e09707c
E1-available-r1 call 2: exit 6, did not name the gold
    argv   fill -- git -C .../work/hyperfine show @{commit:names the borrow in return type for slowness calculation}
    printed (nothing)
    gold   98d262c   agent answered e09707c
E1-available-r1 call 3: exit 6, did not name the gold
    argv   fill -- git show @{commit:renames borrow in return type for slowness}
    printed (nothing)
    gold   98d262c   agent answered e09707c
E1-required-r1 call 1: exit 0, did not name the gold
    argv   pick --from commit names the borrow in the return type of the two entry points
    printed 2899deb8fdb37f0b5318464e16a7ccdafede101a
    gold   98d262c   agent answered 98d262c
E1-required-r2 call 1: exit 0, did not name the gold
    argv   pick --from commit names the borrow in return types of compute functions
    printed 2899deb8fdb37f0b5318464e16a7ccdafede101a
    gold   98d262c   agent answered 98d262c
E1-required-r2 call 2: exit 0, did not name the gold
    argv   pick --from commit added lifetime annotations to compute_with_check and compute functions
    printed 2899deb8fdb37f0b5318464e16a7ccdafede101a
    gold   98d262c   agent answered 98d262c
E4-available-r2 call 1: exit 6, did not name the gold
    argv   pick --files machine-readable output record shapes, minimal allocation, one-way serialization
    printed (nothing)
    gold   crates/printer/src/jsont.rs   agent answered crates/printer/src/jsont.rs
E4-available-r2 call 2: exit 0, did not name the gold
    argv   pick --files machine-readable output record shapes
    printed ./tests/json.rs
    gold   crates/printer/src/jsont.rs   agent answered crates/printer/src/jsont.rs
E4-required-r1 call 1: exit 0, did not name the gold
    argv   pick --files declares record shapes for machine-readable output, shaped for efficient
           writing with no deserialization
    printed ./crates/printer/src/json.rs
    gold   crates/printer/src/jsont.rs   agent answered crates/printer/src/jsont.rs
E4-required-r1 call 2: abstained
    argv   pick --files specifically declares only Serialize, not Deserialize, for machine-readable
           shapes with borrowed data to minimize allocation
    printed (nothing)
    gold   crates/printer/src/jsont.rs   agent answered crates/printer/src/jsont.rs
D5-required-r1 and D5-available-r1: a first pick --from commit named a commit that
    was not the gold; both runs went on to answer 5e2d32f correctly by other means.
```

Two patterns worth naming. `E1` is the one task where jevify is confidently wrong
rather than abstaining: three calls across two runs all returned
`2899deb8` for a question whose answer is `98d262c`, and in both runs the agent
went on to the right answer anyway. `E4` is the decoy working as designed: jevify
returned `crates/printer/src/json.rs` and `tests/json.rs`, the two neighbours of
`jsont.rs`, on the first call and the gold on a later one.

### The three wrong answers

| run | answered | that commit's subject | gold |
|:---|:---|:---|:---|
| `E1-control-r1` | `9806ed7` | "Added option to manually specify a reference to compare the results to." | `98d262c` |
| `E1-available-r1` | `e09707c` | "Extract computation of relative speed" | `98d262c` |
| `D4-available-r2` | `9b5dd60` | "Refactored ExporterManager" | `038f369` |

All three are near misses in the right neighbourhood, which is what a hard task
should produce. `E1-control-r1` reached its wrong answer after 56 turns and 0.45
USD; `E1-required-r1` reached the right one after 12 turns and 0.054 USD.

## 6. What the evidence supports, and what it does not

**Supported.**

1. Adoption at this model is not zero once the tasks are ones lexical search
   cannot answer: 11 of 16, [0.44, 0.86]. The zero in `STUDY.md` was a fact about
   the task set, not about the tool.
2. `available` is the cheapest arm on every resource measured: 139 turns against
   `control`'s 202, 2.74 M input tokens against 4.93 M, 0.81 USD against 1.20.
   Its median run costs 0.027 USD against `control`'s 0.044.
3. Where jevify named the gold in the `required` arm, the agent used it, 15 of 15
   such runs, and never overrode it.
4. A tool call that goes wrong is usually an argument-shape error, not a wrong
   judgment. Corrected 2026-09-28 after a recount of the call logs of the eight
   scored tasks: of 56 calls, 15 produced no judgment at all, 27 per cent — 7 exited
   6, one exited 2, and 7 were killed at 128. The figure first published here, 7 of
   49 exiting 6 or 2, counted only the two usage exits and took its denominator from
   the arms rather than from the logs; it understated the rate by more than half. The
   remaining 41 calls judged: 26 answered, 15 abstained, and 3 of the answers were
   wrong. A reader should take the 27 per cent, not the original 14.

**Not supported.**

1. **No correctness difference is demonstrated.** 15/16, 14/16 and 16/16, with
   intervals [0.72, 0.99], [0.64, 0.97] and [0.81, 1.00]. Every pair overlaps over
   most of its range. On correctness alone **the three arms are indistinguishable**,
   and 16 runs per arm cannot separate rates this close to 1.
2. The cost gap is not an effect either. `available`'s advantage over `control` is
   carried by a handful of runs — `E1-control-r1` alone is 0.45 of `control`'s 1.20
   USD and 2.4 M of its 4.9 M tokens — and one run of 16 moving the arm total by a
   third is the signature of a sample too small to have a median that means
   anything. `required` costs *more* than `control` in turns and wall time, so
   "jevify makes an agent cheaper" is not what these numbers say; what they say is
   that an agent left to choose spent less on these eight tasks than one with no
   tool, once, at n = 2.
3. Nothing here generalises past `claude-haiku-4-5`, eight tasks and three
   repositories. The adoption rate in particular is a property of a model, a
   prompt and a candidate-set size, and D8 shows it moving with the last of those
   inside a single study.
4. jevify's own accuracy is not measured. `E1` shows it confidently wrong three
   times out of three on one task, which is a fact about one question in one
   repository, not a rate.

The honest summary: **the adoption question now has an answer and the efficacy
question does not.** An agent told once that jevify exists reached for it on most
of these tasks. Whether reaching for it made the agent more correct is a null
result at this sample size, and reported as one.

## 7. Changes to the harness, and what is still broken

Two of the faults listed in `STUDY.md` were in scope.

**The post-run scan no longer claims to detect writes.** It was `find -newer` over
the home tree and it was described as listing "every file written outside the run".
It cannot: an mtime names no writer. It is now `mtime_moved_outside_run`, the
docstring says what it is and is not, and 13 of the 48 cells report something. The
contents make the point better than the argument does: the agent-mail server's
SQLite file, `~/.cargo/.global-cache`, `~/.config/screenplays-backup/sync.log`,
`/Users/tpellet/Projects/jevify/evals/cache-retention/README.md`, and 47 paths
under one run — every one of them the operator's own machine moving while a cell
ran, and every one in a directory the canary shows is denied for writing. The
canary, run while nothing else moves, is the containment evidence.

**`~/.claude` per-run: attempted, measured, and not adopted.** `--per-run-config`
exists and works, and the study was not run with it. The reason is measured rather
than assumed: the CLI consults the login Keychain only for the default
configuration directory, so a per-run `CLAUDE_CONFIG_DIR` answers `Not logged in`
— both when seeded with the four keys from `~/.claude.json` and when given a whole
clone of `~/.claude`. Authenticating a per-run directory therefore means writing
the OAuth token into a file inside the run, and the agent under test is the process
that reads it. That is a worse hole than the shared directory. What was done
instead: every path inside the shared directory that is the operator's rather than
the CLI's — history, projects, todos, file history, downloads, shell snapshots,
debug, settings, commands, skills, plugins, agents, hooks, backups — is now denied
for reading, and the canary checks three of them. The directory stays writable,
which is the fault as it stands.

Also changed: `jevify_log.py` records what the tool printed (clipped to 4,000
characters), because "did jevify's answer become the agent's" cannot be read off a
request counter; `run_study.sh` takes an `ARMS` list; `unblind.py` reports three
arms, per-arm intervals, the adoption rate and the abstention list.

The other faults in `STUDY.md` stand as written, and one new one:

8. **A per-cell budget cap decides some outcomes.** The cap is 1 USD.
   `D9-control-r1` hit it and was recorded as answering nothing, which scores
   wrong. No cell of the eight kept tasks hit it, so no figure above depends on it,
   but a harder set would make correctness partly a function of the cap.
9. **n = 2 was chosen after seeing the cost of n = 1.** The repetition count was
   set by the budget after the first repetition had been run and priced, not
   before. That is a decision made with the data in view, and although it is a
   decision about sample size rather than about which runs to keep, it belongs on
   the record next to the dropping of D9.

### Containment, re-checked for three arms

`canary.py` builds all three arms from the real task set and runs each check
through the profile the agent gets. With real sibling runs present, 2026-09-27:

| canary | `control` | `available` | `required` |
|:---|:---|:---|:---|
| write into `~/Projects/testify` | denied | denied | denied |
| write into `/tmp`, into `~` | denied | denied | denied |
| read `scripts/study/tasks.jsonl` (the gold) | denied | denied | denied |
| read a sibling run's `answer.txt`, `transcript.jsonl` | denied | denied | denied |
| read `~/.claude/history.jsonl`, `settings.json` | denied | denied | denied |
| read `~/.ssh/typesafe-ai-key` | denied | denied | denied |
| exec `~/.cargo/bin/jevify`, `target/*/jevify` | denied, 126 | denied, 126 | denied, 126 |
| exec the study's logged copy | denied, 126 | allowed | allowed |
| `cd ~` then write; `cd` into the repository and read `AGENTS.md` | denied | denied | denied |
| write inside the run's own `work/` | allowed | allowed | allowed |
| write into the shared `~/.claude` | allowed | allowed | allowed |
| the network | allowed | allowed | allowed |
| `jevify pick` through the logged wrapper, and its log line | n/a | allowed, logged | allowed, logged |
| list sibling run *names* (fault 3 in `STUDY.md`) | allowed | allowed | allowed |

## 8. Reproducing this

```
export JEVSTUDY=~/jevify-study-d
sh      scripts/study/setup.sh
python3 scripts/study/baseline.py D1 D2 D3 D4 D5 D6 D7 D8 D9 D10 E1 E2 E3 E4 \
        --json benchmarks/agents/lexical-baseline.json
python3 scripts/study/canary.py
for T in D1 D2 D4 D5 D8 D10 E1 E4; do for A in control available required; do
  for R in 1 2; do python3 scripts/study/run_cell.py --task $T --arm $A --rep $R; done
done; done
python3 scripts/study/blind.py && python3 scripts/study/score.py
python3 scripts/study/unblind.py --tasks D1,D2,D4,D5,D8,D10,E1,E4 --max-rep 2
```
