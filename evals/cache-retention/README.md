# What the answer cache retains

The open question from the harness trial: does a cached *answer* retain raw record text, or only a
digest of the redacted request? If it retained record text, the secret-retention surface would not
be "`why` and `filter`, suppressible with `--no-save`" but "every verb that caches, suppressible
only with `--no-cache`".

Answer: **the cache value holds no record text.** `probe.log` is one run of `probe.sh` on
2026-09-27 against classifier.dev, with a distinctive marker planted in every input and a private
`JEVIFY_CACHE_DIR` per verb. The marker appears in exactly two files, both saved inputs
(`outputs/<blake3-16>.log`), and in none of the 30 answer files.

## The code

`src/jev/cache.rs` writes `serde_json::to_vec(&Response)` to
`answers/<first 2 hex>/<blake3 of the canonical request>.json`. `Response`
(`src/jev/mod.rs`) is a closed struct: `model`, `usage`, and `answers: BTreeMap<String, Answer>`
with `Answer { noul, choice, probabilities, confidence }`. There is no `flatten`, no
`serde_json::Value` and no echo of the request, so nothing of the state or the question text can
reach the file. The request text is hashed into the file *name* only, over a canonical body that
also names the decision contract, the endpoint, the backend and the model
(`src/jev/client.rs::ask_inner`, `ask_batch`), which is what keeps an entry from crossing any of
them.

The only caller-supplied strings that can appear in a value are option names, because a Choice
answer echoes the option it picked:

- `pick`, `why`, `fill`, `route` (tournaments): `L000`…, `NONE` — opaque IDs.
- `sort`: `D000`…, `NONE` — opaque IDs.
- `filter`: its three fixed phrases (`the record says the statement holds`, …).
- `label`: **the labels the caller passed on the command line**, e.g. `bug`, `docs`, `chore`. Argv,
  never record text.
- batched verbs also store `model:<i>` entries whose `choice` is the answering model's name.

## Per verb, from the run

| Verb | Answer files | Saved input | Other files | Marker survives |
|:---|:---|:---|:---|:---|
| `why` | 2 (`any`, `pick` with `L000`…) | yes | — | only in the saved input |
| `filter` | 1 (`<i>:filter`, `model:<i>`) | yes | — | only in the saved input |
| `label` | 1 (`<i>:label`, `model:<i>`; options are the caller's labels) | no | — | no |
| `pick` | 1 (`any`, `pick`) | no | — | no |
| `is` | 1 (`is`) | no | — | no |
| `fill` | 1 (`any`, `pick`) | no | — | no |
| `sort` | 1 (`a<k>`, `f<k>` with `D00…`) | no | journal only with `--apply` | no |
| `add` | 1 (`h<k>`) | no | — | no |
| `route` | 21 (20 tournament windows + one `fit<k>` finals) | no | `inventory-<fingerprint>.json` | no |
| `capabilities`, `health` | none — these verbs do not cache | no | — | no |

`robot-docs` and `init` ask nothing and write nothing either.

## Which switch stops which store

| Store | `--no-cache` / `JEVIFY_NO_CACHE=1` | `--no-save` / `JEVIFY_NO_SAVE=1` |
|:---|:---|:---|
| Answer cache | stops it | no effect |
| Saved inputs (`why`, `filter`) | no effect | stops it |
| Tool inventory, from `route` | stops it | no effect |
| Tool inventory, from the `tool` kind of `fill` and `pick --from` | **no effect** | no effect |
| `sort --apply` journal | **relocates** it to `TMPDIR` | no effect |

Two rows are surprises.

With `--no-cache` the configured cache directory is `None`, and `src/cmd/sort.rs` falls back to
`std::env::temp_dir()`, so the journal's absolute path bytes and inode numbers are written outside
the cache directory instead of not at all. The run shows the journal under the redirected `TMPDIR`
and zero files under the cache directory.

The `tool` kind does not go through `Config`: `source::Env::from_process` reads `JEVIFY_CACHE_DIR`
out of the process environment itself (`src/source.rs`, `Env::from_process`) and never sees `--no-cache` or
`JEVIFY_NO_CACHE`, so `pick --no-cache --from tool` still writes `inventory-<fingerprint>.json`.
It also does not fall back to the platform cache directory, so with `JEVIFY_CACHE_DIR` unset that
path writes nothing and rebuilds the inventory each time. `route`, which loads the inventory
through `Config::cache_dir`, honours the switch: the run shows zero files for
`route --no-cache`.

## Retention

- Answer cache: `DiskCache::get` ignores an entry older than seven days, but nothing ever deletes
  one. `save.rs::prune` only touches `outputs/` and only names it wrote itself. Files accumulate.
- Saved inputs: seven days, pruned on each save (hunch-abw8).
- Tool inventory: no expiry at all. A new PATH fingerprint writes a new file next to the old one.
- `sort` journals: no expiry; they exist to be read by `--undo`.

## What the documents got wrong, before this bead

1. PRIVACY.md was headed "Two stores" while four live in the cache directory, and named the other
   two only in a trailing paragraph.
2. "Answers and probabilities, keyed by a hash of the redacted request" was true but could not be
   read as a bound: a reader could not tell whether the value also held the request. It now says
   what the value holds and what it cannot hold.
3. No document said which verbs cache. The narrower story it left ("`why` and `filter` are the
   verbs with a store") is wrong: all nine asking verbs cache.
4. The tool inventory was described as living in the cache directory, with no retention and no
   switch. It never expires, and `--no-cache` stops only `route`'s copy of it.
5. `sort`'s recovery journal was described as living in the cache directory. Under `--no-cache` it
   is written to the system temporary directory instead, which no document said.
6. `capabilities` described `saved_inputs` in full and the answer cache only as a one-line
   `JEVIFY_NO_CACHE` gloss, so an agent reading the envelope had to assume the worse case. It now
   carries an `answer_cache` object.

## If someone wants the behaviour changed

Nothing here argues for it: the answer cache is already the narrow store, and the documents were
the gap. The two behaviour bugs this turned up are cheap and are worth their own beads.

- Make `--no-cache` suppress the `tool` kind's inventory file: `source::Env` would have to carry the
  resolved cache directory from `Config` instead of reading `JEVIFY_CACHE_DIR` itself
  (`Env::from_process`), which touches every caller of it. A few lines plus test
  churn; no effect on answer hit rate, and a cold inventory rebuild costs ~2 s once per PATH.
- Decide what `--no-cache` should do to the `sort` journal. Suppressing it outright would remove
  `--undo` for that run, so the honest options are to keep writing it to the cache directory
  regardless of the switch, or to keep the current temp-directory fallback and say so (this bead
  says so). One line either way.
- Bounding the answer cache and the inventory by deletion rather than by being ignored would mean
  a prune for `answers/` shaped like `save::prune`, about the size of that function, and zero cost
  to the hit rate within the seven days.

## Reproducing

    MARK=<a string that cannot collide with anything real> zsh evals/cache-retention/probe.sh

The marker is read from the environment and is never written into the script or the log. The run
costs about 30 keyless classifications plus one 20-window `route` tournament, and touches no
directory outside its own `mktemp -d`.
