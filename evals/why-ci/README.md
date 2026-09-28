# Failed CI diagnostic benchmark

Requires Python 3.9+, `gh` authenticated for public Actions logs, and `jevify`.
Run `TYPESAFE_API_KEY_FILE=/path/to/key python3 evals/why-ci/run.py --binary ~/.cargo/bin/jevify`.
The backend is TypeSafe; the installed binary and its default model determine behavior.
Raw logs and private results stay in `~/.cache/jevify-why-ci/`; override with `JEVIFY_WHY_CI_CACHE` or `--cache`.
Each log uses the exact `gh run view ID --log-failed -R OWNER/REPO` stdout, including prefixes.
Hash mismatch, unavailable/expired logs, ambiguous gold, and tool errors are NOT RUN.
Gold labels precede inference; hit@k requires a selected line in the first explanatory diagnostic block.
Baseline window hit checks all returned lines; their first/first-three positions are not relevance rankings.
Tokens are UTF-8 bytes/4, including the complete JSON stdout; latency excludes log fetching.
At most two jevify calls per case and 120 per invocation; results appear in [the report](../../benchmarks/why-ci.md).
