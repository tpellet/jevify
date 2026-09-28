use crate::cli::Shell;
use crate::cmd::Outcome;
use crate::config::{Backend, Config};
use crate::exit::{Exit, JevifyError};
use crate::source;

const GUIDE: &str = include_str!("../../docs/ROBOT_MODE.md");

/// The path patterns whose excerpts `--files` and the `file` kind withhold, as
/// `records::withheld` judges them: one line per component rule.
pub const WITHHELD_PATTERNS: [&str; 6] =
    [".*", "id_*", "*.pem", "*.key", "*credentials*", "*secret*"];

/// Every kind with its lister argv: the coded kinds, the shipped recipes, the user's recipes
/// from `kinds.jsonl` in the configuration directory, then the two caller-option kinds that
/// list nothing. A bad user file adds `kinds_error` and never fails the caller.
fn kinds() -> (Vec<serde_json::Value>, Option<String>) {
    let env = source::Env::from_process(source::LISTER_TIMEOUT);
    let catalog = source::catalog(&env);
    let mut kinds: Vec<serde_json::Value> = catalog
        .kinds
        .iter()
        .map(|entry| {
            let mut value = serde_json::json!({
                "name": entry.name,
                "origin": entry.origin,
                "family": if entry.name == "-" { "input records" } else { "existing things" },
                "list": entry.list,
            });
            let extra = match entry.name.as_str() {
                "-" => serde_json::json!({ "input": "stdin or --candidates FILE; --field is 1-based whitespace, --key selects a JSON handle" }),
                "branch" => serde_json::json!({
                    "enrich": ["git", "log", "-5", "--format=%x00%s%x00", "--name-only", "-z", "--no-renames", "--no-ext-diff", "--end-of-options", "<handle>", "--"],
                    "evidence": "name, subject, age; local and remote twins collapse; newest first",
                    "forms": "bare '@{branch:x}' substitutes the short name a branch-taking command accepts (git switch, checkout, push); a literal prefix 'origin/@{branch:x}' lists that remote's refs and substitutes the qualified ref a revision-taking command resolves (git log, rev-parse, diff)",
                    "ordered": true,
                }),
                "commit" => serde_json::json!({
                    "evidence": "full OID and subject; finalists add body, changed paths and the diffstat with the first 1,000 characters of the patch, so a subject that claims a change another commit holds loses the finals; -n <limit> after log, the total from git rev-list --count HEAD; newest first",
                    "ordered": true,
                }),
                "file" => serde_json::json!({
                    "evidence": "path; finalists add first lines, withheld for the patterns of withheld; a literal prefix ending in / narrows the walk; outside a work tree a no-follow walk",
                    "ordered": false,
                }),
                "dir" => serde_json::json!({
                    "evidence": "directory path; finalists add the names of their first children, withheld for the patterns of withheld; a literal prefix ending in / narrows the walk",
                    "ordered": false,
                }),
                "tool" => serde_json::json!({
                    "evidence": "name and one-line manual summary from the PATH and the man index, cached under JEVIFY_CACHE_DIR",
                    "ordered": false,
                }),
                _ => serde_json::json!({
                    "evidence": "the whole line of the listing; the handle is field N or key KEY of the recipe",
                    // A shipped recipe is in the registry; a user recipe is known to lookup only.
                    "ordered": source::lookup(&entry.name, &env).ok().flatten().is_some_and(|kind| kind.ordered),
                }),
            };
            if let (Some(target), Some(fields)) = (value.as_object_mut(), extra.as_object()) {
                target.extend(fields.clone());
            }
            value
        })
        .collect();
    kinds.push(serde_json::json!({ "name": "one", "origin": "coded", "family": "caller options", "list": [], "input": "options in '@{one:a|b|c:question}'; context on stdin or --context FILE" }));
    kinds.push(serde_json::json!({ "name": "flag", "origin": "coded", "family": "caller options", "list": [], "input": "whole argument '@{flag:--name:question}'; yes keeps, no removes, unsure abstains" }));
    (kinds, catalog.error)
}

