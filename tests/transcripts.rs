//! Every transcript of `README.md` and `docs/guide/getting-started.md`, run against a real
//! backend, checked against the answer the page shows.
//!
//! `tests/agent.rs` already walks the documented shell examples, but it answers them with a
//! fake, so it proves the command parses and exits 0. It cannot notice that the binary now
//! names a different commit. Three transcripts drifted that way before this file existed: a
//! commit demo naming a commit the listing no longer holds, a two-candidate `fill` abstaining
//! where the page printed a resolution, and a nothing-fits demo resolving. Each was true the
//! day it was written.
//!
//! **What is compared, and what is not.** Only the decision (resolved or abstained, kept or
//! dropped, which bucket) and the chosen item. Never the probability, never the candidate,
//! window or request counts, never the elapsed time. A probability is the backend's score on
//! the task and it moves between model versions, between the two backends and between two runs
//! of the same input; a page that promised a number would fail every week for no reader's
//! benefit. The page's reader acts on the item: if `fill` still names `317cbf7` the demo is
//! honest whether it says 0.99 or 0.73. The numbers printed on the pages carry the date and
//! the backend they were measured on, which is the honest claim to make about them.
//!
//! **Where the expectations live.** In the `transcripts()` table below, not extracted from the Markdown.
//! Parsing a console block for "the answer" means guessing which lines of a sample output are
//! the decision and which are the status line, a guess that breaks whenever a status line gains
//! a field. Each row instead carries `shown`, the command exactly as the page prints it after
//! `$`, and `cites`, the page and the line it was read from. `every_expectation_matches_the_page_it_cites`
//! closes the loop from the other side: it finds the console block that prints `shown` and
//! checks that every expected item appears in it. So a hand edit to a documented answer, or to
//! a documented command, fails here; the table and the pages cannot drift apart silently. The
//! recorded line number is for the reader — the block is found by its command text, so an
//! unrelated edit further up the page moves no expectation.
//!
//! **Demos whose input is this repository.** The history grows, so `git log --oneline -30` is a
//! different list every week and the commit it named falls out of the window: an input that
//! grows cannot carry a documented answer. The commit demo asks for a range between two release
//! tags instead. Tags do not move, so the reader, the page and this check all judge the same
//! thirty commits, and the row's `argv` is the page's own command rather than a pinned stand-in
//! for it. The other listing, `git ls-files`, is deliberately not fixed: `pick --files` reads
//! the first lines of each path, so it runs over the current checkout, and its expectation —
//! that `src/cli.rs` is still where the flags are defined — is a claim about this repository
//! that should fail if it ever stops being true.
//!
//! Live, and never part of the ordinary gate: every test here is `#[ignore]`.
//!
//! ```sh
//! cargo test --test transcripts -- --ignored --test-threads=1                    # keyless
//! TYPESAFE_API_KEY_FILE=/path/to/key \
//!     cargo test --test transcripts -- --ignored --test-threads=1                # both backends
//! ```
//!
//! Without a key the TypeSafe half prints `SKIPPED` and returns; with no network, or a backend
//! that answers with a model other than Jev, each case prints `SKIPPED` and the run reports
//! nothing about the pages. A hand-off lists a skipped backend as NOT RUN, never as passed.
//! Backend usage depends on the retained examples and request shapes.

use serde_json::Value;

const README: &str = "README.md";
const GETTING_STARTED: &str = "docs/guide/getting-started.md";

/// The page and the line this expectation was read from, for the reader and for the failure
/// message. The line is recorded, not searched on.
struct Cite(&'static str, u32);

/// What the page shows the binary deciding.
#[derive(Clone, Copy)]
enum Expect {
    /// Exit 0, and these items chosen, in this order: the picked lines, the kept records, the
    /// labels in record order, or the marker handles in marker order.
    Chose(&'static [&'static str]),
    /// Exit 3 and nothing chosen. The demos that promise an honest "nothing fits".
    NothingFits,
}

