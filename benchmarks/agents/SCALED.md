# Four arms, two models, seventeen tasks: jevify against a thin wrapper of the same model

The question: what does jevify add over the thinnest possible client of the model it
calls? `scripts/thinjev/jev` is that client, 56 lines of python3 stdlib: the agent passes a
question and its own options, one keyless request goes to classifier.dev with the request
shape jevify uses, and every option comes back ranked, `p<TAB>option`. No listers, no
evidence, no second round, no threshold, no abstention.

**On correctness the four arms are indistinguishable.** haiku: `control` 12/13, `thin`
16/19, `available` 14/17, `required` 14/15, every 95 % Wilson interval overlapping the
others over most of its range. sonnet: every arm answered every scored run. **What
separates them is adoption, how the tool is used, and cost.** An agent told once about
jevify reaches for it about twice as often as one told once about `jev` (haiku 11/17
against 6/19; sonnet 5/10 against 2/11). When it does, jevify finds its own candidates
and the evidence to judge them, and `jev` judges only what the agent typed; jevify also
abstains on most of its unprompted calls and spends 87 keyless classifications a call on
average (haiku `available`), where `jev` spends one.

The block stopped at 109 of 408 planned cells: the keyless free allowance for this
machine's network ran out at 07:58 UTC, and every keyless call after it failed (section 2).
The second pass, on the binary after the ergonomics change, did not run for the same
reason. Every figure below is from the first pass and is partial.

Measured 2026-09-28 on a macOS dev machine. jevify 0.13.0 (`~/.cargo/bin/jevify`, sha256
`9e2cfa88e867ce1a…`, the binary `ADOPTION.md` measured), `jev` sha256 `b1be1c79dda5a3cc…`,
both keyless against classifier.dev. Agent models `claude-haiku-4-5` (per-cell budget
1 USD) and `claude-sonnet-4-5` (2 USD), one agent process per cell, study tree
`~/jevify-study-scaled`. **Six cells ran at once** (`run_parallel.sh`, `P=6`): every wall
time includes waiting on five other agents and on the shared backend, so wall times
compare arms within this block and nothing else.

## 1. Four arms

| arm | tool the agent can run | the one paragraph that differs | what it answers |
|:---|:---|:---|:---|
| `control` | none | "The tool jevify is not installed on this machine." | the floor |
| `thin` | `jev` | its path and its usage, 8 lines | what the bare model adds |
| `available` | jevify | its path and `jevify init agents`, 3,765 bytes | does an agent reach for jevify |
| `required` | jevify | the same, plus "you must use jevify for the step that chooses among candidates" | does jevify help when used |

`thin` is `available` with the tool swapped: told once, never required, logged by the same
wrapper (argv, exit, stdout and stderr of every call). The Seatbelt profiles differ only in
which of the two tools may be read and executed; `canary.py` shows `thin` running `jev`
and nothing else, the jevify arms running jevify and not `jev`, and `control` running
neither. The network is open in every arm, so a `control` agent could have written its own
request to classifier.dev; no transcript shows one.

## 2. The keyless allowance ran out, and what is excluded because of it

classifier.dev gives one IP network 0.50 USD of free classification per UTC day. This
machine shares it between every worker on it. At 07:58 UTC it was spent: `jev` answers
`HTTP 429 free_ip_daily_budget`, jevify exits 4 with `HTTP 402 request_spending_limit`,
even for a two-candidate `pick`. The allowance is a dollar budget, and jevify's own
`README.md` figure of 20,000 classifications a day does not describe what a shared machine
gets: this block spent 3,632 classifications (1,097 requests) before the refusals began,
alongside whatever the other workers on the network spent.

Ten finished runs made a tool call that was refused for budget. They measured the outage,
not a tool, and every table below drops them (`unblind.py` does by default and names
them): `D10-available-haiku-r4`, `D2-`, `D5-`, `D8-`, `E1-`, `F1-required-haiku-r{1,3}`,
`F15-required-haiku-r4`, `E2-available-haiku-r1`, `F1-thin-haiku-r2`. Kept, they change
haiku `required` from 14/15 to 21/22 and `available` from 14/17 to 15/19: the agents
recovered from the refusal by reading, and correctness does not move. Two cells were
stopped mid-run when the block was halted and have no result (`F12-thin-haiku-r3`,
`F13-required-sonnet-r1`, the latter after 11 refused calls).

What ran is therefore not the design. Of 408 cells, 109 finished and 99 are scored below;
the cells that finished are the ones the fixed shuffle put first, so tasks and arms are
unevenly covered (the per-task tables show which cells exist), and n per task and arm is
0 to 3. The block resumes where it stopped: `run_parallel.sh` skips every cell with a
result.

## 3. The task set

