# scripts/thinjev — the thin baseline

`jev` is the least a Jev CLI can be: python3 stdlib, one keyless call to
classifier.dev with the request shape `src/jev/classifier.rs` sends.

    jev "which commit changed how retries back off" a1b2c3 d4e5f6 ...
    git log --format='%h %s' | jev "which commit changed how retries back off"

The question is the one item, the options are the labels (2..100, each at most
200 characters). It prints every option as `p<TAB>option`, best first. It has no
listers, no evidence building, no second round, no threshold and no abstention,
which is what makes it the baseline for measuring what jevify adds on top.