/// Where the records come from: nothing, a literal the page prints, a fixture of `docs/demo`,
/// or a listing this repository produces.
enum Stdin {
    Text(&'static str),
    File(&'static str),
    /// A listing this repository produces, run in the repository root as argv, not a shell
    /// line.
    Listing(&'static [&'static str]),
}

struct Transcript {
    /// The command as the page prints it after `$`, used to find the console block.
    shown: &'static str,
    /// What the check runs: the page's command, with `--json` added by the runner.
    argv: &'static [&'static str],
    stdin: Stdin,
    expect: Expect,
    /// What the console block prints for this decision, when that is not the chosen item
    /// itself: `is` answers with an exit code, so its block prints what the `&&` ran.
    on_page: &'static [&'static str],
    cites: &'static [Cite],
}

/// The thirty commits between two release tags. Tags do not move, so this is the same listing
/// for the check, for the page and for a reader who types the command, however far the branch
/// moves on: the argv below is the page's command, not a pinned stand-in for it. An input that
/// grows cannot be an expectation.
const RELEASE_LOG: &[&str] = &["git", "log", "--oneline", "v0.8.3..v0.9.3"];

fn transcripts() -> Vec<Transcript> {
    vec![
        Transcript {
            shown: "jevify why < docs/demo/build.log",
            argv: &["why"],
            stdin: Stdin::File("docs/demo/build.log"),
            expect: Expect::Chose(&["error[E0425]: cannot find value `conifg` in this scope"]),
            on_page: &[],
            cites: &[Cite(README, 37), Cite(GETTING_STARTED, 49)],
        },
        Transcript {
            shown: "git log --oneline v0.8.3..v0.9.3 | jevify fill --field 1 --dry-run -- git show --stat --format=%s '@{-:made route abstain when two commands are too close}'",
            argv: &[
                "fill",
                "--field",
                "1",
                "--dry-run",
                "--",
                "git",
                "show",
                "--stat",
                "--format=%s",
                "@{-:made route abstain when two commands are too close}",
            ],
            stdin: Stdin::Listing(RELEASE_LOG),
            expect: Expect::Chose(&["317cbf7"]),
            on_page: &[],
            cites: &[Cite(GETTING_STARTED, 136)],
        },
        // README's promise under the nothing-fits demo: "when no commit fits your description,
        // no command runs". Over the same release listing, which holds no such commit.
        Transcript {
            shown: "jevify fill --dry-run -- git show '@{commit:ports the user interface to Android}'",
            argv: &[
                "fill",
                "--field",
                "1",
                "--dry-run",
                "--",
                "git",
                "show",
                "@{-:ports the user interface to Android}",
            ],
            stdin: Stdin::Listing(RELEASE_LOG),
            expect: Expect::NothingFits,
            on_page: &[],
            cites: &[],
        },
        // The same call through `cut | sort | uniq -c`: the counts the page prints are these
        // ten labels counted, so the labels are the expectation and the counts follow.
        Transcript {
            shown: "jevify label bug,feature,question < docs/demo/issues.txt | cut -f1 | sort | uniq -c",
            argv: &["label", "bug,feature,question"],
            stdin: Stdin::File("docs/demo/issues.txt"),
            expect: Expect::Chose(&[
                "bug", "feature", "question", "bug", "feature", "question", "bug", "feature",
                "question", "bug",
            ]),
            on_page: &[],
            cites: &[Cite(GETTING_STARTED, 92)],
        },
        Transcript {
            shown: "jevify filter 'reports a crash' < docs/demo/issues.txt",
            argv: &["filter", "reports a crash"],
            stdin: Stdin::File("docs/demo/issues.txt"),
            expect: Expect::Chose(&[
                "#312 Crash when the config file is empty",
                "#290 Panic on non-UTF-8 file names",
            ]),
            on_page: &[],
            cites: &[Cite(GETTING_STARTED, 62)],
        },
        Transcript {
            shown: "printf 'build started\\nerror: connection timed out\\nbuild stopped\\n' | jevify filter --strict 'reports a network failure'",
            argv: &["filter", "--strict", "reports a network failure"],
            stdin: Stdin::Text("build started\nerror: connection timed out\nbuild stopped\n"),
            expect: Expect::Chose(&["error: connection timed out"]),
            on_page: &[],
            cites: &[Cite(GETTING_STARTED, 79)],
        },
        Transcript {
            shown: "jevify pick 'what I paid a streaming service' < docs/demo/downloads.txt",
            argv: &["pick", "what I paid a streaming service"],
            stdin: Stdin::File("docs/demo/downloads.txt"),
            expect: Expect::Chose(&["spotify_receipt.pdf"]),
            on_page: &[],
            cites: &[Cite(GETTING_STARTED, 68)],
        },
        Transcript {
            shown: "jevify pick 'the tax return' < docs/demo/downloads.txt",
            argv: &["pick", "the tax return"],
            stdin: Stdin::File("docs/demo/downloads.txt"),
            expect: Expect::NothingFits,
            on_page: &[],
            cites: &[Cite(GETTING_STARTED, 119)],
        },
        Transcript {
            shown: "jevify is 'asks for a refund' < docs/demo/mail.txt && echo refund",
            argv: &["is", "asks for a refund"],
            stdin: Stdin::File("docs/demo/mail.txt"),
            expect: Expect::Chose(&["yes"]),
            on_page: &["refund"],
            cites: &[Cite(GETTING_STARTED, 103)],
        },
        Transcript {
            shown: "git ls-files | jevify pick --files 'where the command-line flags are defined'",
            argv: &[
                "pick",
                "--files",
                "where the command-line flags are defined",
            ],
            stdin: Stdin::Listing(&["git", "ls-files"]),
            expect: Expect::Chose(&["src/cli.rs"]),
            on_page: &[],
            cites: &[Cite(GETTING_STARTED, 110)],
        },
    ]
}