pub fn capabilities() -> Outcome {
    let (kinds, kinds_error) = kinds();
    let exit_codes: Vec<_> = Exit::ALL
        .iter()
        .map(|(e, d)| {
            let meaning = match e {
                Exit::No => "is: at least one no; filter: kept none",
                Exit::Unavailable => {
                    "backend unavailable, deadline passed or quota exhausted: error.kind says which"
                }
                Exit::Interrupted => "declined at add confirmation",
                _ => d,
            };
            serde_json::json!({
                "code": e.code(),
                "name": e,
                "meaning": meaning,
                "kinds": JevifyError::KINDS.iter().filter(|(_, x)| x == e).map(|(k, _)| *k).collect::<Vec<_>>(),
            })
        })
        .collect();
    // The complete enumeration, from the one table in src/exit.rs that `kind()` is tested
    // against: a caller reads its branches off the machine interface instead of the source.
    let error_kinds: Vec<_> = JevifyError::KINDS
        .iter()
        .map(|(kind, exit)| serde_json::json!({ "kind": kind, "exit": exit.code() }))
        .collect();
    let input_error_kinds: Vec<_> = JevifyError::KINDS
        .iter()
        .filter(|(_, exit)| *exit == Exit::Input)
        .map(|(kind, _)| *kind)
        .collect();
    // The keyless quota per verb: one classification is one record under one question, 20,000
    // a day per IP. The quota and the `is` and `filter` costs were measured (benchmarks/
    // results.md); the calls a day are computed from the shape of each verb's requests.
    let keyless_cost = serde_json::json!({
        "is": "1 per statement",
        "filter": "1 per distinct record; jevify sends at most 60 records per request, the largest batch tried (75 is refused with HTTP 402 request_spending_limit)",
        "label": "1 per distinct record; jevify sends at most 60 records per request, the largest batch tried (75 is refused with HTTP 402 request_spending_limit)",
        "pick": "2 per window of 99 lines, plus 2 for the final round when there is more than one window",
        "why": "2 per window of 99 lines, plus 2 for the final round, which always runs",
        "route": "2 per window of 99 commands, plus 1 per finalist, at most 12"
    });
    let keyless_calls = serde_json::json!({
        "is": "6,600 (3 statements) to 20,000 (1)",
        "filter": "20,000 records in total: 333 calls of 60 records",
        "label": "20,000 records in total: 333 calls of 60 records",
        "pick": "830 (1,000 lines) to 10,000 (at most 99)",
        "why": "830 (1,000 lines) to 5,000 (at most 99)",
        "route": "about 380 over a PATH of 1,883 commands"
    });
    // The other store under the cache directory: what a cached answer holds, which is the answer
    // and never the record it judged, and the two files that share the directory with it.
    let answer_cache = serde_json::json!({
        "verbs": ["fill", "pick", "why", "route", "filter", "label", "is", "add", "sort"],
        "directory": "JEVIFY_CACHE_DIR/answers/<2 hex>/<blake3>.json, or the platform cache directory/jevify/answers",
        "key": "blake3 of the redacted request, which names the decision contract, endpoint, backend and model, so an entry never crosses any of them",
        "contents": "the answer alone: the answering model's name, and per question a probability, the chosen option and a probability per option. No record text, no file excerpt, no path and no question text; options are the request's opaque ids (L000, D000, NONE), the labels passed to label, or filter's three fixed phrases",
        "retention": "entries are ignored after 7 days; nothing deletes the files",
        "disable": "--no-cache for one call, JEVIFY_NO_CACHE=1 for every call (both independent of --no-save); it also stops route's tool inventory file, but not the tool kind's",
        "hit": "replays the stored answer and sends nothing: meta.cache_hits counts it, meta.requests stays at 0",
        "other_files_in_the_directory": {
            "inventory": "inventory-<fingerprint>.json: names and man-page one-line summaries of the executables on PATH, written by route and by the tool kind of fill and pick --from; never expired. --no-cache stops route from writing it; the tool kind reads JEVIFY_CACHE_DIR directly and writes it even then, and writes nothing when that variable is unset",
            "sort_journal": "sort-undo-<stamp>-<pid>-<n>.jsonl, written by sort --apply: absolute source and destination path bytes with each file's device and inode, no file content, never sent to a model, never expired, and written to the system temporary directory instead when --no-cache is given"
        }
    });
    // How the argv an agent writes is read, and what a failure tells it back.
    let argument_shapes = serde_json::json!({
        "descriptions": "the intent of pick, route and add and the statement of filter are one quoted argument or several bare words, joined with spaces; is reads each argument as one statement, so quote a sentence, and warns on stderr when three or more statements are single words",
        "stdin": "a leading - before a description is ignored: stdin is already the default source",
        "placement": "a verb's options may come before the verb (jevify --no-save why); global options go anywhere before --",
        "directory": "-C DIR (--repo) on fill and pick runs as if started in DIR",
        "errors": "every error names the caller's own command, corrected when jevify can, in error.example and on the try: line of stderr (fill: on its hint line); an unknown verb names the nearest verb and this version",
        "abstentions": "pick, why, add and fill print the nearest candidates as nearest (not chosen) with a hint; is prints its p and band; they are never the answer"
    });
    let mut data = serde_json::json!({
        "name": "jevify",
        "version": env!("CARGO_PKG_VERSION"),
        "summary": "Answers questions about text that already exists (your input, the installed tools, man pages, folders) by meaning. It selects and never generates. Backend-specific decision scores are not evidence of calibration for every task. Model: Jev through TypeSafe with a key or classifier.dev without one.",
        "use_when": "a question is about meaning and grep or keywords cannot ask it, or the input is too long to read; skip it when a literal search answers the question or you already know the exact command",
        "global_flags": ["--json (alias --robot)", "--format human|json|jsonl|toon", "-t/--threshold <0..1>", "--model <id>", "--no-cache", "--verbose"],
        "output": "human stdout is plain text made for pipes: pick and filter preserve input records; label prints LABEL<TAB>RECORD; why prints numbered context; is prints nothing for one statement and VERDICT<TAB>STATEMENT lines for several. Pass --json for the envelope; non-UTF-8 records have text, lossy: true, ordinal",
        "commands": [
            { "name": "fill", "usage": "jevify fill [--dry-run] [-q] [-C DIR] [--candidates FILE] [--context FILE] [--field N | --key KEY] [-0 | --para] -- COMMAND ARGS...", "stdin": true, "exit": [0, 2, 3, 4, 5, 6], "exit_meaning": "2–6 nothing ran; otherwise the command's own exit code; --dry-run exits 0 on resolution. After exec the command can also exit 2–6, so set JEVIFY_STATUS_FILE and read its ran field to tell the two apart (see exec_status).", "data": "argv under --dry-run on success, markers[{arg,kind,reason,handle,p,candidates,total,omitted}], reason", "when": "about to list branches, commits, files, PRs or CI runs only to choose one: write the description inside the command and let one call resolve and run it", "example": "jevify fill -- git switch '@{branch:the auth refactor}'", "note": "all-or-nothing; machine output requires --dry-run; quote the whole marker argument with single quotes; preview, never eval; stdin has one role and is empty for the command when consumed; every kind of capabilities.kinds is a marker kind; -C DIR runs listers, excerpts and the command as if started in DIR, and the listers of a git command run where its own -C DIR points; a bare branch marker in a git subcommand that reads a revision (log, show, diff, rev-parse, ...) becomes the remote ref (origin/x) of a branch that exists only on a remote, and stderr says so; status line of a resolved marker: candidates N[ of M[, newest first]][, omitted K], windows W[, excerpts withheld: E]; the not run: line of an abstention: nearest (not chosen) candidates, then candidates N of M, omitted K, always both and no windows, then a hint unless -q" },
            { "name": "pick", "usage": "<stdin> | jevify pick <intent...> [-n N] [--index | --files] [-0 | --para] [-C DIR]; jevify pick --from KIND <intent...> [-n N] [-C DIR]", "stdin": true, "exit": [0, 3], "data": "matches[{line?,text,ordinal,p,lossy?}], any, source; --from adds reason, candidates, total, omitted, windows, finalists_per_window; an abstention adds closest[{text,p}] (the nearest, not chosen) and hint", "when": "one record or file out of a listing, by content; or a handle alone, without a run", "example": "jevify pick --from commit 'made folder moves atomic'; git ls-files | jevify pick --files 'where man pages are parsed'", "note": "--from accepts every kind of capabilities.kinds except one and flag; stdin is the default source, and --from - or a leading - before the description names it; --files with nothing on stdin ranks the files under the current directory (git ls-files, or a walk outside a work tree) and says so on stderr; -C DIR runs as if started in DIR; --from conflicts with --files, --index, -0 and --para; hidden or secret-looking paths and symlink files receive no excerpt; selected stdin records retain their bytes and input order; an abstention prints jevify pick: REASON; nearest (not chosen): TEXT (P), ...; hint: ... on stderr, and those candidates are never the answer; input is not saved" },
            { "name": "why", "usage": "<cmd> 2>&1 | jevify why [-C N] [-n N] [--no-save]", "stdin": true, "exit": [0, 3], "data": "causes[{line,text,p,context[]}], any, considered, total, hint, saved_input, complete (the raw input reached the saved-input store, nothing about the lines: selection coverage is considered against total)", "when": "a failed build, test or CI log longer than about 50 lines, or grep found only the symptom: read the cause it prints, not the whole log", "example": "gh run view --log-failed | jevify why --json", "note": "prints numbered lines with context; no split options; past 1,500 distinct lines keeps error neighbourhoods within 4,000 lines: compare considered with total" },
            { "name": "route", "usage": "jevify route <intent...>", "stdin": false, "exit": [0, 3], "data": "tool, summary, synopsis, fit, ties[{tool,fit}], alternatives[]", "when": "which installed tool does a task, before guessing names with which or --help", "example": "jevify route 'keep my mac awake for an hour'", "note": "prints a tool, summary and synopsis; starts no command; when a command above the threshold fits within 0.10 of the best, the two are too close to tell apart: exit 3, tool null, and every tied command in ties" },
            { "name": "filter", "usage": "<stdin> | jevify filter [-v] [-c] [--strict] [-0|--para] [--files] [--no-save] '<statement>'", "stdin": true, "exit": [0, 1, 3], "data": "records[{text,ordinal,p,verdict,lossy?,unreadable?}], kept, total, unsure, complete (this run emitted every record and the raw input reached the saved-input store, nothing about judgment: judgment coverage is unsure against total), saved_input, excerpts_withheld", "when": "many records or files, one question: one process keeps the matching ones instead of reading each", "example": "cargo test 2>&1 | jevify filter 'reports a failed assertion'; fd -0 | jevify filter -0 --files 'tests the retry backoff'", "note": "-v inverts; -c counts; each record is judged three ways (the statement holds, it does not hold, the record does not say) and the kept set is holds plus does-not-say, so unsure records stay, even under -v, unless --strict; 0 kept some, 1 kept none, 3 every record unsure; --files reads stdin paths and withholds hidden or secret-looking excerpts; a file it cannot read is unsure, never judged by name" },
            { "name": "label", "usage": "CMD | jevify label a,b,c [-0|--para] [--files]", "stdin": true, "exit": [0, 2, 3, 4, 6], "data": "records[{label,text,ordinal,p,lossy?,unreadable?}], labelled, total, unsure, complete, excerpts_withheld", "when": "every record or file needs a bucket: one process tags them all instead of reading each one", "example": "gh issue list | jevify label bug,feature,question | cut -f1 | sort | uniq -c; ls reports/*.md | jevify label --files bug,feature,docs", "note": "prints LABEL<TAB>RECORD in input order, the record unchanged after the tab; ? marks an unsure record; 0 labelled, 3 every record unsure; at least two distinct labels, none ? or NONE, at most the backend window (99 on classifier.dev, 200 on TypeSafe) else exit 2; the record limits of filter apply; --files reads stdin paths and withholds hidden or secret-looking excerpts; a file it cannot read is ?, never labelled by name; saves nothing" },
            { "name": "is", "usage": "jevify is '<statement>' ['<statement>' ...] [--context FILE] [--band 0.15]", "stdin": true, "exit": [0, 1, 3], "data": "one statement: p, verdict, truncated, reason (when oversized); several: statements[{statement,verdict,p}], verdict, truncated, reason (when oversized)", "when": "the next step depends on a fact: write the condition so that yes means act", "example": "jevify is 'asks for a refund' 'mentions an order' --context mail.txt", "note": "stdin unless --context supplies a file; 0 all yes, 1 one no, 3 otherwise; oversized evidence abstains before API requests" },
            { "name": "add", "usage": "jevify add [--dry-run|--yes] \"<topic>\"", "stdin": false, "exit": [0, 3, 6, 130], "data": "hunks[{file,header,p,staged}]", "when": "stage part of a working tree without a terminal: git add -p is interactive, add is not", "example": "jevify add --json --dry-run \"the token expiry fix\"", "note": "stages single hunks of tracked files; rejects oversized hunks or batches before requests or staging; index only, never commits; works from any subdirectory" },
            { "name": "sort", "usage": "jevify sort <dir|file> [--into <root>] [--apply | --undo <log>]", "stdin": false, "exit": [0, 3, 6], "data": "moves[{from,to,p}], skipped[{file,reason}], undo_log, applied", "when": "files whose names say nothing need a home among the folders that already exist; it reads an excerpt", "example": "jevify sort --json ~/Downloads --into ~/Documents", "note": "dry-run by default; atomic no-replace apply/undo; unique durable JSONL recovery journal; symlink entries skipped; same volume only; concurrent source replacement unsupported; failures identify recovery log and progress" },
            { "name": "capabilities", "usage": "jevify capabilities --json", "exit": [0], "data": "name, version, commands, global_flags, exit_codes, env, limits, backends, saved_inputs, envelope, telemetry, workflows, safety" },
            { "name": "robot-docs", "usage": "jevify robot-docs [guide|commands|exit-codes|examples|privacy]", "exit": [0, 2], "data": "topic, text" },
            { "name": "health", "usage": "jevify health --json", "exit": [0, 4, 5], "data": "backend, base_url, key, api, latency_ms, models" },
            { "name": "init", "usage": "jevify init zsh|bash|agents", "exit": [0], "data": "script" }
        ],
        "common_exit": { "codes": [2, 4, 5, 6], "meaning": "any command: usage, API unavailable, auth, input" },
        "exit_codes": exit_codes,
        "kinds": kinds,
        "kinds_error": kinds_error,
        "recipes": {
            "file": "kinds.jsonl in JEVIFY_CONFIG_DIR, or the platform configuration directory (~/Library/Application Support/jevify on macOS; $XDG_CONFIG_HOME/jevify or ~/.config/jevify on Linux)",
            "line": "one JSON object per line: kind (required, [a-z][a-z-]*), list (required, the lister argv), field N (1-based whitespace field) or key KEY (a key of a JSON value) as the handle, default the whole line, ordered (default false: the lister prints newest first)",
            "example": "{\"kind\":\"pod\",\"list\":[\"kubectl\",\"get\",\"pods\",\"--no-headers\"],\"field\":1}",
            "rules": [
                "jevify reads no recipe from a repository or the working directory; a user recipe is the user's own command, as an alias is",
                "a user recipe cannot replace a coded kind or a shipped recipe",
                "the user's file is read only for a kind that is neither coded nor shipped; then any bad line is exit 6 recipe_invalid with its line number, whichever kind was asked for",
                "every lister runs with one 20 s deadline, stdin at /dev/null, GH_PROMPT_DISABLED=1, GIT_TERMINAL_PROMPT=0 and NO_COLOR=1; a tool that is missing, not logged in or rate-limited is exit 6 lister_failed with the tool's own text",
                "a list above the limit: an ordered kind keeps its newest part and the status line says candidates N of M, newest first; any other kind is exit 6 too_many",
                "a list with no kind is a pipe into '@{-:...}', shaped by sed, cut or jq first"
            ]
        },
        "withheld": { "patterns": WITHHELD_PATTERNS, "rule": "a path with a component matching one pattern is listed and its excerpt withheld; symlink files receive no excerpt; a file that cannot be read (missing, a directory, a permission or sandbox denial) is named on stderr as excerpt unreadable: PATH: REASON, and filter and label leave it unsure without asking, with unreadable: REASON on its record; the status line counts both as excerpts withheld: N" },
        "error_kinds": error_kinds,
        "input_errors": { "exit": 6, "field": "error.kind", "kinds": input_error_kinds },
        "exec_status": {
            "env": crate::cmd::fill::STATUS_FILE_ENV,
            "verb": "fill",
            "written": "one JSON object, before the exec, for every outcome fill decides: a resolution, an abstention and an error alike",
            "fields": ["command", "version", "exit_code", "ran", "argv", "reason", "markers", "error"],
            "rule": "ran true means fill handed the process to the command, so the exit code the caller observes is the command's own; ran false, or no file at all, means nothing ran and the exit code is jevify's. Read it instead of the stderr prefix; exec mode has no envelope",
            "dry_run": "a successful dry run is exit_code 0 with ran false: the argv was resolved and nothing was started",
            "unwritable": "exit 6, error.kind status_file_unwritable, and nothing runs: jevify never starts a command it cannot report having started",
            "inheritance": "the command inherits the variable; unset it in a wrapper when the command itself runs jevify fill",
            "handoff": "the one case a status file cannot cover is execvp itself failing after the write: exit 6, error.kind cannot_run, on the jevify fill: stderr line"
        },
        "fill_abstention": { "exit": 3, "error": null, "field": "data.reason", "marker_field": "data.markers[].reason", "order": "first failed marker in argv order", "reasons": ["no_match", "ambiguous", "unsure_flag", "insufficient_evidence"] },
        "fill_model_guard": { "exit": 4, "kind": "api_unavailable", "message": "answered by <model>, not Jev", "missing_model": "unknown", "rule": "every answering model must be Jev, including under --dry-run" },
        "selection_limits": {
            "formula": "W = backend window; fill F = W * (W / 3), integer division; pick and pick --from min(W * W, 20000)",
            "typesafe": { "window": 200, "fill": 13200, "pick": 20000, "one_options": 200 },
            "classifier": { "window": 99, "fill": 3267, "pick": 9801, "one_options": 99 },
            "finalists_per_window": "fill: 3 names per window in the shortlist round; with one window, a commit, file or dir marker always runs its finals, and a branch marker runs them when the names leave it undecided or the runner-up stays in play; for branch, commit, file or dir every name with p > 0 reaches the finals with its evidence, up to 24, and a commit takes the names at p 0.00 as well, since a subject that scores 0.00 may still be the commit holding the change; pick and pick --from: 3 if 3 * windows <= W, else 2 if 2 * windows <= W, else 1; by rank, never cross-request probability",
            "measured": "2026-09-24: classifier.dev answers a fill of 3267 candidates in 6.3 s median at JEVIFY_CONCURRENCY=4 and TypeSafe one of 13200 in 3.5 s at 8; every sweep point from 500 up answered on both backends, benchmarks/results.md",
            "overflow": "ordered kinds retain newest candidates and report coverage; unordered lists return too_many; one above W options is exit 2"
        },
        "env": [
            { "name": "TYPESAFE_API_KEY", "meaning": "API key (never printed); its presence selects the typesafe backend" },
            { "name": "TYPESAFE_API_KEY_FILE", "meaning": "path to a file holding the key (read only when a key is needed)" },
            { "name": "JEVIFY_BACKEND", "default": "typesafe with a key, classifier without one", "meaning": "typesafe|classifier: which API answers. Classifier is free and needs no key; meta.model identifies the service-controlled answering model" },
            { "name": "JEVIFY_BASE_URL", "default": "the active backend's own URL", "meaning": "HTTPS on port 443 at api.typesafe.ai for typesafe or classifier.dev for classifier; localhost/127.0.0.1 allow any scheme and port; no userinfo or redirects" },
            { "name": "JEVIFY_MODEL", "default": "jev-1.13.0", "meaning": "Default applies to TypeSafe model selection; jev-latest moves with each release. Explicit overrides are rejected on classifier.dev, which controls its model" },
            { "name": "JEVIFY_THRESHOLD", "default": 0.5 },
            { "name": "JEVIFY_CONCURRENCY", "default": "8 on typesafe, 4 on classifier" },
            { "name": "JEVIFY_DEADLINE", "default": 600, "meaning": "seconds for the whole verb; expiry is exit 4, error.kind api_deadline, apart from the transport failure api_unavailable" },
            { "name": "JEVIFY_STATUS_FILE", "meaning": "path fill writes its exec status to before starting a command: see exec_status. Unset by default, and nothing is written then" },
            { "name": "JEVIFY_CACHE_DIR", "default": "platform cache dir/jevify" },
            { "name": "JEVIFY_CONFIG_DIR", "default": "platform config dir/jevify", "meaning": "where the user's kinds.jsonl lives; read only for a kind that is neither coded nor shipped" },
            { "name": "JEVIFY_NO_CACHE", "meaning": "disable the answer cache and route's tool inventory file (answer entries are ignored after 7 days anyway, but their files stay); it does not disable saved inputs or the tool kind's inventory file, and it moves sort --apply's recovery journal to the system temporary directory rather than suppressing it. See answer_cache" },
            { "name": "JEVIFY_NO_SAVE", "meaning": "set to 1 so why and filter save no raw input anywhere in a fleet, without every call site passing --no-save; saved_input is null and complete false" },
            { "name": "JEVIFY_DECISION", "meaning": "round_one: add meta.decision.round_one, every candidate of every window of round one and the finals as sent, to the envelope of a verb that ran a tournament" },
            { "name": "JEVIFY_PRICE_PER_MTOK", "default": 0.042 },
            { "name": "JEVIFY_INVENTORY_FILE", "meaning": "JSON array of {name, summary} replacing the PATH inventory (tests, evals)" },
            { "name": "JEVIFY_CNF", "meaning": "enable the command-not-found hook from `jevify init`" }
        ],
        "limits": { "choice_options": 255, "window": crate::tournament::WINDOW, "state_tokens": 32000, "request_tokens": 64000, "requests_per_minute": 1200, "tokens_per_second": 250000, "stdin_bytes": crate::input::MAX_BYTES, "pick_lines": crate::cmd::pick::MAX_LINES, "distinct_records": 20000, "records_per_request": { "classifier": crate::jev::classifier::KEYLESS_DECISIONS, "typesafe": 20 }, "too_many": "exit 6, error.kind too_many: narrow distinct records with grep or head", "connect_timeout_s": 5, "request_read_timeout_s": 60, "timeouts": "one request waits 5 s for the connection and 60 s for the response, then counts as a transport failure and is retried within the JEVIFY_DEADLINE budget; a truncated response costs the full 60 s" },
        "answer_cache": answer_cache,
        "saved_inputs": { "verbs": ["why", "filter"], "directory": "JEVIFY_CACHE_DIR/outputs, or the platform cache directory/jevify/outputs", "filename": "<blake3-16>.log", "contents": "raw input bytes, secrets included", "retention": "7 days, matching the answer cache; a save deletes the store's own files past it and nothing else, never descending or following a symlink, and saving the same input again refreshes its file", "disable": "--no-save for one call, JEVIFY_NO_SAVE=1 for every call (both independent of --no-cache)", "permissions": "directory 0700, file 0600", "incomplete": "failed or skipped save: saved_input null, complete false" },
        "backends": [
            { "name": "typesafe", "key": "required", "model": "Jev", "window": Backend::Typesafe.window(), "choice_options": 255, "state_chars": "32k tokens", "requests_per_minute": 1200, "meta": "input_tokens is null unless every inference attempt reports usage; cost_usd estimates input-token cost at the configured price and is null when that basis is incomplete" },
            { "name": "classifier", "key": "none", "model": "service-controlled Jev; explicit model overrides unsupported", "decision_semantics": "two-label Choice substitutes for Noul; scores and thresholds are not assumed interchangeable with TypeSafe Noul", "window": Backend::Classifier.window(), "choice_options": crate::jev::classifier::MAX_LABELS, "state_chars": crate::jev::classifier::MAX_INPUT_CHARS, "questions_per_request": crate::jev::classifier::MAX_DIMENSIONS, "classifications_per_minute": 3000, "classifications_per_day": 20000, "classification": "one record under one question, per IP", "cost_per_call": keyless_cost, "calls_per_day": keyless_calls, "basis": "computed from request shapes; the 20,000 a day quota, the is and filter costs and the 60-record batch measured 2026-09-22 with JEVIFY_CONCURRENCY=4, benchmarks/results.md", "meta": "input_tokens is null when token usage is unavailable; cost_usd is 0 at the default zero service price, with an explicit telemetry.cost_estimate basis" }
        ],
        "envelope": { "fields": ["ok", "command", "version", "exit_code", "data", "meta{backend,model,elapsed_ms,requests,cache_hits,input_tokens,cost_usd,threshold,request_id,usage,telemetry,decision}", "error{kind,message,hint,example}"], "ok": "not the field to branch on: it says only that jevify itself reached the end without an error of its own, so it is true on exit 0, true on exit 1 where is answers no, and true on exit 3 where a verb abstains and the data a caller expects is absent", "branch_on": "exit_code, which equals the process exit code; then read data", "decision": { "fields": "decision{verb,backend,model{requested,answering},threshold,gates[{best,next,none,any,fails}],round_one?[{windows[{ranks[{index,p}],none,any}],finalists[],n}]}", "round_one": "present only under JEVIFY_DECISION=round_one, on a verb that ran a tournament; absent otherwise, since it holds every candidate of every window. One entry per tournament, in decision order (fill one per listing marker, pick, why and route one): every window of round one in input order with every candidate by rank, its P(NONE) and Noul; finalists, the items the finals request held, in its order (the shortlist's n per window by rank then window, widened by fill, joined by why's panic lines, capped at 12 by route), empty when one window decided alone; and n. index is the verb's own number, 1-based: the line for why, the record for pick, the listing position for pick --from and fill, the inventory position for route", "model": "requested is the model the request names (null on classifier, which chooses its own); answering is what the service reported, unknown when it did not say", "gates": "one entry per decision in decision order: fill one per marker, is one per statement, filter and label one per judged record, add one per hunk, sort one per file, pick, why and route one; best and next are the two top Choice probabilities, none is P(NONE), any is the Noul (the whole answer of a yes/no question), fails is P(the record says the statement does not hold) on filter, the side that produces a no; a score the verb does not use is null", "threshold": "the one threshold in force; inside a gate it is compared to any alone, never to best: a selection resolves when best beats next and none and reaches twice the larger of them, so a winner's own p can sit below the threshold, and p is relative to the pool it was scored in; no calibration is claimed" } },
        "telemetry": {
            "attempt_groups": ["inference_posts", "health_gets", "prewarm_gets", "semantic_calls"],
            "conservation": "attempted = succeeded + failed + cancelled + in_flight",
            "transport_success": "HTTP 200 with the full response body received; semantic parsing and validation are counted separately",
            "semantic_questions": "questions submitted to ask, including cache hits and locally rejected calls",
            "retries": "retry_sends counts sends after the initial attempt; retry_sleep_ms counts elapsed completed or interrupted retry waits",
            "usage": "input_tokens and output_tokens each expose reported_subtotal, reported_attempts, unknown_attempts, complete; reported_attempts + unknown_attempts = inference_posts.attempted; cache hits add no service usage",
            "logical_rounds": "null: client-level accounting cannot infer logical rounds",
            "cost_estimate": "configured input price and reported input subtotal; complete is false for unknown input usage unless the configured price is zero"
        },
        "phrasing": [
            "write what must be true of the text, literally: the statement is judged word for word (\"the customer is about to stop being a customer\" beats \"this customer is about to leave\", which also matches an employee who is leaving their company)",
            "describe the thing, not what you will do with it: \"the line with the failing assertion\", not \"what should I fix\"",
            "one process for many records: use filter rather than a loop of is; is accepts several statements about one context",
            "English works best; jevify does not count, do arithmetic, compare dates or judge quality"
        ],
        "workflows": [
            { "goal": "preview a command with a described branch", "command": "jevify fill --dry-run -- git switch '@{branch:the auth refactor}'" },
            { "goal": "get a branch handle without running a command", "command": "jevify pick --from branch 'the auth refactor'" },
            { "goal": "revert a described commit", "command": "jevify fill --dry-run -- git revert '@{commit:made folder moves atomic}'" },
            { "goal": "open a described file under one directory", "command": "jevify fill --dry-run -- cat 'src/@{file:parses the marker}'" },
            { "goal": "find the tool for a task", "command": "jevify route --json '<task>'" },
            { "goal": "explain a failure", "command": "<cmd> 2>&1 | jevify why --json" },
            { "goal": "explain a failed CI run, however long the log", "command": "gh run view --log-failed | jevify why --json" },
            { "goal": "select an item", "command": "<list> | jevify pick --json \"<intent>\"" },
            { "goal": "branch in a script", "command": "jevify is \"<condition>\" < file; case $? in 0) ...;; 1) ...;; 3) ...;; esac" },
            { "goal": "triage many files without reading them", "command": "fd -0 -e txt | jevify filter -0 --files '<statement>'" },
            { "goal": "count records by meaning", "command": "gh issue list | jevify label bug,feature,question | cut -f1 | sort | uniq -c" },
            { "goal": "stage one topic out of a mixed working tree", "command": "jevify add --json --dry-run \"<topic>\"   # then --yes, when the user asked you to stage" },
            { "goal": "run a resolved command unattended and still know whether it started", "command": "S=$(mktemp); JEVIFY_STATUS_FILE=$S jevify fill -- CMD '@{branch:<description>}'; code=$?; jq -e .ran \"$S\" >/dev/null && echo \"the command exited $code\" || echo \"nothing ran, jevify exited $code\"" }
        ],
        "safety": [
            "exec mode has no envelope: JEVIFY_STATUS_FILE is the only machine-readable way to tell an abstention from the command's own exit code, and the stderr prefix is not a protocol",
            "error.kind is enumerated in error_kinds with the exit code each kind carries; branch on the kind, never on the message text",
            "allow fill --dry-run freely; authorize fill per command prefix, exactly as the underlying command; jevify is not a permission system",
            "meta.requests counts attempted inference POSTs, including retries and failures, excluding prewarm and health GETs",
            "route, why, pick, filter, label and is start no user command; route prints a tool and the caller writes its arguments; a near-tie (fit within 0.10 of the best) is exit 3 with tool null and the tied commands in data.ties, so read ties before writing the command",
            "is abstains without an API request when input exceeds its evidence budget: exit 3, p null, verdict unsure, truncated true, and a reason; stderr warns that the whole input was not judged",
            "add rejects any hunk above 3000 characters or any complete batch exceeding the backend state budget before API requests or staging; it never classifies clipped evidence",
            "filter and label judge records alone only on classifier.dev; on TypeSafe 20 records share one request and each question names its record",
            "rate_limit_day HTTP 429: exit 4, daily quota of the free backend reached; never retried",
            "meta.model is a string; several answering models are joined with comma and space",
            "meta.decision carries verb, backend, model.requested, model.answering, threshold and the gate scores of every decision; scores are backend-specific and not a calibration",
            "only add stages hunks and sort --apply or --undo moves files; caller authorization remains required",
            "obvious secrets are masked before text is sent (best effort)",
            "results are pointers into input, the machine, or man pages; nothing is generated"
        ]
    });
    // Outside the literal: one more key would pass the json! macro's recursion limit.
    data["argument_shapes"] = argument_shapes;
    Outcome {
        exit: Exit::Ok,
        human: format!("{}\n", serde_json::to_string_pretty(&data).unwrap()).into_bytes(),
        exec: None,
        data,
    }
}

