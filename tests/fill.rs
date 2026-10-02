mod common;

use common::FakeJev;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::{ffi::OsString, os::unix::ffi::OsStringExt, process::Output};
use wiremock::{MockServer, Request, Respond, ResponseTemplate};

fn run(mut command: assert_cmd::Command, args: &[&str], input: impl AsRef<[u8]>) -> Output {
    command
        .args(args)
        .write_stdin(input.as_ref())
        .output()
        .unwrap()
}

/// `jevify fill --dry-run --json -- <argv>`.
fn dry_run(command: assert_cmd::Command, argv: &[&str], input: impl AsRef<[u8]>) -> Output {
    let fill: &[&str] = &["fill", "--dry-run", "--json", "--"];
    run(command, &[fill, argv].concat(), input)
}

/// `jevify fill -- sh <script> <sentinel> <markers>`, run for real: the output, and whether the
/// command started.
fn exec(mut command: assert_cmd::Command, markers: &[&str], input: &str) -> (Output, bool) {
    let dir = tempfile::tempdir().unwrap().keep();
    let (script, sentinel) = (dir.join("sentinel.sh"), dir.join("sentinel"));
    std::fs::write(&script, "printf ran > \"$1\"\n").unwrap();
    command
        .args(["fill", "--", "sh"])
        .arg(&script)
        .arg(&sentinel);
    let out = run(command, markers, input);
    (out, sentinel.exists())
}

async fn posts(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

fn envelope(out: &Output, exit: i32) -> Value {
    assert_eq!(
        out.status.code(),
        Some(exit),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn fake() -> FakeJev {
    FakeJev {
        choose: |_, _, options| {
            options
                .iter()
                .find(|s| s.as_str() != "NONE")
                .unwrap()
                .clone()
        },
        noul: |_, _| 0.9,
    }
}

/// A TypeSafe backend answering every Noul with `noul` and every Choice with `choice` as the
/// probabilities of L000, L001 and NONE, in the name of `model` (`None`: the field is absent).
fn fixed(model: Option<&'static str>, noul: f64, choice: [f64; 3]) -> impl Respond + 'static {
    move |request: &Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let answers: serde_json::Map<_, _> = body["questions"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(id, q)| {
                let answer = if q["type"] == "noul" {
                    json!({ "noul": noul })
                } else {
                    let [first, second, none] = choice;
                    json!({"choice": "L000", "probabilities": {"L000": first, "L001": second, "NONE": none}})
                };
                (id.clone(), answer)
            })
            .collect();
        let mut value = json!({ "answers": answers });
        if let Some(model) = model {
            value["model"] = model.into();
        }
        ResponseTemplate::new(200).set_body_json(value)
    }
}

async fn server(classifier: bool) -> MockServer {
    if classifier {
        common::mock_classifier(fake()).await
    } else {
        common::mock(fake()).await
    }
}

fn jevify(server: &MockServer, classifier: bool) -> assert_cmd::Command {
    if classifier {
        common::jevify_classifier(server)
    } else {
        common::jevify(server)
    }
}

fn executable(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The command with `dir` first on its PATH, where the fake `git` of a fixture lives.
fn fixture_command(mut command: assert_cmd::Command, dir: &Path) -> assert_cmd::Command {
    command
        .env("FILL_FIXTURE", dir)
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()));
    command
}

/// A fake `git` whose `for-each-ref` lists `count` local branches and which records its calls.
fn branch_fixture(count: usize) -> PathBuf {
    let dir = tempfile::tempdir().unwrap().keep();
    let refs: String = (0..count)
        .map(|i| format!("refs/heads/b{i}\0\x001700000000\0subject {i}\0\n"))
        .collect();
    std::fs::write(dir.join("refs"), refs).unwrap();
    executable(
        &dir.join("git"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$FILL_FIXTURE/calls\"\ncase \"$1\" in\nfor-each-ref) cat \"$FILL_FIXTURE/refs\";;\nlog) printf '\\000rich evidence\\000src/code.rs\\000';;\nesac\n",
    );
    dir
}

/// A fake `git` for the `commit` and `file` kinds: the log holds `count` commits with full
/// 40-hex OIDs, newest first; `ls-files` prints `files`.
const KINDS_GIT: &str = r#"#!/bin/sh
case "$1" in
  rev-list) cat "$FILL_FIXTURE/count" ;;
  ls-files) cat "$FILL_FIXTURE/files" ;;
  log)
    if [ "$2" = "-n" ]; then
      n=$3
      total=$(cat "$FILL_FIXTURE/count")
      [ "$n" -gt "$total" ] && n=$total
      i=0
      while [ "$i" -lt "$n" ]; do
        printf '%040d\000made folder moves atomic %s\000' "$i" "$i"
        i=$((i + 1))
      done
    else
      printf 'body of the commit\000\000src/moves.rs\000'
    fi ;;
  *) exit 1 ;;
esac
"#;

fn kinds_fixture(commits: usize, files: &[u8]) -> PathBuf {
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::write(dir.join("count"), format!("{commits}\n")).unwrap();
    std::fs::write(dir.join("files"), files).unwrap();
    executable(&dir.join("git"), KINDS_GIT);
    dir
}