fn root() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A listing of this repository, as the page's pipeline produces it.
fn listing(argv: &[&str]) -> String {
    let out = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .current_dir(root())
        .output()
        .unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    assert!(
        out.status.success(),
        "{argv:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The items the run chose, in the order the page prints them: the picked lines, the kept
/// records, the label of each record, the handle of each marker, the verdict of `is`.
fn chosen(verb: &str, data: &Value) -> Vec<String> {
    let texts = |key: &str, field: &str| -> Vec<String> {
        data[key]
            .as_array()
            .map(|records| {
                records
                    .iter()
                    // An abstained marker has a null handle: nothing was chosen.
                    .filter_map(|r| r[field].as_str())
                    .map(|text| text.trim_end().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    match verb {
        "why" => texts("causes", "text"),
        "pick" => texts("matches", "text"),
        "filter" => texts("records", "text"),
        "label" => texts("records", "label"),
        "fill" => texts("markers", "handle"),
        "is" => data["verdict"]
            .as_str()
            .map(|v| vec![v.to_owned()])
            .unwrap_or_default(),
        other => panic!("no rule for `{other}`"),
    }
}

/// A `jevify` on `backend`, or `None` (with SKIPPED on stderr) when TypeSafe has no key.
fn live_command(backend: &str) -> Option<assert_cmd::Command> {
    if backend == "typesafe"
        && std::env::var_os("TYPESAFE_API_KEY").is_none()
        && std::env::var_os("TYPESAFE_API_KEY_FILE").is_none()
    {
        eprintln!(
            "SKIPPED: set TYPESAFE_API_KEY_FILE=/path/to/key (or TYPESAFE_API_KEY) to check the transcripts on TypeSafe"
        );
        return None;
    }
    let mut cmd = assert_cmd::Command::cargo_bin("jevify").unwrap();
    cmd.current_dir(root())
        .env("JEVIFY_BACKEND", backend)
        .env("JEVIFY_NO_CACHE", "1")
        .env("JEVIFY_NO_SAVE", "1")
        .env("JEVIFY_CACHE_DIR", tempfile::tempdir().unwrap().keep());
    if backend == "classifier" {
        cmd.env_remove("TYPESAFE_API_KEY")
            .env_remove("TYPESAFE_API_KEY_FILE");
    }
    Some(cmd)
}

fn run_transcripts(backend: &str) {
    let mut requests = 0u64;
    let mut checked = 0;
    let mut skipped = 0;
    for case in transcripts() {
        let Some(mut cmd) = live_command(backend) else {
            return;
        };
        let stdin = match &case.stdin {
            Stdin::Text(text) => (*text).to_owned(),
            Stdin::File(path) => std::fs::read_to_string(root().join(path)).unwrap(),
            Stdin::Listing(argv) => listing(argv),
        };
        // `--json` goes right after the verb: on `fill` anything after `--` belongs to the
        // command being filled.
        let out = cmd
            .arg(case.argv[0])
            .arg("--json")
            .args(&case.argv[1..])
            .write_stdin(stdin)
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "{}: no envelope ({e}); stderr {}",
                case.shown,
                String::from_utf8_lossy(&out.stderr)
            )
        });
        // No network, no quota, no key: nothing is proved about the pages, and the run says so.
        if matches!(out.status.code(), Some(4) | Some(5)) {
            eprintln!(
                "SKIPPED: {backend} answered {}: {}",
                out.status.code().unwrap(),
                value["error"]["message"]
            );
            return;
        }
        let model = value["meta"]["model"].as_str().unwrap_or_default();
        if !jevify::jev::all_jev(model) {
            assert_eq!(
                backend, "classifier",
                "typesafe answered with `{model}`, not Jev: {value}"
            );
            eprintln!("SKIPPED: classifier.dev answered with `{model}`, not Jev");
            skipped += 1;
            continue;
        }
        requests += value["meta"]["requests"].as_u64().unwrap_or_default();
        let where_from = if case.cites.is_empty() {
            case.shown.to_owned()
        } else {
            case.cites
                .iter()
                .map(|Cite(doc, line)| format!("{doc}:{line}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let got = chosen(case.argv[0], &value["data"]);
        match case.expect {
            Expect::Chose(want) => {
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "{where_from} shows an answer, {backend} exited {:?}: {value}",
                    out.status.code()
                );
                assert_eq!(
                    got, want,
                    "{where_from} shows {want:?}, {backend} chose {got:?}: {value}"
                );
            }
            Expect::NothingFits => {
                assert_eq!(
                    out.status.code(),
                    Some(3),
                    "{where_from} shows nothing fitting, {backend} exited {:?} with {got:?}: {value}",
                    out.status.code()
                );
                assert!(
                    got.is_empty(),
                    "{where_from} shows nothing fitting, {backend} chose {got:?}: {value}"
                );
            }
        }
        eprintln!("{backend} ok: {}", case.shown);
        checked += 1;
    }
    eprintln!("{backend}: {checked} transcripts, {requests} classifications");
    assert_eq!(checked + skipped, transcripts().len(), "{backend}");
}

#[test]
#[ignore]
fn transcripts_still_answer_as_the_pages_show_on_classifier() {
    run_transcripts("classifier");
}

#[test]
#[ignore]
fn transcripts_still_answer_as_the_pages_show_on_typesafe() {
    run_transcripts("typesafe");
}

/// The other half of the check, and it needs no network: every expected item is an item the
/// cited page prints, in the console block that prints the command. An answer edited on the
/// page, a command edited on the page, or a table row edited away from the page fails here.
#[test]
#[ignore]
fn every_expectation_matches_the_page_it_cites() {
    assert_eq!(
        listing(RELEASE_LOG).lines().count(),
        30,
        "the release listing is not the thirty commits the pages print"
    );
    for case in transcripts() {
        for Cite(doc, line) in case.cites {
            let text = std::fs::read_to_string(root().join(doc)).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            let at = lines
                .iter()
                .position(|l| l.trim_start_matches("$ ") == case.shown)
                .unwrap_or_else(|| {
                    panic!(
                        "{doc} (recorded line {line}) no longer prints `{}`",
                        case.shown
                    )
                });
            let start = lines[..at]
                .iter()
                .rposition(|l| l.starts_with("```"))
                .unwrap_or_else(|| panic!("{doc}:{} is in no fenced block", at + 1));
            let end = start
                + 1
                + lines[start + 1..]
                    .iter()
                    .position(|l| l.starts_with("```"))
                    .unwrap_or_else(|| panic!("{doc}:{} has no closing fence", start + 1));
            let block = lines[start..=end].join("\n");
            let want: &[&str] = match (case.on_page, case.expect) {
                ([], Expect::Chose(items)) => items,
                (items, _) => items,
            };
            for item in want {
                assert!(
                    block.contains(item),
                    "{doc}:{} prints `{}` but not the expected `{item}`:\n{block}",
                    at + 1,
                    case.shown
                );
            }
            if at + 1 != *line as usize {
                eprintln!(
                    "{doc}: `{}` moved from line {line} to {}",
                    case.shown,
                    at + 1
                );
            }
        }
    }
}