pub fn robot_docs(topic: Option<&str>) -> Result<Outcome, JevifyError> {
    let caps = capabilities().data;
    let text = match topic.unwrap_or("guide") {
        "guide" => GUIDE.to_string(),
        "commands" => serde_json::to_string_pretty(&caps["commands"]).unwrap(),
        "exit-codes" => serde_json::to_string_pretty(&caps["exit_codes"]).unwrap(),
        "examples" => serde_json::to_string_pretty(&caps["workflows"]).unwrap(),
        "privacy" => include_str!("../../PRIVACY.md").to_string(),
        other => {
            return Err(JevifyError::Usage(format!(
                "unknown topic `{other}`; topics: guide, commands, exit-codes, examples, privacy"
            )));
        }
    };
    Ok(Outcome {
        exit: Exit::Ok,
        data: serde_json::json!({ "topic": topic.unwrap_or("guide"), "text": text }),
        human: format!("{text}\n").into_bytes(),
        exec: None,
    })
}

pub async fn health(ctx: &Config) -> Result<Outcome, JevifyError> {
    let base = crate::config::base_url(ctx.backend, Some(&ctx.base_url))?;
    // Both backends are probed the same way, at the cheapest endpoint each offers; only
    // TypeSafe needs a key, and only there can the answer be "the key is wrong".
    let (path, key) = match ctx.backend {
        Backend::Typesafe => ("/v1/models", Some(ctx.api_key()?)),
        Backend::Classifier => ("/v1/health", None),
    };
    let mut req = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| JevifyError::Unavailable(e.to_string()))?
        .get(format!("{base}{path}"))
        .timeout(std::time::Duration::from_secs(5));
    if let Some(k) = &key {
        req = req.bearer_auth(k);
    }
    let start = std::time::Instant::now();
    let mut attempt = ctx.stats.start(crate::jev::client::AttemptKind::Health);
    let r = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            attempt.finish(false);
            return Err(JevifyError::Unavailable(e.to_string()));
        }
    };
    let ms = start.elapsed().as_millis();
    let backend = ctx.backend.as_str();
    match r.status().as_u16() {
        200 => {
            let bytes = match r.bytes().await {
                Ok(bytes) => bytes,
                Err(e) => {
                    attempt.finish(false);
                    return Err(JevifyError::Unavailable(e.to_string()));
                }
            };
            attempt.finish(true);
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            let key_state = if key.is_some() {
                "present"
            } else {
                "not needed"
            };
            Ok(Outcome {
                exit: Exit::Ok,
                human: format!("ok: {backend} reachable in {ms} ms (key {key_state})\n")
                    .into_bytes(),
                exec: None,
                data: serde_json::json!({
                    "backend": backend,
                    "base_url": ctx.base_url,
                    "key": key_state,
                    "api": "reachable",
                    "latency_ms": ms,
                    "models": body["models"],
                }),
            })
        }
        401 | 403 => {
            attempt.finish(false);
            Err(JevifyError::BadKey(r.status().as_u16()))
        }
        s => {
            attempt.finish(false);
            Err(JevifyError::Unavailable(format!("HTTP {s}")))
        }
    }
}