/// A git work tree with the given files, each holding its own name, all tracked.
fn work_tree(files: &[&str]) -> PathBuf {
    let root = tempfile::tempdir().unwrap().keep();
    for file in files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("VISIBLE {file} BODY\n")).unwrap();
    }
    for args in [vec!["init", "-q"], vec!["add", "-A"]] {
        assert!(
            std::process::Command::new("git")
                .current_dir(&root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    root
}

fn jevify_in(server: &MockServer, dir: &Path) -> assert_cmd::Command {
    let mut command = common::jevify(server);
    command.current_dir(dir);
    command
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_round_trips_raw_argv_and_consumes_stdin() {
    let server = common::mock(fake()).await;
    let literal = OsString::from_vec(b"literal\xff'\nline".to_vec());
    let fill = |mode: &str| {
        let mut cmd = common::jevify(&server);
        cmd.args(["fill", mode, "-0", "--", "sh", "tests/bin/argv.sh", "0"])
            .arg(&literal)
            .arg("@{-:x}");
        run(cmd, &[], b"handle\xfe\0")
    };
    let (dry, real) = (fill("--dry-run"), fill("-q"));
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(real.status.success() && real.stderr.is_empty());
    assert_eq!(real.stdout, b"literal\xff'\nline\0handle\xfe\0stdin:eof\n");
    // The shell reads the printed line back from a script file, as a caller would run it.
    let script = tempfile::tempdir().unwrap().keep().join("replay.sh");
    std::fs::write(&script, &dry.stdout).unwrap();
    let replay = std::process::Command::new("sh")
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(replay.stdout, real.stdout);
    // Machine output cannot represent a non-UTF-8 argv.
    let args = [
        "fill",
        "--json",
        "--dry-run",
        "-0",
        "--",
        "printf",
        "@{-:x}",
    ];
    let out = run(common::jevify(&server), &args, b"handle\xfe\0");
    assert_eq!(envelope(&out, 6)["error"]["kind"], "cannot_run");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_flag_is_kept_or_removed_and_an_unsure_flag_runs_nothing() {
    // (P(yes), exit, argv length after substitution)
    for (noul, exit, len) in [(0.9, 0, 2), (0.1, 0, 1), (0.5, 3, 0)] {
        let server = common::mock(fixed(Some("jev-fake"), noul, [0.0; 3])).await;
        let marker = "@{flag:--draft:uncertain}";
        let (out, ran) = exec(common::jevify(&server), &[marker], "context");
        assert_eq!((out.status.code(), ran), (Some(exit), exit == 0));
        let out = dry_run(common::jevify(&server), &["printf", marker], "context");
        let value = envelope(&out, exit);
        if exit == 0 {
            assert_eq!(value["data"]["argv"].as_array().unwrap().len(), len);
        } else {
            assert_eq!(value["data"]["markers"][0]["reason"], "unsure_flag");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_not_from_jev_run_nothing() {
    let servers = [
        (
            common::mock(fixed(Some("other-model, jev-fake"), 0.9, [0.0; 3])).await,
            false,
        ),
        (common::mock(fixed(None, 0.9, [0.0; 3])).await, false),
        // classifier.dev names a model per dimension: one of the two is another model's.
        (
            common::mock_classifier(fake().with_model(|instructions| {
                if instructions.starts_with("first") {
                    "other-model".into()
                } else {
                    "jev-fake".into()
                }
            }))
            .await,
            true,
        ),
    ];
    let markers = ["@{flag:--first:first}", "@{flag:--second:second}"];
    for (server, classifier) in &servers {
        let (out, ran) = exec(jevify(server, *classifier), &markers, "context");
        assert_eq!(
            out.status.code(),
            Some(4),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stdout.is_empty() && !ran);
    }
    let out = dry_run(
        common::jevify(&servers[1].0),
        &["printf", markers[0]],
        "context",
    );
    assert_eq!(envelope(&out, 4)["error"]["kind"], "api_unavailable");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abstention_reports_every_marker_in_argv_order_with_a_null_error() {
    let server = common::mock(FakeJev {
        noul: |_, _| 0.48,
        ..fake()
    })
    .await;
    let context = tempfile::tempdir().unwrap().keep().join("context");
    std::fs::write(&context, "text").unwrap();
    let mut cmd = common::jevify(&server);
    cmd.args(["fill", "--dry-run", "--json", "--context"])
        .arg(&context);
    let argv = [
        "--",
        "printf",
        "@{-:x}",
        "literal",
        "@{flag:--draft:uncertain}",
    ];
    let value = envelope(&run(cmd, &argv, ""), 3);
    assert!(value["error"].is_null());
    assert_eq!(value["data"]["reason"], "no_match");
    assert_eq!(value["data"]["markers"][0]["arg"], 2);
    assert_eq!(value["data"]["markers"][1]["reason"], "unsure_flag");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_record_never_becomes_an_option_and_newlines_are_omitted() {
    let server = common::mock(fake()).await;
    let args = [
        "fill",
        "--dry-run",
        "--json",
        "-0",
        "--",
        "printf",
        "@{-:x}",
        "--value=@{-:x}",
    ];
    let out = run(
        common::jevify(&server),
        &args,
        b"-option\0safe\0bad\nline\0",
    );
    let value = envelope(&out, 0);
    assert_eq!(
        value["data"]["argv"],
        json!(["printf", "safe", "--value=-option"])
    );
    assert_eq!(value["data"]["markers"][0]["omitted"], 2);
    assert_eq!(value["data"]["markers"][1]["omitted"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn stdin_belongs_to_the_command_unless_a_marker_reads_it() {
    let server = common::mock(fake()).await;
    let dir = branch_fixture(1);
    let file = dir.join("input");
    std::fs::write(&file, "x\n").unwrap();
    let file = file.to_str().unwrap();
    // Markers that read a file or a listing leave stdin to the command.
    let cases: [(&[&str], &str, &[u8]); 3] = [
        (&["--candidates", file], "@{-:x}", b"x\0stdin:data\n"),
        (&["--context", file], "@{one:x|y:x}", b"x\0stdin:data\n"),
        (&[], "@{branch:x}", b"b0\0stdin:data\n"),
    ];
    for (options, marker, stdout) in cases {
        let command: &[&str] = &["--", "sh", "tests/bin/argv.sh", "0", marker];
        let args = [&["fill", "-q"][..], options, command].concat();
        let out = run(
            fixture_command(common::jevify(&server), &dir),
            &args,
            "inherited\n",
        );
        assert_eq!(
            (out.status.code(), out.stdout.as_slice()),
            (Some(0), stdout),
            "{marker}"
        );
    }
    // A stdin marker consumes stdin, and the command's exit code is fill's.
    let args = ["fill", "-q", "--", "sh", "tests/bin/argv.sh", "1", "@{-:x}"];
    let out = run(common::jevify(&server), &args, "x\n");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"x\0stdin:eof\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_errors_and_a_missing_program_fail_before_any_request() {
    let server = common::mock(fake()).await;
    // (option, command, exit, error kind)
    let cases: [(&str, &[&str], i32, &str); 8] = [
        ("--dry-run", &["printf", "@{widget:x}"], 2, "usage"),
        ("--dry-run", &["printf", "{user}@{host:>8}"], 2, "usage"),
        ("--dry-run", &["printf", "@{file:x"], 2, "usage"),
        // Stdin cannot be both the candidates and the context.
        (
            "--dry-run",
            &["printf", "@{-:x}", "@{one:x|y:x}"],
            2,
            "usage",
        ),
        // Machine output describes a dry run only.
        ("-q", &["printf", "@{-:x}"], 2, "usage"),
        // The command is literal, one argument per word.
        (
            "--dry-run",
            &["@{tool:the GitHub command line}", "--version"],
            2,
            "usage",
        ),
        ("--dry-run", &["git switch", "@{file:x}"], 2, "usage"),
        (
            "--dry-run",
            &["jevify-test-missing-program", "@{-:x}"],
            6,
            "cannot_run",
        ),
    ];
    for (option, command, exit, kind) in cases {
        let args = [&["fill", "--json", option, "--"][..], command].concat();
        let out = run(common::jevify(&server), &args, "x\n");
        assert_eq!(envelope(&out, exit)["error"]["kind"], kind, "{command:?}");
    }
    assert!(posts(&server).await.is_empty());
    // The escape passes a marker-like text through.
    let argv = ["printf", "{user}@@{host:>8}", "@{-:x}"];
    let out = dry_run(common::jevify(&server), &argv, "x\n");
    assert_eq!(envelope(&out, 0)["data"]["argv"][1], "{user}@{host:>8}");
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_to_choose_from_abstains_without_a_request() {
    let server = common::mock(fake()).await;
    let dir = branch_fixture(0);
    for (marker, input, reason) in [
        ("@{-:x}", String::new(), "no_match"),
        ("@{branch:x}", String::new(), "no_match"),
        // Context over the text a request may carry.
        ("@{one:x|y:x}", "x".repeat(96_001), "insufficient_evidence"),
    ] {
        let command = fixture_command(common::jevify(&server), &dir);
        let value = envelope(&dry_run(command, &["printf", marker], input), 3);
        assert_eq!(value["data"]["reason"], reason, "{marker}");
        assert!(value["error"].is_null());
    }
    assert!(posts(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn context_questions_go_twenty_to_a_request() {
    for classifier in [false, true] {
        let server = server(classifier).await;
        let mut cmd = jevify(&server, classifier);
        cmd.args(["fill", "--dry-run", "--json", "--", "printf"])
            .args((0..21).map(|i| format!("@{{flag:--flag{i}:condition {i}}}")));
        envelope(&cmd.write_stdin("context").output().unwrap(), 0);
        let key = if classifier {
            "dimensions"
        } else {
            "questions"
        };
        let mut sizes: Vec<_> = posts(&server)
            .await
            .iter()
            .map(|request| request[key].as_object().unwrap().len())
            .collect();
        sizes.sort();
        assert_eq!(sizes, [1, 20]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_marker_option_limit_on_each_backend() {
    for (classifier, limit) in [(false, 200), (true, 99)] {
        let server = server(classifier).await;
        for count in [limit, limit + 1] {
            let options = (0..count)
                .map(|i| format!("option{i}"))
                .collect::<Vec<_>>()
                .join("|");
            let marker = format!("@{{one:{options}:choose}}");
            let before = posts(&server).await.len();
            let out = dry_run(jevify(&server, classifier), &["printf", &marker], "context");
            let value = envelope(&out, if count == limit { 0 } else { 2 });
            if count == limit {
                assert_eq!(value["data"]["argv"][1], "option0");
            } else {
                assert_eq!(posts(&server).await.len(), before);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tie_none_or_a_no_below_threshold_runs_nothing() {
    // (P(any), probabilities of L000, L001 and NONE, reason)
    for (noul, choice, reason) in [
        (0.9, [0.45, 0.45, 0.1], "ambiguous"),
        (0.9, [0.2, 0.1, 0.7], "no_match"),
        (0.1, [0.9, 0.05, 0.05], "no_match"),
    ] {
        let server = common::mock(fixed(Some("jev-fake"), noul, choice)).await;
        let (out, ran) = exec(common::jevify(&server), &["@{-:x}"], "a\nb\n");
        assert_eq!(out.status.code(), Some(3));
        assert!(out.stdout.is_empty() && !ran);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(reason), "{stderr}");
    }
    // Distinct invalid UTF-8 handles have the same lossy evidence.
    let server = common::mock(fake()).await;
    let args = [
        "fill",
        "--dry-run",
        "--json",
        "-0",
        "--",
        "printf",
        "@{-:x}",
    ];
    let out = run(common::jevify(&server), &args, b"\xfe\0\xff\0");
    assert_eq!(envelope(&out, 3)["data"]["reason"], "ambiguous");
}

/// On exit 3, `data.shortlist` lists the nearest candidates of the marker `data.reason` names,
/// in `pick`'s shape (`[{text, p}]`, best first), and is an empty list, never null, when that
/// marker scored nothing. Each marker carries its own under `data.markers[].shortlist`.
#[tokio::test(flavor = "multi_thread")]
async fn an_abstention_lists_the_nearest_candidates_or_an_empty_list() {
    // (P(any), probabilities of L000, L001 and NONE, reason, shortlist)
    for (noul, choice, reason, shortlist) in [
        (
            0.9,
            [0.2, 0.1, 0.7],
            "no_match",
            json!([{"text": "a", "p": 0.2}, {"text": "b", "p": 0.1}]),
        ),
        (
            0.9,
            [0.45, 0.45, 0.1],
            "ambiguous",
            json!([{"text": "a", "p": 0.45}, {"text": "b", "p": 0.45}]),
        ),
        (
            0.1,
            [0.9, 0.05, 0.05],
            "no_match",
            json!([{"text": "a", "p": 0.9}, {"text": "b", "p": 0.05}]),
        ),
    ] {
        let server = common::mock(fixed(Some("jev-fake"), noul, choice)).await;
        let out = dry_run(common::jevify(&server), &["printf", "@{-:x}"], "a\nb\n");
        let value = envelope(&out, 3);
        assert_eq!(value["data"]["reason"], reason);
        assert_eq!(value["data"]["shortlist"], shortlist, "{reason}");
        assert_eq!(value["data"]["markers"][0]["shortlist"], shortlist);
    }
    // The first abstaining marker's list is the top-level one; an unsure flag scored no
    // candidate and lists none.
    let server = common::mock(fixed(Some("jev-fake"), 0.5, [0.2, 0.1, 0.7])).await;
    let context = tempfile::tempdir().unwrap().keep().join("context");
    std::fs::write(&context, "text").unwrap();
    let mut cmd = common::jevify(&server);
    cmd.args(["fill", "--dry-run", "--json", "--context"])
        .arg(&context);
    let argv = ["--", "printf", "@{-:x}", "@{flag:--draft:uncertain}"];
    let value = envelope(&run(cmd, &argv, "a\nb\n"), 3);
    assert_eq!(value["data"]["reason"], "no_match");
    assert_eq!(value["data"]["shortlist"][0]["text"], "a");
    assert_eq!(value["data"]["markers"][1]["reason"], "unsure_flag");
    assert_eq!(value["data"]["markers"][1]["shortlist"], json!([]));
    // Nothing to score: an empty listing, an oversized context, a lone unsure flag.
    let dir = branch_fixture(0);
    for (marker, input) in [
        ("@{-:x}", String::new()),
        ("@{branch:x}", String::new()),
        ("@{one:x|y:x}", "x".repeat(96_001)),
        ("@{flag:--draft:uncertain}", "context".into()),
    ] {
        let command = fixture_command(common::jevify(&server), &dir);
        let value = envelope(&dry_run(command, &["printf", marker], input), 3);
        assert_eq!(value["data"]["shortlist"], json!([]), "{marker}");
        assert_eq!(
            value["data"]["markers"][0]["shortlist"],
            json!([]),
            "{marker}"
        );
    }
    // A resolved command carries no shortlist: candidates are not answers, and there was one.
    let server = common::mock(fake()).await;
    let value = envelope(
        &dry_run(common::jevify(&server), &["printf", "@{-:x}"], "a\n"),
        0,
    );
    assert!(value["data"]["shortlist"].is_null());
    assert!(value["data"]["markers"][0]["shortlist"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lister_that_leaves_its_pipe_open_fails_without_a_request() {
    let server = common::mock(fake()).await;
    let dir = tempfile::tempdir().unwrap().keep();
    executable(&dir.join("git"), "#!/bin/sh\nsleep 2 &\nexit 0\n");
    let command = fixture_command(common::jevify(&server), &dir);
    let out = dry_run(command, &["printf", "@{branch:x}"], "");
    assert_eq!(envelope(&out, 6)["error"]["kind"], "lister_failed");
    assert!(posts(&server).await.is_empty());
}

/// Run with the environment of `configured` and a stdin that stays open with no bytes: any
/// attempted stdin read blocks, and a process that exits within five seconds never read it.
fn run_with_open_stdin(configured: &assert_cmd::Command, args: &[&str]) -> Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_jevify"));
    for (key, value) in configured.get_envs() {
        if let Some(value) = value {
            cmd.env(key, value);
        } else {
            cmd.env_remove(key);
        }
    }
    if let Some(dir) = configured.get_current_dir() {
        cmd.current_dir(dir);
    }
    cmd.args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let _stdin = child.stdin.take().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let exited = loop {
        if child.try_wait().unwrap().is_some() {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let out = child.wait_with_output().unwrap();
    assert!(exited, "the kind check read stdin before validation");
    out
}

const WIDGET_RECIPE: &str = "{\"kind\":\"widget\",\"list\":[\"sh\",\"-c\",\"printf 'w1 alpha\\\\nw2 beta\\\\n'\"],\"field\":1}\n";

fn config_with(lines: &str) -> PathBuf {
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::write(dir.join("kinds.jsonl"), lines).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn recipes_come_from_the_config_directory_and_are_checked_before_any_work() {
    let server = common::mock(fake()).await;
    let dir = branch_fixture(1);
    let valid = config_with(WIDGET_RECIPE);
    let invalid = config_with("{\"kind\":\"widget\"}\n");
    let branch_recipe = WIDGET_RECIPE.replace("\"widget\"", "\"branch\"");
    let shadowing = config_with(&format!("{WIDGET_RECIPE}{branch_recipe}"));
    let fill = |config: &Path| {
        let mut cmd = fixture_command(common::jevify(&server), &dir);
        cmd.env("JEVIFY_CONFIG_DIR", config);
        cmd
    };
    // An unknown kind, a bad recipe file and a recipe named like a built-in kind stop the run
    // while stdin stays open and unread, before any lister runs or any request leaves.
    let cases: [(&Path, [&str; 3], i32, &str); 3] = [
        (
            &valid,
            ["@{widget:x}", "@{-:x}", "{user}@{host:>8}"],
            2,
            "usage",
        ),
        (
            &invalid,
            ["@{branch:x}", "@{-:x}", "@{widget:x}"],
            6,
            "recipe_invalid",
        ),
        (
            &shadowing,
            ["@{branch:x}", "@{-:x}", "@{widget:x}"],
            6,
            "recipe_invalid",
        ),
    ];
    for (config, markers, exit, kind) in cases {
        let args = [
            &["fill", "--dry-run", "--json", "--", "printf"][..],
            &markers,
        ]
        .concat();
        let out = run_with_open_stdin(&fill(config), &args);
        assert_eq!(envelope(&out, exit)["error"]["kind"], kind, "{markers:?}");
    }
    assert!(!dir.join("calls").exists());
    assert!(posts(&server).await.is_empty());
    let out = dry_run(fill(&valid), &["printf", "@{widget:x}"], "");
    assert_eq!(envelope(&out, 0)["data"]["argv"][1], "w1");
    // The unknown-kind suggestion knows the user's recipes.
    let out = dry_run(fill(&valid), &["printf", "@{widgt:x}"], "");
    let message = envelope(&out, 2)["error"]["message"].to_string();
    assert!(message.contains("nearest kind: widget"), "{message}");
    // The built-in kind never reads the recipe file.
    let out = dry_run(fill(&shadowing), &["printf", "@{branch:x}"], "");
    assert_eq!(envelope(&out, 0)["data"]["argv"][1], "b0");
    // A kinds.jsonl in the working directory is never read.
    let out = dry_run(jevify_in(&server, &valid), &["printf", "@{widget:x}"], "");
    assert_eq!(envelope(&out, 2)["error"]["kind"], "usage");
}

#[tokio::test(flavor = "multi_thread")]
async fn status_keeps_handles_and_scope_without_echoing_candidate_evidence() {
    let server = common::mock(fake()).await;
    let dir = tempfile::tempdir().unwrap().keep();
    executable(
        &dir.join("gh"),
        "#!/bin/sh\nprintf '%s\\n' '[{\"number\":4018,\"title\":\"PRIVATE_EVIDENCE\"}]'\n",
    );
    for (args, input) in [
        (
            vec!["fill", "--dry-run", "--", "echo", "@{pr:x}"],
            String::new(),
        ),
        (
            vec!["fill", "--dry-run", "--field", "1", "--", "echo", "@{-:x}"],
            format!("4018 {}\n", "PRIVATE_EVIDENCE".repeat(300)),
        ),
        (vec!["pick", "--from", "pr", "x"], String::new()),
    ] {
        let out = run(fixture_command(common::jevify(&server), &dir), &args, input);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("PRIVATE_EVIDENCE"), "{stderr}");
        if args.contains(&"pr") || args.contains(&"@{pr:x}") {
            assert!(
                stderr.contains("capped")
                    && stderr.contains("--limit N")
                    && stderr.contains("--key number"),
                "{stderr}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_commit_is_a_full_oid_and_a_long_history_keeps_the_newest() {
    let server = common::mock_classifier(fake()).await;
    let capacity = 99 * 33;
    let dir = kinds_fixture(capacity + 1, b"");
    let command = fixture_command(common::jevify_classifier(&server), &dir);
    let argv = ["git", "revert", "@{commit:made folder moves atomic}"];
    let value = envelope(&dry_run(command, &argv, ""), 0);
    let oid = value["data"]["argv"][2].as_str().unwrap();
    assert!(
        oid.len() == 40 && oid.bytes().all(|b| b.is_ascii_hexdigit()),
        "{value}"
    );
    assert_eq!(value["data"]["markers"][0]["candidates"], capacity);
    assert_eq!(value["data"]["markers"][0]["total"], capacity + 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prefix_scopes_the_file_listing_and_the_handle_is_relative_to_it() {
    let server = common::mock(FakeJev {
        choose: |_, state, options| {
            let needle = if state["request"].as_str().unwrap().contains("stages") {
                "add.rs"
            } else {
                "app.toml"
            };
            common::option_containing(state, options, needle)
        },
        noul: |_, _| 0.9,
    })
    .await;
    let root = work_tree(&[
        "src/cmd/add.rs",
        "src/cmd/other.rs",
        "conf/app.toml",
        "README",
    ]);
    let argv = [
        "printf",
        "src/cmd/@{file:stages hunks}",
        "--config=conf/@{file:the app configuration}",
    ];
    let value = envelope(&dry_run(jevify_in(&server, &root), &argv, ""), 0);
    assert_eq!(
        value["data"]["argv"],
        json!(["printf", "src/cmd/add.rs", "--config=conf/app.toml"])
    );
    assert_eq!(value["data"]["markers"][0]["candidates"], 2);
    // A prefix that names no directory fails the lister, before any request.
    let before = posts(&server).await.len();
    let out = dry_run(jevify_in(&server, &root), &["printf", "nope/@{file:x}"], "");
    assert_eq!(envelope(&out, 6)["error"]["kind"], "lister_failed");
    assert_eq!(posts(&server).await.len(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_path_with_a_leading_dash_gets_dot_slash_and_non_utf8_reaches_the_command() {
    let server = common::mock(FakeJev {
        choose: |_, state, options| {
            let needle = if state["request"] == "dash" {
                "-weird"
            } else {
                "caf"
            };
            common::option_containing(state, options, needle)
        },
        noul: |_, _| 0.9,
    })
    .await;
    let dir = kinds_fixture(0, b"-weird\0caf\xe9.txt\0");
    let command = fixture_command(common::jevify(&server), &dir);
    let out = dry_run(command, &["printf", "@{file:dash}"], "");
    assert_eq!(envelope(&out, 0)["data"]["argv"][1], "./-weird");
    let helper = format!("{}/tests/bin/argv.sh", env!("CARGO_MANIFEST_DIR"));
    let args = ["fill", "-q", "--", "sh", &helper, "0", "@{file:the cafe}"];
    let command = fixture_command(common::jevify(&server), &dir);
    let out = run(command, &args, "inherited\n");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"caf\xe9.txt\0stdin:data\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn file_finals_read_excerpts_and_secret_files_never_leave_the_machine() {
    let root = work_tree(&["notes.txt", "other.txt", ".npmrc", ".env"]);
    std::fs::write(root.join(".npmrc"), "TOKEN=1099\n").unwrap();
    std::fs::write(root.join(".env"), "SECRET=zq7mvalue\n").unwrap();
    // Names alone put notes.txt narrowly ahead of .npmrc; the excerpts decide in the finals,
    // where .npmrc competes on its name alone.
    let server = common::mock(fake().with_probabilities(|_, state, options| {
        if options == ["yes", "no"] {
            return vec![0.9, 0.1];
        }
        let items = state["items"].as_array().unwrap();
        let finals = items
            .iter()
            .any(|item| item.as_str().unwrap().contains("VISIBLE"));
        options
            .iter()
            .map(|option| {
                if option == "NONE" {
                    return 0.01;
                }
                let text = items[option[1..].parse::<usize>().unwrap()]
                    .as_str()
                    .unwrap();
                match (finals, text.contains("notes.txt"), text.contains(".npmrc")) {
                    (true, true, _) => 0.9,
                    (true, ..) => 0.02,
                    (false, true, _) => 0.45,
                    (false, _, true) => 0.40,
                    _ => 0.1,
                }
            })
            .collect()
    }))
    .await;
    let out = dry_run(jevify_in(&server, &root), &["printf", "@{file:notes}"], "");
    assert_eq!(envelope(&out, 0)["data"]["argv"][1], "notes.txt");
    let requests = posts(&server).await;
    let finals = requests.last().unwrap().to_string();
    assert!(finals.contains("VISIBLE notes.txt BODY"), "{finals}");
    assert!(finals.contains(".npmrc"), "{finals}");
    for request in &requests {
        let body = request.to_string();
        assert!(
            !body.contains("1099") && !body.contains("zq7mvalue"),
            "{body}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn dir_finals_list_children_and_withhold_secret_dirs() {
    let root = work_tree(&[
        "evals/why/cargo-build.log",
        "evals/why/pytest.log",
        "evals/report.md",
        "src/lib.rs",
        ".secrets/token.txt",
    ]);
    // Round one over directory names alone is a no_match (low `any`): the finals decide.
    let server = common::mock(fake().with_probabilities(|_, state, options| {
        let items = state["items"].as_array().unwrap();
        let finals = items
            .iter()
            .any(|item| item.as_str().unwrap().contains("entries:"));
        if options == ["yes", "no"] {
            return if finals {
                vec![0.9, 0.1]
            } else {
                vec![0.1, 0.9]
            };
        }
        options
            .iter()
            .map(|option| {
                if option == "NONE" {
                    return 0.01;
                }
                let text = items[option[1..].parse::<usize>().unwrap()]
                    .as_str()
                    .unwrap();
                match (
                    finals,
                    text.contains("pytest.log"),
                    text.contains(".secrets"),
                ) {
                    (true, true, _) => 0.9,
                    (true, ..) => 0.02,
                    (false, _, true) => 0.45,
                    (false, ..) => 0.40,
                }
            })
            .collect()
    }))
    .await;
    let argv = ["ls", "@{dir:the failure logs}"];
    let value = envelope(&dry_run(jevify_in(&server, &root), &argv, ""), 0);
    assert_eq!(value["data"]["argv"][1], "evals/why");
    let requests = posts(&server).await;
    assert!(!requests[0].to_string().contains("entries:"));
    let finals = requests.last().unwrap().to_string();
    assert!(
        finals.contains("2 entries: cargo-build.log, pytest.log"),
        "{finals}"
    );
    assert!(
        finals.contains(".secrets") && !finals.contains("token.txt"),
        "{finals}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn too_many_files_are_refused_with_a_hint_before_any_request() {
    let server = common::mock_classifier(fake()).await;
    let files: Vec<u8> = (0..99 * 33 + 1)
        .flat_map(|i| format!("f{i}\0").into_bytes())
        .collect();
    let dir = kinds_fixture(0, &files);
    let command = fixture_command(common::jevify_classifier(&server), &dir);
    let value = envelope(&dry_run(command, &["printf", "@{file:x}"], ""), 6);
    assert_eq!(value["error"]["kind"], "too_many");
    assert!(value["error"]["hint"].is_string());
    assert!(posts(&server).await.is_empty());
}

/// `sh -c 'exit 3'` behind a resolved marker and an abstention both exit 3: the status file
/// tells them apart without stderr parsing, and reports a dry run and an error the same way.
#[tokio::test(flavor = "multi_thread")]
async fn the_status_file_says_whether_the_command_ran() {
    let status = tempfile::tempdir().unwrap().keep().join("status.json");
    let server = common::mock(fake()).await;
    let none = common::mock(FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.01,
    })
    .await;
    let exit3: &[&str] = &["--", "sh", "-c", "exit 3", "@{-:the record}"];
    // (backend, arguments after `fill -q`, exit, the status file's fields, `kind` standing for
    // `error.kind`). `exit_code` is jevify's own: 0 when it handed over to the command.
    let cases: [(&MockServer, &[&str], i32, Value); 5] = [
        (
            &server,
            exit3,
            3,
            json!({"ran": true, "exit_code": 0, "argv": ["sh", "-c", "exit 3", "three"], "reason": null, "kind": null}),
        ),
        (
            &none,
            exit3,
            3,
            json!({"ran": false, "exit_code": 3, "argv": null, "reason": "no_match", "kind": null}),
        ),
        (
            &server,
            &["--dry-run", "--", "true", "@{-:the record}"],
            0,
            json!({"ran": false, "exit_code": 0, "argv": ["true", "three"], "reason": null, "kind": null}),
        ),
        (
            &server,
            &["--", "true", "@{nosuchkind:x}"],
            2,
            json!({"ran": false, "exit_code": 2, "argv": null, "reason": null, "kind": "usage"}),
        ),
        (
            &server,
            &["--", "/nonexistent/program", "@{-:the record}"],
            6,
            json!({"ran": false, "exit_code": 6, "argv": null, "reason": null, "kind": "cannot_run"}),
        ),
    ];
    for (backend, args, exit, expected) in cases {
        let mut cmd = common::jevify(backend);
        cmd.env("JEVIFY_STATUS_FILE", &status);
        let out = run(cmd, &[&["fill", "-q"][..], args].concat(), "three\n");
        assert_eq!(out.status.code(), Some(exit), "{args:?}");
        let mut value: Value = serde_json::from_slice(&std::fs::read(&status).unwrap()).unwrap();
        assert_eq!(value["command"], "fill");
        value["kind"] = value["error"]["kind"].clone();
        for (key, want) in expected.as_object().unwrap() {
            assert_eq!(&value[key], want, "{key} for {args:?}");
        }
    }
}

/// jevify never starts a command it cannot report having started.
#[tokio::test(flavor = "multi_thread")]
async fn an_unwritable_status_file_stops_the_run() {
    let server = common::mock(fake()).await;
    let blocked = tempfile::tempdir()
        .unwrap()
        .keep()
        .join("missing/status.json");
    let mut cmd = common::jevify(&server);
    cmd.env("JEVIFY_STATUS_FILE", &blocked);
    let (out, ran) = exec(cmd, &["@{-:the record}"], "three\n");
    assert_eq!(out.status.code(), Some(6));
    assert!(!ran && !blocked.exists());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("status_file_unwritable"), "{stderr}");
}

/// A real git work tree at `root/rel`: two commits on the default branch, and a branch that
/// exists only as the remote ref `origin/ticket/TPE-791`.
fn real_repo(root: &Path, rel: &str) -> PathBuf {
    let repo = root.join(rel);
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("a"), "a\n").unwrap();
    git(&["add", "a"]);
    git(&["commit", "-qm", "Add the allergy model and its migration"]);
    git(&["update-ref", "refs/remotes/origin/ticket/TPE-791", "HEAD"]);
    std::fs::write(repo.join("b"), "b\n").unwrap();
    git(&["add", "b"]);
    git(&["commit", "-qm", "Borrow in the return type of compute"]);
    repo
}

#[tokio::test(flavor = "multi_thread")]
async fn git_dash_c_in_the_command_is_where_the_listers_run() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| common::option_containing(s, o, "Borrow in the return type"),
        noul: |_, _| 0.9,
    })
    .await;
    let parent = tempfile::tempdir().unwrap().keep().canonicalize().unwrap();
    real_repo(&parent, "work/hyper");
    let marker = "@{commit:names the borrow in the return type}";
    for (args, exit) in [
        (
            vec!["--dry-run", "--", "git", "-C", "work/hyper", "show", marker],
            0,
        ),
        (
            vec!["-C", "work/hyper", "--dry-run", "--", "git", "show", marker],
            0,
        ),
        // From the parent without -C there is no repository: the hint names the one below.
        (vec!["--dry-run", "--", "git", "show", marker], 6),
    ] {
        let mut cmd = jevify_in(&server, &parent);
        cmd.env("GIT_CEILING_DIRECTORIES", &parent).arg("fill");
        let out = run(cmd, &args, "");
        assert_eq!(out.status.code(), Some(exit), "{out:?}");
        let (stdout, stderr) = (
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        if exit == 0 {
            assert!(stdout.contains("'show' '"), "{stdout}");
        } else {
            assert!(stderr.contains("try:"), "{stderr}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_only_branch_is_spelled_the_way_the_command_reads_it() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| common::option_containing(s, o, "TPE-791"),
        noul: |_, _| 0.9,
    })
    .await;
    let root = tempfile::tempdir().unwrap().keep().canonicalize().unwrap();
    let repo = real_repo(&root, "r");
    let marker = "@{branch:the allergy model}";
    for (command, expected) in [
        (
            vec!["git", "log", "-1", marker],
            "'git' 'log' '-1' 'origin/ticket/TPE-791'\n",
        ),
        (
            vec!["git", "--no-pager", "show", marker],
            "'git' '--no-pager' 'show' 'origin/ticket/TPE-791'\n",
        ),
        // switch resolves the short name by its DWIM rule, and refuses the remote ref.
        (
            vec!["git", "switch", marker],
            "'git' 'switch' 'ticket/TPE-791'\n",
        ),
        // A literal prefix already says which spelling the caller wants.
        (
            vec!["git", "log", "origin/@{branch:the allergy model}"],
            "'git' 'log' 'origin/ticket/TPE-791'\n",
        ),
    ] {
        let mut cmd = jevify_in(&server, &repo);
        cmd.args(["fill", "--dry-run", "--"]);
        let out = run(cmd, &command, "");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            expected,
            "{command:?} {out:?}"
        );
    }
    // Run for real: git resolves the remote ref.
    let args = ["fill", "--", "git", "log", "-1", "--oneline", marker];
    let out = run(jevify_in(&server, &repo), &args, "");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Add the allergy model"),
        "{out:?}"
    );
}
