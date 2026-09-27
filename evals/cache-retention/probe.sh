#!/bin/zsh
# What the answer cache retains (hunch-6pv8).
#
# Runs every caching verb once against the keyless backend, each into its own cache directory, so
# every file on disk is attributable to one verb; then prints each answer file whole and reports
# whether a marker planted in the input survives anywhere in the store. Ends with the switch
# matrix: which store each of --no-cache, JEVIFY_NO_SAVE and --no-save actually stops.
#
# Usage: MARK=<a string that cannot collide with anything real> zsh evals/cache-retention/probe.sh
# The marker is read from the environment and never written into this file. Costs ~30 keyless
# classifications plus a 20-window `route` tournament; nothing touches the real cache directory.
set -u
: ${MARK:?set MARK to a distinctive marker string}
J=${JEVIFY:-$PWD/target/release/jevify}
S=$(mktemp -d)
export JEVIFY_BACKEND=classifier JEVIFY_CONCURRENCY=2
print -r -- "scratch: $S"

LOG=$S/log.txt
{
  print -r -- "[1/4] compiling module alpha"
  print -r -- "[2/4] connector $MARK opened"
  print -r -- "error: payment gateway rejected the token: api_key=$MARK"
  print -r -- "[4/4] build failed after 12s"
} > $LOG
RECS=$S/records.txt
{
  print -r -- "fix: retry the $MARK connector on a 503"
  print -r -- "docs: describe the $MARK handshake"
  print -r -- "chore: bump the linter"
} > $RECS
mkdir -p $S/sortdir/notes $S/sortdir/code
print -r -- "meeting notes: the $MARK rollout is next week" > $S/sortdir/a.txt
print -r -- "fn main() { println!(\"$MARK\"); }" > $S/sortdir/b.rs
R=$S/repo
mkdir -p $R
git -C $R init -q
git -C $R config user.email a@b.c
git -C $R config user.name t
print -r -- "line one" > $R/f.txt
git -C $R add f.txt
git -C $R commit -qm init
print -r -- "token expiry fix for the $MARK path" >> $R/f.txt

one() {
  name=$1; shift
  export JEVIFY_CACHE_DIR=$S/c-$name
  mkdir -p $JEVIFY_CACHE_DIR
  print -r -- "##### VERB $name"
  "$@" >$S/o-$name.txt 2>$S/e-$name.txt
  print -r -- "exit=$?"
  find $JEVIFY_CACHE_DIR -type f | sed "s|$JEVIFY_CACHE_DIR/|  |" | sort
  print -r -- "  marker present in store: $(grep -rl "$MARK" $JEVIFY_CACHE_DIR | sed "s|$JEVIFY_CACHE_DIR/||" | tr '\n' ' ')"
  for f in $JEVIFY_CACHE_DIR/answers/*/*.json(N); do
    print -r -- "  --- answers value ${f:t}"
    print -r -- "  $(cat $f)"
  done
}

one why    sh -c "$J why < $LOG"
one filter sh -c "$J filter 'the change is a bug fix' < $RECS"
one label  sh -c "$J label bug,docs,chore < $RECS"
one pick   sh -c "$J pick 'the change about retries' < $RECS"
one is     sh -c "$J is 'the build failed' < $LOG"
one fill   $J fill --dry-run --candidates $RECS -- echo "@{-:the change about retries}"
one sort   $J sort $S/sortdir
one add    sh -c "cd $R && $J add --dry-run 'the token expiry fix'"
one route  $J route "count the lines in notes.txt"
one capabilities $J capabilities --json
one health $J health

print -r -- "##### SWITCH MATRIX"
sw() { print -r -- "--- $1"; export JEVIFY_CACHE_DIR=$S/sw-$2; mkdir -p $JEVIFY_CACHE_DIR; }

sw "why --no-cache" 1
$J why --no-cache < $LOG >/dev/null 2>&1
find $JEVIFY_CACHE_DIR -type f | sed "s|$JEVIFY_CACHE_DIR/|  |" | sort

sw "why JEVIFY_NO_SAVE=1" 2
JEVIFY_NO_SAVE=1 $J why < $LOG >/dev/null 2>&1
find $JEVIFY_CACHE_DIR -type f | sed "s|$JEVIFY_CACHE_DIR/|  |" | sort

sw "filter --no-save" 3
$J filter --no-save 'the change is a bug fix' < $RECS >/dev/null 2>&1
find $JEVIFY_CACHE_DIR -type f | sed "s|$JEVIFY_CACHE_DIR/|  |" | sort

sw "label --no-cache (file count)" 4
$J label --no-cache bug,docs,chore < $RECS >/dev/null 2>&1
find $JEVIFY_CACHE_DIR -type f | wc -l

sw "sort --apply, cache on: where the journal lands" 5
A=$S/sa; mkdir -p $A/notes $A/code
print -r -- "meeting notes: the $MARK rollout is next week" > $A/a.txt
print -r -- "fn main() { println!(\"$MARK\"); }" > $A/b.rs
$J sort --apply $A >/dev/null 2>&1
find $JEVIFY_CACHE_DIR -type f | sed "s|$JEVIFY_CACHE_DIR/|  |" | sort
print -r -- "  journal files holding the marker: $(grep -rl "$MARK" $JEVIFY_CACHE_DIR/sort-undo-*.jsonl(N) 2>/dev/null | wc -l)"

sw "sort --apply --no-cache, TMPDIR redirected: where the journal lands" 6
T=$S/tmp; mkdir -p $T
B=$S/sb; mkdir -p $B/notes $B/code
print -r -- "meeting notes: the $MARK rollout is next week" > $B/a.txt
print -r -- "fn main() { println!(\"$MARK\"); }" > $B/b.rs
TMPDIR=$T $J sort --apply --no-cache $B >/dev/null 2>&1
print -r -- "  under the cache directory: $(find $JEVIFY_CACHE_DIR -type f | wc -l) file(s)"
print -r -- "  under TMPDIR:"; find $T -type f | sed "s|$T/|    |" | sort

sw "route --no-cache: the inventory must not be written" 7
$J route --no-cache "count the lines in notes.txt" >/dev/null 2>&1
print -r -- "  $(find $JEVIFY_CACHE_DIR -type f | wc -l) file(s)"

sw "pick --from tool --no-cache: the inventory is written anyway" 8
$J pick --no-cache --from tool 'counts the lines in a file' >/dev/null 2>&1
find $JEVIFY_CACHE_DIR -type f | sed "s|$JEVIFY_CACHE_DIR/|  |" | sort

print -r -- "##### REPLAY: a second identical run must send nothing"
export JEVIFY_CACHE_DIR=$S/c-is
$J is --json 'the build failed' < $LOG |
  python3 -c "import json,sys;d=json.load(sys.stdin);print('  cache_hits',d['meta']['cache_hits'],'requests',d['meta']['requests'],'exit',d['exit_code'])"
