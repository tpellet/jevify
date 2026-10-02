#!/bin/bash
# Runs local fixture examples from the getting-started guide and this directory, and from
# this repository's own history. Run it from any directory on a typical macOS dev machine
# with jevify on the PATH; the free backend answers without a key, and
# TYPESAFE_API_KEY_FILE=/path/to/key selects TypeSafe.
#
#   bash docs/demo/examples.sh            every example
#   bash docs/demo/examples.sh why        one of: why, fill, label, nothing-fits, is, try
#
# The fixtures: build.log is the output of `cargo build` in benchmarks/fixtures/demo/buildfail
# (one error under 300 warnings); issues.txt is ten issue titles; downloads.txt is the listing
# of a downloads folder; mail.txt is a customer mail.
set -u
cd "$(dirname "$0")/../.." || exit 1
DEMO=docs/demo

show() {
    printf '\n$ %s\n' "$*"
    eval "$*"
    printf '(exit %s)\n' "$?"
}

why() {
    show "jevify why < $DEMO/build.log"
}
fill() {
    show "git log --oneline v0.8.3..v0.9.3 | jevify fill --field 1 --dry-run -- git show --stat --format=%s '@{-:made route abstain when two commands are too close}'"
    show "printf 'retry_backoff\nparse_header\n' | jevify fill --dry-run -- cargo test '@{-:the test that retries a failed request}'"
    show "printf 'A crash with no reproduction steps.\n' | jevify fill --dry-run -- printf '%s\n' '@{one:bug|feature|docs:what kind of report is this}' '@{flag:--draft:the report lacks steps to reproduce}'"
}
label() {
    show "jevify filter --label bug,feature,question < $DEMO/issues.txt"
    show "jevify filter --label bug,feature,question < $DEMO/issues.txt | cut -f1 | sort | uniq -c"
    show "jevify filter 'reports a crash' < $DEMO/issues.txt"
}
nothing_fits() {
    show "jevify pick 'what I paid a streaming service' < $DEMO/downloads.txt"
    show "jevify pick 'the tax return' < $DEMO/downloads.txt"
    show "jevify fill --dry-run -- git show '@{commit:ports the user interface to Android}'"
}
is() {
    show "jevify is 'asks for a refund' < $DEMO/mail.txt && echo refund"
    show "cargo build 2>&1 | jevify is 'the build failed' || echo 'build is fine'"
}
try() {
    show "printf 'build started\nerror: connection timed out\nbuild stopped\n' | jevify filter --strict 'reports a network failure'"
    show "git ls-files | jevify pick --files 'where the command-line flags are defined'"
    show "jevify pick --from tool 'keep my mac awake for an hour'"
}

want=${1:-all}
for name in why fill label nothing-fits is try; do
    if [ "$want" = all ] || [ "$want" = "$name" ]; then
        "${name//-/_}"
    fi
done