Seventeen tasks, each answered by a commit hash, a path or a branch name at a pinned sha,
and each passing `baseline.py`: no search built from the question's words, and no
identifier a reader might guess, returns the gold among three candidates or fewer.

- Nine from `ADOPTION.md`: D1 D2 D4 D5 D8 D10 E1 E4 and E2, its surplus task. D9 passes
  the gate and stays out for cost (one of its runs spent 1,784 classifications).
- Eight new: F1 F4 over bat `4987f76`, F8 F14 over fd `ce97e47`, F11 F15 over bat, F12 over
  hyperfine, F13 over fzf. Commit tasks are written from the diff of a commit whose subject
  names nothing ("fix format", "Formatting", "improve patch", "Clean up comments and
  tests"); F8 is a ref named `pull/620/head`; F11 and F15 are files named `vscreen.rs` and
  `controller.rs`.
- Seven new candidates were written and disqualified by the gate, each by a guessed
  identifier: F2 (`-S 'Theme: '`, 2 candidates), F3 (`-S shell_quote`, 2), F5
  (`-S Sanitized`, 3), F6 (`-S 'b"."'`, 1), F7 (`-S io_error`, 2), F9 (`git grep -l
  is_socket`, 2), F10 (`git grep -il to_ansi`, 2). A question that paraphrases a diff
  still points at an identifier more often than not.

The full probe record is `$JEVSTUDY/lexical-baseline.json`, written by
`baseline.py ... --json`.

## 4. Results per model and arm

Sums over the arm, medians in brackets. `adopt` is runs that called the arm's tool at
least once. `named` is runs where a call's answer named the gold, then how many of those
the agent answered right and wrong. The answer is `jev`'s first line, or the handles in
jevify's envelope (a `fill` marker, a `pick` match); output holding more than three
candidates -- the log a `fill` ran, the 522 records a `filter` kept -- is a listing and
names nothing, even when the gold is somewhere in it. `shape` is calls that
failed on the form of the request (exit 2 usage, exit 6 input, or a child command's own
exit after `fill` ran it); `abst` is exit 3, the tool saying nothing fits.

### haiku

| arm | k/n | 95 % Wilson | adopt | 95 % Wilson | calls | shape | abst | named / right / wrong | turns | input tokens | wall s | agent USD |
|:---|:---:|:---|:---:|:---|---:|---:|---:|:---:|---:|---:|---:|---:|
| `control` | 12/13 | [0.67, 0.99] | – | – | 0 | – | – | – | 181 [7] | 5,659,004 [113,416] | 563 [26] | 1.27 [0.062] |
| `thin` | 16/19 | [0.62, 0.95] | 6/19 | [0.15, 0.54] | 9 | 0 | 0 | 2 / 2 / 0 | 218 [8] | 3,754,355 [106,355] | 709 [31] | 1.13 [0.052] |
| `available` | 14/17 | [0.59, 0.94] | 11/17 | [0.41, 0.83] | 18 | 3 | 11 | 1 / 1 / 0 | 275 [7] | 7,158,372 [113,302] | 874 [31] | 1.65 [0.042] |
| `required` | 14/15 | [0.70, 0.99] | 15/15 | [0.80, 1.00] | 44 | 5 | 18 | 12 / 12 / 0 | 235 [12] | 5,400,905 [180,292] | 978 [50] | 1.36 [0.070] |

Exit mix: `thin` 9 × 0. `available` 4 × 0, 11 × 3, 2 × 6, 1 child. `required` 21 × 0,
18 × 3, 2 × 2, 1 × 6, 2 child. Keyless use: `available` 206 requests / 1,565
classifications, `required` 564 / 1,059, `thin` one classification per call, 9.

### sonnet

| arm | k/n | 95 % Wilson | adopt | 95 % Wilson | calls | shape | abst | named / right / wrong | turns | input tokens | wall s | agent USD |
|:---|:---:|:---|:---:|:---|---:|---:|---:|:---:|---:|---:|---:|---:|
| `control` | 7/7 | [0.65, 1.00] | – | – | 0 | – | – | – | 82 [6] | 1,474,069 [46,998] | 418 [28] | 0.98 [0.048] |
| `thin` | 11/11 | [0.74, 1.00] | 2/11 | [0.05, 0.48] | 2 | 0 | 0 | 1 / 1 / 0 | 79 [4] | 1,416,149 [39,471] | 312 [12] | 0.93 [0.027] |
| `available` | 10/10 | [0.72, 1.00] | 5/10 | [0.24, 0.76] | 7 | 0 | 3 | 2 / 2 / 0 | 104 [6] | 3,146,844 [84,196] | 502 [19] | 1.62 [0.075] |
| `required` | 7/7 | [0.65, 1.00] | 7/7 | [0.65, 1.00] | 35 | 1 | 17 | 7 / 7 / 0 | 94 [10] | 2,200,580 [175,394] | 409 [42] | 1.14 [0.101] |

Exit mix: `thin` 2 × 0. `available` 4 × 0, 3 × 3. `required` 17 × 0, 17 × 3, 1 × 2.
Keyless use: `available` 71 requests / 140 classifications, `required` 101 / 230.

### The wrong answers

| run | answered | gold | tool calls |
|:---|:---|:---|:---|
| `E1-thin-haiku-r1` | wrong commit | `98d262c` | none |
| `F8-thin-haiku-r2` | `update-crossbeam` | `pull/620/head` | `jev` over 6 branch names, gold not among them |
| `F8-thin-haiku-r4` | `master` | `pull/620/head` | `jev` 4 × over 9 branch names; top line `master` each time |
| `E2-available-haiku-r3` | wrong commit | `ebcf794` | 2 abstentions, then a `filter` over 1,154 records (1,202 classifications) |
| `E2-available-haiku-r4` | wrong commit | `ebcf794` | exit 6, then an abstention; 80 turns, 0.48 USD |
| `F12-available-haiku-r4` | wrong commit | `de07c77` | abstain, then two answers that were not the gold |
| `F12-required-haiku-r3` | wrong commit | `de07c77` | abstain, then three answers that were not the gold |
| `F12-control-haiku-r3` | wrong commit | `de07c77` | none |

F12 ("breaks one chained call across two lines") is the hardest task in the set: the
only wrong answers on it are haiku's, in three arms. E2 is the only task where jevify's
arm did worse than the others that ran it.

## 5. jevify against the thin wrapper

**Where jevify does what `jev` cannot.**

1. **It finds the candidates.** An agent using `jev` has to build the option list itself,
   and on F8 both haiku runs built the wrong one: branch *names*, which for this task are
   the one thing that says nothing (`pull/620/head` against `update-crossbeam`,
   `highlight_match`, ...). One run left the gold out entirely; the other included it and
   `jev` ranked `master` first four times. `jevify pick --from branch` lists the refs
   itself and judges each by its tip subject, and `fill -- git switch '@{branch:...}'`
   does the same inside a command; every jevify-arm run of F8 that exists is right (haiku
   `required`, sonnet `available` and `required`, each through one of those two calls),
   against 0/2 for haiku `thin`. The one sonnet `thin` run of F8 is right too, without
   calling `jev`.
2. **It reads more than the name.** `pick --from commit` walks the whole history (1,018
   to 4,031 candidates in 11 to 41 windows here) and `pick --files` reads excerpts; `jev`
   sees exactly the strings the agent typed, capped at 100 options of 200 characters. In
   `required`, jevify's answer named the gold in 12 of 15 haiku runs and 7 of 7 sonnet
   runs, and the agent answered it every time. Unprompted it did far less: in `available`,
   1 of 11 adopting haiku runs and 2 of 5 sonnet runs, against `jev`'s 2 of 6 and 1 of 2
   in `thin`. Told once, haiku asks jevify questions it abstains on; told to use it for
   the choosing step, haiku asks questions it answers.
3. **It is picked up.** With the same one-paragraph mention, haiku reaches for jevify in
   11 of 17 runs and for `jev` in 6 of 19; sonnet in 5 of 10 and 2 of 11. The intervals
   touch ([0.41, 0.83] against [0.15, 0.54]), so the direction is clear and its size is
   not.

**Where it does not.**

1. **Correctness.** None of the above moves the right-answer rate: 14/17 for `available`
   against 16/19 for `thin` on haiku, 10/10 against 11/11 on sonnet. An agent that does
   not use a tool, or uses it and then reads, reaches the same answers on these tasks.
2. **It abstains more than it answers.** Exit 3 is 11 of 18 calls in haiku `available`, 18
   of 44 in haiku `required`, 17 of 35 in sonnet `required`. The threshold does what it
   is for -- the agent gets "nothing fits" rather than a guess, and no run in any arm was
   handed the gold by its tool and then answered otherwise -- but most calls end with the
   agent reading anyway. `jev` never abstains: its first line is always an answer, which
   on F8 was always wrong.
3. **It costs more, in the tool and in the agent.** Keyless: haiku `available` spent 1,565
   classifications over 18 calls, `thin` 9 over 9. One `filter` over 1,154 records is 1,202
   of them. Agent side, the sums run `thin` < `control` < `available` on both models
   (haiku 1.13 / 1.27 / 1.65 USD, sonnet 0.93 / 0.98 / 1.62), carried by tails:
   `E2-available-haiku-r4` alone is 80 turns and 0.48 USD. The medians say the other
   thing for haiku (`available` 0.042 against `thin` 0.052), which is what a sample this
   small and this skewed does; neither is an effect.
4. **Shape failures.** Three `available` and five `required` haiku calls failed on the
   form of the request (one of the five was the harness's denied here-document, section
   7): exit 2 (a flag), exit 6 (an input), and three child exits, where
   `fill` resolved its marker and ran `git show <branch>` against a ref that exists only as
   `origin/<branch>` (git's own exit 128). `jev` has one form and had no shape failures.

**The answer to the owner's question, on this evidence.** jevify adds candidate discovery
and evidence: it knows how to list branches, commits and files and what to read about
each, and the one task where the agent's own list was the wrong list (F8) is the one task
where that shows. It also adds a refusal to guess, which the agents mostly respond to by
reading. On seventeen tasks that a literal search cannot answer, none of it changes how
often the agent is right; it changes how often the agent reaches for a tool, what the tool
is given to judge, and how much the keyless backend is asked to do.

## 6. First pass against second pass

Not measured. The second pass (arms `available` and `required`, haiku, n = 4, on the
binary after the ergonomics change) needs the keyless allowance, and the allowance was
spent before the first pass was a quarter done.

## 7. Deviations from the design

1. **109 of 408 cells**, 99 scored, uneven by task and arm (section 2). The design was 17
   tasks × 4 arms × (haiku n = 4 + sonnet n = 2).
2. **The block was stopped on budget.** The brief caps keyless use at half a day's
   allowance; after 102 cells the projection was 69 % of the 20,000-classification figure,
   the runner was paused, and the allowance turned out to be already spent at 07:58 UTC.
   The price projection (`price.py`, from `ADOPTION.md`'s cells) put the whole first pass at
   39 USD and 4,500 classifications; the measured rate was 10.46 USD and 3,429
   classifications per 102 cells, and its classification tail is what `ADOPTION.md`
   already saw in D9.
3. **Ten runs dropped for the outage**, listed in section 2, with the figures they would
   change.
4. **Concurrency six.** Wall times include contention; `control`'s wall is not an
   agent's wall on an idle machine.
5. **E2 enters the set** as `ADOPTION.md`'s surplus task; D9 stays out for cost.
6. **`run_parallel.sh` shuffles cells with a fixed seed**, so the finished cells are the
   first 109 of one order, not a stratified sample.
7. **Two sandbox holes affected every arm of pass 1.** The profile denied writes to
   `/tmp`, and the CLI's Bash tool writes `/tmp/claude-XXXX-cwd` after every command, so
   every Bash call reported exit 1 with `Operation not permitted` appended, whatever the
   command did: 1,085 of 1,198 Bash results, in 108 of the 109 runs, carry it (1,079 are
   marked as errors). The command's own output was intact, and the fault hit all four arms
   alike, so it inflates turns and tokens without favouring an arm; it is also why wall
   and cost figures here run higher than `ADOPTION.md`'s would suggest. zsh also writes
   every here-document to `/tmp/zsh*`; that was denied, and two calls piped a
   here-document into jevify: `F15-required-haiku-r2` got exit 6 on empty input (one of
   haiku `required`'s five shape failures, which is therefore the harness's, not the
   agent's), and `F11-required-haiku-r1` abstained on a question built around the failed
   here-document. Both runs answered correctly. `run_cell.py` now allows exactly those two
   paths (`^/private/tmp/claude-[0-9a-f]+-cwd$`, `^/private/tmp/zsh[A-Za-z0-9]*$`), as
   `scripts/ergonomics` does, and `canary.py` checks both in every arm; every other write
   to `/tmp` stays denied.
8. The thin arm's paragraph is 8 lines against jevify's 3,765-byte block. That is the
   difference between the two tools' own documentation, and it is part of what the
   adoption comparison measures.

## 8. Reproducing it

```
export JEVSTUDY=~/jevify-study-scaled
sh      scripts/study/setup.sh ~/.cargo/bin/jevify
python3 scripts/study/baseline.py D1 D2 D4 D5 D8 D10 E1 E2 E4 F1 F4 F8 F11 F12 F13 F14 F15 \
        --json $JEVSTUDY/lexical-baseline.json
python3 scripts/study/canary.py --task D1
python3 scripts/study/price.py --from ~/jevify-study-d --exclude D9 --tasks 17 \
        --block haiku:4:control,thin,available,required --block sonnet:2:control,thin,available,required
P=6 MODELS="haiku:4:1 sonnet:2:2" sh scripts/study/run_parallel.sh \
        "D1 D2 D4 D5 D8 D10 E1 E2 E4 F1 F4 F8 F11 F12 F13 F14 F15"
python3 scripts/study/blind.py && python3 scripts/study/score.py
python3 scripts/study/unblind.py --max-rep 4 --json $JEVSTUDY/report/p1.json
```

The same `run_parallel.sh` line resumes the block after 00:00 UTC; it skips finished
cells. The second pass is the same line with `JEVSTUDY` pointing at a tree staged with the
new binary, `ARMS="available required"` and `MODELS="haiku:4:1"`.