pub fn init(shell: Shell) -> Outcome {
    if matches!(shell, Shell::Agents) {
        let capabilities = capabilities().data;
        let verbs = capabilities["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|command| {
                let name = command["name"].as_str().unwrap();
                match (command["when"].as_str(), command["example"].as_str()) {
                    (Some(when), Some(example)) => format!("- {name}: {when}: {example}"),
                    _ => format!("- {name}: {}", command["usage"].as_str().unwrap()),
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let kinds = capabilities["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|kind| kind["name"].as_str().unwrap())
            .filter(|name| !matches!(*name, "-" | "one" | "flag"))
            .collect::<Vec<_>>()
            .join(", ");
        let exits = capabilities["exit_codes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| format!("{} {}", entry["code"], entry["meaning"].as_str().unwrap()))
            .collect::<Vec<_>>()
            .join("; ");
        let block = format!(
            "# jevify\nUse meaning when literal search cannot answer; cheap tools go first. Input: fill real arguments; output: select existing records; never generate text.\nShapes that work: jevify pick --from commit names the borrow in the return type (a description is quoted or bare words); options go before or after the verb; -C DIR runs fill or pick in DIR, and fill lists where git -C DIR points; pick --files with nothing on stdin ranks the working tree. Every error ends with try: <your command, corrected>; an abstention's nearest (not chosen) candidates are never the answer.\n{verbs}\nMore fill: jevify fill -- git show '@{{commit:made folder moves atomic}}'; jevify fill -- gh run view --log-failed '@{{ci-run:the failed run on tag v0.7.0}}' | jevify why; git log --oneline | jevify fill -- git revert '@{{-:the pricing change}}'\nKinds for '@{{kind:description}}' and pick --from KIND: {kinds}; - for piped candidates; one and flag write an option: '@{{one:bug|feature|docs:what kind of report}}', '@{{flag:--draft:it lacks steps to reproduce}}'.\nUse one filter or label call for many records, not a loop of is calls or one read per file; label prints LABEL<TAB>RECORD, ? when unsure.\nfilter keeps the records the statement holds for and the records that do not say it either way; --strict keeps only the ones it holds for. Write the condition so that yes means act; check pick's exit before using its output.\nQuote the whole marker argument: jevify fill --dry-run -- git switch '@{{branch:the auth refactor}}'; never eval the preview.\nOnly why and filter save raw input, secrets included; --no-save disables saving.\nAllow fill --dry-run freely; authorize fill per command prefix, add staging and sort moves.\nExit codes: {exits}. fill: 2–6 nothing ran; after exec the command owns its exit code.\nUse jevify capabilities --json as the source of truth for commands, kinds and flags.\n"
        );
        return Outcome {
            exit: Exit::Ok,
            data: serde_json::json!({ "script": block }),
            human: block.into_bytes(),
            exec: None,
        };
    }
    let body = match shell {
        Shell::Zsh => {
            r#"# jevify shell integration — add to ~/.zshrc: eval "$(jevify init zsh)"
alias ,='noglob jevify route'
# Opt-in: route unknown commands of 3+ words to jevify (export JEVIFY_CNF=1).
if [[ -n $JEVIFY_CNF ]] && ! (( $+functions[command_not_found_handler] )); then
  command_not_found_handler() {
    if (( $# >= 3 )); then jevify route "$*"; return $?; fi
    print -u2 "zsh: command not found: $1"; return 127
  }
fi
"#
        }
        Shell::Bash => {
            r#"# jevify shell integration — add to ~/.bashrc: eval "$(jevify init bash)"
alias ,='jevify route'
if [[ -n $JEVIFY_CNF ]] && ! declare -F command_not_found_handle >/dev/null; then
  command_not_found_handle() {
    if (( $# >= 3 )); then jevify route "$*"; return $?; fi
    echo "bash: $1: command not found" >&2; return 127
  }
fi
"#
        }
        Shell::Agents => unreachable!("handled above"),
    };
    Outcome {
        exit: Exit::Ok,
        data: serde_json::json!({ "script": body }),
        human: body.as_bytes().to_vec(),
        exec: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capabilities_match_the_exit_contract_and_name_every_command() {
        let d = capabilities().data;
        // An observable value, not the list's own length echoed back: 130 is what an agent sees
        // when the user declines at the prompt.
        assert!(
            d["exit_codes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["code"] == 130),
            "{}",
            d["exit_codes"]
        );
        assert_eq!(d["limits"]["window"], crate::tournament::WINDOW);
        // Both backends are documented, with the env var that picks one.
        let names: Vec<&str> = d["backends"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["typesafe", "classifier"]);
        assert!(
            d["env"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["name"] == "JEVIFY_BACKEND")
        );
        assert!(
            d["envelope"]["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f.as_str().unwrap().contains("meta{backend,"))
        );
        assert!(
            d["commands"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["name"].is_string()
                    && c["usage"].is_string()
                    && c["data"].is_string()
                    && c["exit"].as_array().is_some_and(|codes| !codes.is_empty()))
        );
        // An agent must learn from here when each verb is worth a call, with a command to copy.
        assert_eq!(
            d["commands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            crate::VERBS
        );
        for verb in [
            "fill", "pick", "why", "route", "filter", "label", "is", "add", "sort",
        ] {
            let c = d["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == verb)
                .unwrap();
            assert!(c["when"].as_str().is_some_and(|s| !s.is_empty()), "{c}");
            assert!(
                c["example"]
                    .as_str()
                    .is_some_and(|s| s.contains(&format!("jevify {verb}"))),
                "{c}"
            );
        }
    }
    #[test]
    fn capabilities_list_every_coded_kind_and_shipped_recipe_with_its_argv() {
        let d = capabilities().data;
        let kinds = d["kinds"].as_array().unwrap();
        // The coded kinds and the shipped recipes come first, in registry order; a developer's
        // own recipes may follow, then the two caller-option kinds.
        let names: Vec<&str> = kinds.iter().map(|k| k["name"].as_str().unwrap()).collect();
        assert!(names.starts_with(crate::source::KINDS), "{names:?}");
        assert_eq!(&names[names.len() - 2..], ["one", "flag"]);
        for kind in kinds {
            assert!(kind["list"].is_array(), "{kind}");
            assert!(
                ["coded", "shipped", "user"].contains(&kind["origin"].as_str().unwrap()),
                "{kind}"
            );
        }
        let pod = kinds.iter().find(|k| k["name"] == "pod").unwrap();
        assert_eq!(pod["origin"], "shipped");
        assert_eq!(
            pod["list"],
            serde_json::json!(["kubectl", "get", "pods", "--no-headers"])
        );
        assert_eq!(pod["ordered"], false);
        let pr = kinds.iter().find(|k| k["name"] == "pr").unwrap();
        assert_eq!(pr["ordered"], true);
        for name in ["-", "tool", "one", "flag"] {
            let kind = kinds.iter().find(|k| k["name"] == name).unwrap();
            assert_eq!(kind["list"], serde_json::json!([]), "{kind}");
        }
        assert_eq!(
            d["withheld"]["patterns"],
            serde_json::json!(WITHHELD_PATTERNS)
        );
        assert!(
            d["recipes"]["file"]
                .as_str()
                .unwrap()
                .contains("JEVIFY_CONFIG_DIR")
        );
        assert!(
            d["env"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["name"] == "JEVIFY_CONFIG_DIR")
        );
    }
    #[test]
    fn robot_docs_default_to_the_guide_and_reject_unknown_topics() {
        assert_eq!(robot_docs(None).unwrap().data["topic"], "guide");
        // `Outcome` has no Debug, so `unwrap_err` is unavailable; `err()` needs only the error's.
        assert_eq!(robot_docs(Some("nope")).err().unwrap().exit(), Exit::Usage);
    }
    #[test]
    fn init_prints_the_comma_alias_for_both_shells() {
        for shell in [Shell::Zsh, Shell::Bash] {
            let script = String::from_utf8(init(shell).human).unwrap();
            assert!(script.contains("alias ,=") && script.contains("jevify route"));
            assert!(!script.contains(&["jevify", "run"].join(" ")));
        }
    }
    #[test]
    fn init_agents_names_every_verb_and_the_capability_contract() {
        let block = String::from_utf8(init(Shell::Agents).human).unwrap();
        for verb in crate::VERBS {
            assert!(block.contains(verb));
        }
        assert!(block.contains("jevify capabilities --json"));
        assert!(block.lines().count() <= 25);
        // Every situation comes with a complete command on the same line, and every kind
        // that lists something is named, so an agent with only this block can write the call.
        for pair in [
            (
                "- fill: about to list branches",
                "jevify fill -- git switch '@{branch:",
            ),
            ("- why: a failed build", "| jevify why"),
            ("- filter: many records", "jevify filter -0 --files"),
            ("- label: every record", "jevify label --files"),
            ("- pick: one record or file", "jevify pick --from commit"),
        ] {
            let line = block
                .lines()
                .find(|l| l.starts_with(pair.0))
                .unwrap_or_default();
            assert!(line.contains(pair.1), "{line}");
        }
        assert!(block.contains("'@{commit:") && block.contains("'@{ci-run:"));
        let kinds = block.lines().find(|l| l.starts_with("Kinds")).unwrap();
        for kind in [
            "branch",
            "commit",
            "file",
            "dir",
            "tool",
            "pr",
            "issue",
            "ci-run",
            "stash",
            "process",
            "container",
            "pod",
        ] {
            assert!(kinds.contains(kind), "{kind}");
        }
    }
}
