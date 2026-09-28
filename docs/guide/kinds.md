# Kinds

A kind names what a marker selects from: `'@{branch:the auth refactor}'` selects a branch,
`'@{commit:made folder moves atomic}'` a commit, `'@{pod:the payment worker}'` a pod. Each
kind lists its own candidates and attaches the evidence that tells them apart. `fill` puts the
handle in the argument and runs the command; `pick --from KIND` prints the handle alone.
`jevify capabilities --json` lists every kind with the command that lists it, under `kinds`.

```sh
jevify fill --dry-run -- git switch '@{branch:the auth refactor}'
jevify fill --dry-run -- git revert '@{commit:made folder moves atomic}'
jevify fill --dry-run -- cat 'src/@{file:parses the marker}'
jevify pick --from commit 'made folder moves atomic'
```

```text
jevify fill -- kubectl logs '@{pod:the payment worker}'
jevify fill -- gh pr view '@{pr:the Windows path fix}'
jevify fill -- gh run view --log-failed '@{ci-run:the failed run for the commit that bumped the checkout action}'
```

## The table

| Kind | Candidates | Evidence |
|:---|:---|:---|
| `-` | records on stdin, or `--candidates FILE` | the whole record; `--key` or `--field` names the handle inside it |
| `branch` | local and remote refs | name, last commit subject, age; a remote ref folds into its local twin, and a branch that exists only on one remote is its short name (`ticket/TPE-791`), with the ref in its evidence. A literal prefix (`'origin/@{branch:…}'`) scopes the listing to that remote's refs; which form to write is under [Coded kinds and recipe kinds](#coded-kinds-and-recipe-kinds) |
| `commit` | the log of the current branch | subject; finalists add body, changed paths and the diffstat with the start of the patch, 1,000 characters of it |
| `file`, `dir` | tracked and untracked files that are not ignored, hidden ones included | path; `file` finalists add first lines, `dir` finalists the names of their first children |
| `tool` | commands on PATH, or `JEVIFY_INVENTORY_FILE` | name and one-line manual summary; finalist man-page evidence |
| `pr`, `issue`, `ci-run`, `stash`, `process`, `container`, `pod` | a recipe: the owning tool's listing | the whole line of the listing |
| `one`, `flag` | options written in the marker | stdin, or `--context FILE` |

`file` and `dir` always run their finals, including with `pick --from`: excerpts precede selection, which
is where a phrase that describes a file's content and not its name is decided. A names-only
answer to such a phrase can be confident and wrong: on the held-out content phrases of
`evals/fill/finals/` a decisive names round with the rest of the field out of play still
chose a file named for the concept and holding something else, once per twelve to fifteen
such rounds on each backend (`benchmarks/results.md`), and `fill` is the verb whose choice
reaches a command. `commit` always runs its finals, including with `pick --from`, one step further
along: a subject line is a claim about a change, and the commit that makes the claim need not
be the commit that holds the change. On the 20 lying-subject cases of `evals/commit-subjects/`,
where a documentation commit announces a change a dull-sounding commit actually made, the
subject alone answers wrong on every case at 0.82 to 1.00 on both backends; the finals, which
read the patch, answer right on every case of the scratch half (`benchmarks/results.md`). For
the same reason a `commit` finalist reaches the finals even when the subjects round scored it
0.00, as far as the 24 finalists hold. `branch` decides on names alone when the names round is
decisive and its winner holds at least twice the probability of the rest of the field, the
other names and NONE together; when the field stays in play, its finals read the evidence.
No held-out measurement of that rule exists for `branch`. `--dry-run` shows the choice before
anything runs.

`branch` and `commit` are ordered: the lister prints newest first. `pr`, `issue`, `ci-run` and
`stash` are ordered by their recipe. The others are not. That order belongs to the lister, not
to the marker: a description is judged on what a candidate is, never on where it sits in the
list, so "the last failed run" is a question for code (`gh run list --status failure --limit 1`)
and "the failed run for the checkout bump" is a question for the marker.

## Coded kinds and recipe kinds

Two kinds of kind exist. A coded kind needs logic: `branch` folds a remote ref into its local
twin, names a remote-only branch by its short name unless two remotes track it, skips
symbolic refs, and takes a literal prefix as its scope. The marker has two forms, and the
command decides between them: the bare marker for a command that takes a branch name (`git
switch '@{branch:the allergy model}'` runs `git switch ticket/TPE-791`, a short name that
`git switch` and `git checkout` resolve to the remote-only branch), the prefixed marker for a
command that takes a revision (`git log -1 'origin/@{branch:the allergy model}'` runs `git log
-1 origin/ticket/TPE-791`, since git resolves a remote-only branch as a revision only under
its remote). The prefix matches remote refs only: a local branch named `origin/x` is listed by
the bare marker, not under `origin/`, and a prefix under which no remote ref lives fails (exit
6) and names the remotes that exist; `commit` runs `git log` and `git rev-list --count` at the same
time so the total is exact; `file` and `dir` walk the tree, honour a literal prefix and withhold
the excerpts and listings of secret or hidden paths; `tool` reads the PATH and the man index
once and caches the inventory under `JEVIFY_CACHE_DIR`. `-` is the coded form of every list a
pipe can supply.

A recipe kind is the `-` form with a name: the command that lists, and which field is the
handle. The lister's own flags choose what each line says, so a recipe has no evidence option.
jevify's own recipes are data, one JSON object per line in `src/kinds.jsonl`:

```text
{"kind":"pr","list":["gh","pr","list","--state","all","--limit","1000","--json","number,title,state,headRefName"],"key":"number","ordered":true}
{"kind":"issue","list":["gh","issue","list","--state","all","--limit","1000","--json","number,title,state"],"key":"number","ordered":true}
{"kind":"ci-run","list":["gh","run","list","--limit","1000","--json","databaseId,displayTitle,status,conclusion,headBranch"],"key":"databaseId","ordered":true}
{"kind":"stash","list":["git","stash","list","--format=%gd%x09%gs"],"field":1,"ordered":true}
{"kind":"process","list":["ps","-axo","pid=,comm=,args="],"field":1}
{"kind":"container","list":["docker","ps","--format","{{.Names}}\t{{.Image}}\t{{.Status}}"],"field":1}
{"kind":"pod","list":["kubectl","get","pods","--no-headers"],"field":1}
```

## A recipe in one line

| Field | Meaning | Default |
|:---|:---|:---|
| `kind` | the name in the marker: `[a-z][a-z-]*` | required |
| `list` | the lister argv, one string per argument, no shell | required |
| `field` | the handle is the N-th whitespace field of a line, counted from 1 | the whole line |
| `key` | the handle is this key of a JSON value; the output is JSON lines or one array | the whole line |
| `ordered` | the lister prints newest first, so a list above the limit keeps its head | `false` |

`field` and `key` exclude each other. An empty `list`, `field` 0, a `kind` that does not match
the pattern, a kind defined twice, or an unknown field is `recipe_invalid`, exit 6. A new kind
is one appended line, and a recipe is shared by copying it.

## Where kinds.jsonl lives

The user's recipes live in `kinds.jsonl` under explicit `JEVIFY_CONFIG_DIR`. No platform
configuration directory is read. An unset variable or missing file holds no user recipes.

```sh
export JEVIFY_CONFIG_DIR="$HOME/.config/jevify"
mkdir -p "$JEVIFY_CONFIG_DIR"
printf '%s\n' '{"kind":"vm","list":["multipass","list","--format","csv"],"field":1}' \
  >> "$JEVIFY_CONFIG_DIR/kinds.jsonl"
```

- jevify reads no recipe from a repository or the working directory. A clone never adds a
  command that `fill` runs. A user recipe is the user's own command, as an alias is.
- A user recipe cannot replace a coded kind or a shipped recipe. A line that names one is
  `recipe_invalid`.
- The user's file is read only for a kind that is neither coded nor shipped. Then every line
  must parse: any bad line is `recipe_invalid` with its line number, whichever kind was asked
  for. A line of invalid JSON names no kind, so the file is valid as a whole or not at all.
- `capabilities` lists user recipes with origin `user`. A bad file is `kinds_error` there and
  never fails `capabilities`.

## Rules for every kind

- One deadline: 20 s. Every lister runs with stdin at `/dev/null`, `GH_PROMPT_DISABLED=1`,
  `GIT_TERMINAL_PROMPT=0` and `NO_COLOR=1`. A tool that is not on the PATH, not logged in or
  rate-limited is `lister_failed`, exit 6, with the tool's own text, never an empty listing. Run
  the named lister yourself.
- A listing counts only after the lister exited 0 and both of its streams reached end of file.
  A partial listing never reaches the model or the command.
- A list above the limit (3,267 candidates per marker on classifier.dev and 13,200 on TypeSafe,
  both served in full, measured 2026-09-24;
  9,801 and 20,000 for `pick --from`): an ordered kind keeps its newest part and the status
  line says `candidates N of M, newest first`; any other kind is `too_many`, exit 6, with the
  two ways to narrow it: a literal prefix, or a piped list.
- A literal prefix narrows a path kind: `'src/cmd/@{file:stages hunks}'` looks only under
  `src/cmd/`, and `'--config=conf/@{file:the staging profile}'` under `conf/`. The prefix is
  the text before the marker when it ends with `/`. A prefix that names no directory is
  `lister_failed`.
- Candidates come from the directory jevify runs in. `-C DIR` changes it, and a wrapped
  `git -C DIR` also scopes the listers.
- Hidden files are listed; `.git/` never is. An excerpt is never read for a path with a
  component that matches `.*`, `id_*`, `*.pem`, `*.key`, `*credentials*` or `*secret*`, nor
  for a symlink. The status line says `excerpts withheld: N`. The name still reaches the
  backend; the content does not.
- A list with no kind is a pipe into `'@{-:…}'`, shaped by `sed`, `cut` or `jq` first:
  `cargo test -- --list | sed 's/: test$//' | jevify fill -- cargo test '@{-:the retry backoff cap}'`.

## The status line

`fill` reports each marker on stderr with the `jevify fill:` prefix: the kind, the handle, its
probability with the next and none probabilities, the evidence, then the count part, then the
model. `pick --from` prints the count part before the first request:

```text
jevify pick: candidates 150, windows 2
jevify pick: candidates 9801 of 9802, newest first; windows 99
jevify fill: not run: arg 3 -: no_match; no record to choose from; candidates 0 of 0, omitted 0; model not requested
```

On a resolved marker the count part reads `candidates N[ of M[, newest first]][, omitted K],
windows W[, excerpts withheld: E]`, as in `candidates 3267 of 3268, newest first, windows 33`:
`of M` when the listing was cut, `newest first` when the cut kept the head of an ordered
listing, `omitted K` when a remote twin or a symbolic ref was folded away, and
`excerpts withheld: E` for finalists whose excerpt the path rules kept out, absent when none
was. The `not run:` line of an abstention reads `candidates N of M, omitted K` instead, with
both parts always present and no window count, as in `candidates 0 of 0, omitted 0`. In machine
output the same numbers
are `data.markers[].candidates`, `total` and `omitted`, and for `pick --from`
`data.candidates`, `total`, `omitted` and `windows`.
