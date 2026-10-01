# fill on 20 agent requests

`jevify fill --dry-run --json` on 20 requests an agent makes when it can describe a handle but
not name it: commits, branches, files, a directory, pull requests and supplied CI runs, plus one
request with no right answer. Each gold answer is fixed before the run.

**Measured 2026-10-01, jevify 0.14.1, TypeSafe backend, model `jev-1.13.0`, threshold 0.5,
one run per task.**

| Outcome | Count |
| --- | ---: |
| hit (gold handle, exit 0; or exit 3 where nothing fits) | 17 |
| abstain (exit 3 with a gold present) | 2 |
| wrong, no execution (exit 1 or 3 on a wrong item) | 0 |
| wrong and would execute (wrong handle, exit 0) | **0** |

Latency (`meta.elapsed_ms`): median 1.2 s, slowest 6.3 s (`pr`, which lists through `gh`).
Requests (`meta.requests`): 148 over 20 tasks; 1 for a branch or a supplied list, 2 to 7 for a
file or directory, 6 for a pull request, and 7, 13 or 22 for a commit, growing with the history
(1,018, 2,287 and 4,031 commits).

## Data

- `sharkdp/hyperfine` at `f12f3d9` (hf), `BurntSushi/ripgrep` at `3fce3b5` (rg), `sharkdp/bat`
  at `4987f76` (bat): the pins of `tests/e2e.rs`, full clones, with every remote branch fetched
  on the day of the run.
- `pr` reads the live GitHub pull request list of `sharkdp/bat` (1,000 candidates).
- Supplied candidates: `evals/live/runs.json` (34 recorded `gh run list` records) on stdin with
  `--key databaseId`.

## Tasks

| Id | Repo | Kind | Request | Gold | Exit | Chosen | p | Requests | ms | Outcome |
| --- | --- | --- | --- | --- | ---: | --- | ---: | ---: | ---: | --- |
| c1 | hf | commit | the commit that lets users send benchmarked command output to /dev/null, a pipe, or a file | `277807c` | 0 | `277807c` | 0.99 | 0¹ | 941 | hit |
| c2 | hf | commit | the change that always calculates the relative speed comparison when exporting results | `9859eca` | 0 | `9859eca` | 1.00 | 7 | 1576 | hit |
| c3 | rg | commit | the change that avoids creating a decompression reader when it will not be used | `9164158` | 0 | `9164158` | 1.00 | 13 | 1894 | hit |
| c4 | rg | commit | the fix for glob matching of file names that end in a dot | `4df1298` | 0 | `4df1298` | 1.00 | 13 | 1925 | hit |
| c5 | rg | commit | removed a leftover debug print macro from the command-line code | `e92e2ef` | 0 | `e92e2ef` | 1.00 | 13 | 2192 | hit |
| c6 | bat | commit | the fix for wrapping width of control characters shown in caret notation | `fc94a0ec` | 0 | `fc94a0ec` | 0.98 | 22 | 2446 | hit |
| c7 | bat | commit | the fix for a crash when the built-in pager thread panics during drop | `fd67095c` | 0 | `fd67095c` | 0.93 | 22 | 2184 | hit |
| c8 | bat | commit | added detection of Tcl interpreters from the shebang line | `52763e02` | 0 | `52763e02` | 1.00 | 22 | 2070 | hit |
| n1 | hf | commit | the commit that adds GPU kernel benchmarking support | nothing | 3 | — | — | 7 | 1435 | hit |
| b1 | hf | branch | the branch adding the median to exported results | `origin/add-median-to-export` | 0 | same | 1.00 | 1 | 335 | hit |
| b2 | rg | branch | the branch fixing the 2021 security vulnerability | `origin/ag/fix-cve-2021-3013` | 0 | same | 1.00 | 1 | 275 | hit |
| b3 | bat | branch | the branch that wraps by character only when output is a terminal | `origin/character_wrap_only_when_interactive` | 0 | same | 1.00 | 1 | 287 | hit |
| f1 | hf | file | the source file that writes benchmark results as JSON | `src/export/json.rs` | 0 | same | 0.94 | 2 | 376 | hit |
| f2 | bat | file | the module defining how invisible characters are displayed | `src/nonprintable_notation.rs` | 0 | same | 0.93 | 7 | 471 | hit |
| d1 | rg | dir | the crate that reads .gitignore rules while walking directories | `crates/ignore` | 3 | — | — | 2 | 417 | abstain (`ambiguous`) |
| p1 | bat | pr | the pull request that makes triple -p turn off syntax coloring | `4020` | 0 | `4020` | 0.98 | 6 | 6327 | hit |
| p2 | bat | pr | the pull request repairing fish shell completion when it is evaluated in a subshell | `4025` | 3 | — | — | 6 | 5013 | abstain (`ambiguous`) |
| s1 | runs | `-` | the run that was still in progress when the list was taken | `35770201715` | 0 | same | 1.00 | 1 | 235 | hit |
| s2 | runs | `-` | the failed run on main right before release 0.8.0 | `35750107191` | 0 | same | 1.00 | 1 | 241 | hit |
| s3 | runs | `-` | the cancelled run for closing release 0.8.1 | `35758343136` | 0 | same | 1.00 | 1 | 323 | hit |

¹ c1 answered from the response cache: the same request ran once before the measured pass.
Uncached, it spends 7 requests in 1.4 s, as c2 does on the same repository.

Each command has the shape below; the wrapped command (`git show --stat`, `git revert`,
`git cherry-pick`, `git log -1`, `cat`, `ls`, `gh pr checkout`, `gh pr diff`, `gh run view`)
varies per task.

```sh
TYPESAFE_API_KEY_FILE=/path/to/key jevify fill --dry-run --json -- \
    git show --stat "@{commit:the fix for glob matching of file names that end in a dot}"
```

## Reading

- No request resolves to a wrong handle, so none would run a command on the wrong thing. Both
  misses are abstentions with `reason: ambiguous`: the agent gets exit 3 and runs nothing.
- d1: the directory listing holds both `crates/ignore` and its subdirectories, which match the
  description almost as well.
- p2: `sharkdp/bat` holds two pull requests for the same fix, #4025 "fix: fish completion breaks
  under subshell evaluation" and #4006 "Fix fish completions breaking under subshell-based
  evaluation". The request fits both, so abstaining is the right answer and the gold is flawed.
- The sample is small. 0 wrong of 20 bounds the wrong-and-execute rate only loosely (the 95%
  upper bound for 0 of 20 is about 14%).
