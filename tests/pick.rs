mod common;

use common::{FakeJev, option_containing};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;
use wiremock::MockServer;

fn envelope(out: &Output, exit: i32) -> Value {
    assert_eq!(
        out.status.code(),
        Some(exit),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A backend whose every answer, Noul included, is the vector `probabilities` returns.
fn ranked(probabilities: common::ProbabilityVector) -> common::ConfiguredFake {
    FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.9,
    }
    .with_probabilities(probabilities)
}

fn executable(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake `git` that lists `count` branches and prints tier-two evidence for `log`.
fn fake_branches(count: usize) -> PathBuf {
    let root = tempfile::tempdir().unwrap().keep();
    executable(
        &root.join("git"),
        &format!(
            r#"#!/bin/sh
case "$1" in
  for-each-ref)
    i=0
    while [ "$i" -lt {count} ]; do
      printf 'refs/heads/branch-%s\000\0001700000000\000subject\000\n' "$i"
      i=$((i + 1))
    done ;;
  log) printf '\000richer evidence\000\000src/file\000' ;;
  *) exit 1 ;;
esac
"#
        ),
    );
    root
}

/// The binary with `root` as its whole PATH.
fn branch_command(server: &MockServer, root: &Path) -> assert_cmd::Command {
    let mut cmd = common::jevify(server);
    cmd.env("PATH", root);
    cmd
}

/// A real git work tree in a fresh directory, with two commits: `README` then `src/slab.go`.
fn git_repo() -> PathBuf {
    let root = tempfile::tempdir().unwrap().keep().canonicalize().unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .current_dir(&root)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(root.join("README"), "readme\n").unwrap();
    git(&["add", "README"]);
    git(&["commit", "-qm", "Add the readme"]);
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/slab.go"),
        "type Slab struct { I16 []int16 }\n",
    )
    .unwrap();
    git(&["add", "src/slab.go"]);
    git(&["commit", "-qm", "Add pre-allocated integer buffers"]);
    root
}

async fn bodies(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn from_branch_prints_the_handle_and_never_reads_stdin() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "branch-3"),
        noul: |_, _| 0.9,
    })
    .await;
    let root = fake_branches(5);
    let pick = |json: bool| {
        let mut cmd = branch_command(&server, &root);
        if json {
            cmd.arg("--json");
        }
        cmd.args(["pick", "--from", "branch", "x"])
            .write_stdin("STDIN_SENTINEL")
            .output()
            .unwrap()
    };
    let out = pick(false);
    assert_eq!(
        (out.status.code(), out.stdout.as_slice()),
        (Some(0), &b"branch-3\n"[..])
    );
    assert_eq!(
        envelope(&pick(true), 0)["data"]["matches"][0]["text"],
        "branch-3"
    );
    for body in bodies(&server).await {
        assert!(!body.contains("STDIN_SENTINEL"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn from_branch_breaks_a_tie_with_richer_evidence_or_abstains() {
    // Two names tie; the evidence of the second round decides "resolve" and leaves "tie" a tie.
    let server = common::mock(ranked(|_, state, options| {
        if options == ["yes", "no"] {
            return vec![0.9, 0.1];
        }
        let wins = state["request"] == "resolve" && state.to_string().contains("richer evidence");
        options
            .iter()
            .map(|o| match o.as_str() {
                "L000" if wins => 0.9,
                _ if wins => 0.1 / (options.len() - 1) as f64,
                "L000" | "L001" => 0.45,
                _ => 0.025,
            })
            .collect()
    }))
    .await;
    let root = fake_branches(5);
    for (request, exit) in [("resolve", 0), ("tie", 3)] {
        let out = branch_command(&server, &root)
            .args(["--json", "pick", "--from", "branch", request])
            .output()
            .unwrap();
        let value = envelope(&out, exit);
        if exit == 3 {
            assert_eq!(value["data"]["reason"], "ambiguous");
            assert_eq!(value["data"]["matches"], json!([]));
        }
    }
    // An empty listing, and a listing where nothing fits, are a no_match.
    let server = common::mock(FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.1,
    })
    .await;
    for count in [0, 5] {
        let out = branch_command(&server, &fake_branches(count))
            .args(["--json", "pick", "--from", "branch", "x"])
            .output()
            .unwrap();
        assert_eq!(envelope(&out, 3)["data"]["reason"], "no_match");
    }
}

#[test]
fn from_kind_validation_and_lister_failure() {
    let root = tempfile::tempdir().unwrap().keep();
    for (kind, exit, error) in [
        ("branc", 2, "usage"),
        // `--from -` names stdin, the default source: here it is empty.
        ("-", 6, "empty_input"),
        ("branch", 6, "lister_failed"),
    ] {
        let out = common::bin()
            .current_dir(&root)
            .env("GIT_CEILING_DIRECTORIES", &root)
            .args(["--json", "pick", "--from", kind, "x"])
            .write_stdin("")
            .output()
            .unwrap();
        let value = envelope(&out, exit);
        assert_eq!(value["error"]["kind"], error, "{kind}");
        if kind == "branc" {
            let message = value["error"]["message"].as_str().unwrap();
            assert!(message.contains(r#"did you mean "branch""#), "{message}");
        }
    }
    for flag in ["--files", "--index", "-0", "--para"] {
        common::bin()
            .args(["pick", "--from", "branch", flag, "x"])
            .assert()
            .code(2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn from_branch_prints_the_top_handles_and_keeps_raw_bytes() {
    let server = common::mock(ranked(|_, _, options| {
        options
            .iter()
            .map(|o| match o.as_str() {
                "yes" => 0.9,
                "L000" => 0.7,
                "L001" => 0.2,
                _ => 0.1,
            })
            .collect()
    }))
    .await;
    let root = fake_branches(2);
    let pick = |args: &[&str]| branch_command(&server, &root).args(args).output().unwrap();
    let out = pick(&["pick", "--from", "branch", "-n", "2", "x"]);
    assert_eq!(
        (out.status.code(), out.stdout.as_slice()),
        (Some(0), &b"branch-0\nbranch-1\n"[..])
    );
    std::fs::write(
        root.join("git"),
        "#!/bin/sh\nprintf 'refs/heads/raw-\\377\\000\\0001700000000\\000subject\\000\\n'\n",
    )
    .unwrap();
    let out = pick(&["pick", "--from", "branch", "x"]);
    assert_eq!(
        (out.status.code(), out.stdout.as_slice()),
        (Some(0), &b"raw-\xff\n"[..])
    );
    let value = envelope(&pick(&["--json", "pick", "--from", "branch", "x"]), 0);
    let matched = &value["data"]["matches"][0];
    assert_eq!(
        (&matched["text"], &matched["lossy"], &matched["ordinal"]),
        (&json!("raw-\u{fffd}"), &json!(true), &json!(1))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn from_branch_remote_twin_uses_local_handle() {
    let root = tempfile::tempdir().unwrap().keep();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "--no-gpg-sign",
            "-qm",
            "subject",
        ],
        vec!["branch", "release"],
        vec!["update-ref", "refs/remotes/origin/release", "HEAD"],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(&root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "release"),
        noul: |_, _| 0.9,
    })
    .await;
    let out = common::jevify(&server)
        .current_dir(root)
        .args(["pick", "--from", "branch", "release"])
        .output()
        .unwrap();
    assert_eq!(
        (out.status.code(), out.stdout.as_slice()),
        (Some(0), &b"release\n"[..])
    );
    for body in bodies(&server).await {
        assert!(!body.contains("origin/release"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn too_many_records_are_refused_with_a_hint_before_any_request() {
    let server = common::mock_classifier(FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.9,
    })
    .await;
    let input = (0..9802).map(|i| format!("item {i}\n")).collect::<String>();
    let out = common::jevify_classifier(&server)
        .args(["--json", "pick", "item"])
        .write_stdin(input)
        .output()
        .unwrap();
    let value = envelope(&out, 6);
    assert_eq!(value["error"]["kind"], "too_many");
    assert!(value["error"]["hint"].is_string());
    assert!(bodies(&server).await.is_empty());
}

#[tokio::test]
async fn malformed_choice_labels_return_a_protocol_envelope() {
    use wiremock::matchers::method;
    use wiremock::{Mock, ResponseTemplate};
    for label in ["", "é", "X000", "L999"] {
        let server = MockServer::start().await;
        let answers = json!({"any": {"noul": 0.9}, "pick": {"choice": label, "probabilities": {label: 0.9, "NONE": 0.1}}});
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "answers": answers })))
            .mount(&server)
            .await;
        let output = tokio::task::spawn_blocking(move || {
            common::jevify(&server)
                .args(["--json", "pick", "match"])
                .write_stdin("item\n")
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(envelope(&output, 4)["error"]["kind"], "api_protocol");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn selected_records_keep_their_bytes_and_terminators() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "invoice"),
        noul: |_, _| 0.9,
    })
    .await;
    let cases: [(&[&str], &[u8], &[u8]); 7] = [
        (
            &[],
            b"other\n\x1b[31minvoice\x1b[0m\n",
            b"\x1b[31minvoice\x1b[0m\n",
        ),
        (&[], b"other\r\ninvoice\r\n", b"invoice\r\n"),
        (&[], b"other\ninvoice\xff\n", b"invoice\xff\n"),
        (&[], b"other\ninvoice", b"invoice"),
        (&["-0"], b"other\0invoice\0", b"invoice\0"),
        (
            &["--para"],
            b"other\n\ninvoice\ncontinued\n\n",
            b"invoice\ncontinued\n\n",
        ),
        // -n prints only the records that beat NONE.
        (&["-n", "3"], b"notes\ninvoice\nphoto\n", b"invoice\n"),
    ];
    for (options, input, expected) in cases {
        let out = common::jevify(&server)
            .args(["pick", "bill"])
            .args(options)
            .write_stdin(input)
            .output()
            .unwrap();
        assert_eq!(
            (out.status.code(), out.stdout.as_slice()),
            (Some(0), expected),
            "{options:?}"
        );
    }
    let out = common::jevify(&server)
        .args(["--json", "pick", "bill"])
        .write_stdin(&b"other\ninvoice\xff\n"[..])
        .output()
        .unwrap();
    let value = envelope(&out, 0);
    let matched = &value["data"]["matches"][0];
    assert_eq!(
        (&matched["text"], &matched["lossy"], &matched["ordinal"]),
        (&json!("invoice\u{fffd}\n"), &json!(true), &json!(2))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn three_best_index_blank_limit_and_no_saved_input() {
    let server = common::mock(ranked(|_, _, options| {
        options
            .iter()
            .map(|o| match o.as_str() {
                "L000" => 0.65,
                "L001" => 0.2,
                "L002" => 0.1,
                "yes" => 0.9,
                _ => 0.05,
            })
            .collect()
    }))
    .await;
    let root = tempfile::tempdir().unwrap().keep();
    for (args, input, code, expected) in [
        (
            vec!["pick", "-n", "3", "x"],
            "a\nb\nc\n".to_string(),
            0,
            "a\nb\nc\n",
        ),
        (
            vec!["pick", "--index", "x"],
            "\na\nb\nc\n".to_string(),
            0,
            "2\n",
        ),
        (vec!["pick", "x"], " \n\t\n".to_string(), 6, ""),
        (
            vec!["pick", "x"],
            (0..20_001).map(|i| format!("{i}\n")).collect(),
            6,
            "",
        ),
    ] {
        let out = common::jevify(&server)
            .env("JEVIFY_CACHE_DIR", &root)
            .args(args)
            .write_stdin(input)
            .output()
            .unwrap();
        assert_eq!(
            (out.status.code(), out.stdout.as_slice()),
            (Some(code), expected.as_bytes())
        );
    }
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn file_names_go_first_and_excerpts_skip_secret_and_unreadable_files() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "invoice"),
        noul: |_, _| 0.9,
    })
    .await;
    let root = tempfile::tempdir().unwrap().keep();
    std::fs::write(root.join("invoice.txt"), "VISIBLE_FILE_BODY").unwrap();
    std::fs::write(root.join(".npmrc"), "TOKEN=1099").unwrap();
    std::fs::create_dir(root.join("bills")).unwrap();
    let out = common::jevify(&server)
        .current_dir(root)
        .args(["pick", "-0", "--files", "bill"])
        .write_stdin(&b"./invoice.txt\0.npmrc\0bills\0"[..])
        .output()
        .unwrap();
    assert_eq!(
        (out.status.code(), out.stdout.as_slice()),
        (Some(0), &b"./invoice.txt\0"[..]),
        "{out:?}"
    );
    let bodies = bodies(&server).await;
    let (names, finals) = (&bodies[0], bodies.last().unwrap());
    assert!(names.contains(".npmrc") && !names.contains("VISIBLE_FILE_BODY"));
    assert!(finals.contains(".npmrc") && finals.contains("VISIBLE_FILE_BODY"));
    assert!(bodies.iter().all(|body| !body.contains("TOKEN=1099")));
}

// Blank lines take no window slot and a repeated line is sent once, but `line` still counts
// original stdin lines.
#[tokio::test(flavor = "multi_thread")]
async fn blank_and_duplicate_lines_are_skipped_but_line_numbers_are_original() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| {
            assert_eq!(s["items"].as_array().unwrap().len(), 2, "duplicate sent");
            option_containing(s, o, "invoice")
        },
        noul: |_, _| 0.9,
    })
    .await;
    let out = common::jevify(&server)
        .args(["--json", "pick", "--index", "the bill"])
        .write_stdin("notes.txt\n\ninvoice-march.pdf\nnotes.txt\n")
        .output()
        .unwrap();
    let value = envelope(&out, 0);
    assert_eq!(value["data"]["matches"][0]["line"], 3, "{value}");
    assert_eq!(value["data"]["matches"][0]["text"], "invoice-march.pdf");
}

#[tokio::test(flavor = "multi_thread")]
async fn files_without_stdin_rank_the_working_directory() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "slab.go"),
        noul: |_, _| 0.9,
    })
    .await;
    let repo = git_repo();
    // Untracked files are listed too, ignored ones are not; a secret file lends no excerpt.
    std::fs::write(repo.join(".gitignore"), "build/\n").unwrap();
    std::fs::create_dir(repo.join("build")).unwrap();
    std::fs::write(repo.join("build/slab.go"), "generated\n").unwrap();
    std::fs::write(repo.join(".env"), "SECRET=zq7mvalue\n").unwrap();
    for input in ["", "\n  \n"] {
        let out = common::jevify(&server)
            .current_dir(&repo)
            .args(["pick", "--files", "defines", "pre-allocated", "buffers"])
            .write_stdin(input)
            .output()
            .unwrap();
        assert_eq!(
            (out.status.code(), out.stdout.as_slice()),
            (Some(0), &b"src/slab.go\n"[..]),
            "{out:?}"
        );
    }
    let bodies = bodies(&server).await;
    assert!(bodies[0].contains("README") && !bodies[0].contains("build/"));
    assert!(bodies.iter().all(|body| !body.contains("zq7mvalue")));
    // --index has no meaning for paths; an empty directory has nothing to rank.
    let empty = tempfile::tempdir().unwrap().keep();
    for (args, exit, kind) in [
        (["--index", "x"], 2, "usage"),
        (["x", "y"], 6, "empty_input"),
    ] {
        let out = common::jevify(&server)
            .current_dir(&empty)
            .env("GIT_CEILING_DIRECTORIES", &empty)
            .args(["--json", "pick", "--files"])
            .args(args)
            .write_stdin("")
            .output()
            .unwrap();
        assert_eq!(envelope(&out, exit)["error"]["kind"], kind);
    }
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        bodies.len()
    );
    // Without --files, empty stdin is an input error.
    let out = common::bin()
        .args(["pick", "the", "fix"])
        .write_stdin("")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(6));
}

#[tokio::test(flavor = "multi_thread")]
async fn from_file_tool_and_pr_print_a_handle() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| {
            let needle = match s["request"].as_str().unwrap() {
                "the invoice" => "invoice",
                "the beta tool" => "beta-tool",
                _ => "Windows",
            };
            option_containing(s, o, needle)
        },
        noul: |_, _| 0.9,
    })
    .await;
    // The fixture directory is the whole PATH: its scripts use shell builtins only.
    let root = tempfile::tempdir().unwrap().keep();
    executable(
        &root.join("git"),
        "#!/bin/sh\n[ \"$1\" = ls-files ] || exit 1\nprintf '%s\\000' invoice.txt notes.txt\n",
    );
    executable(
        &root.join("gh"),
        "#!/bin/sh\nprintf '[{\"number\":7,\"title\":\"Windows path fix\",\"state\":\"OPEN\",\"headRefName\":\"x\"}]\\n'\n",
    );
    for name in ["alpha-tool", "beta-tool"] {
        executable(&root.join(name), "#!/bin/sh\n");
    }
    for (kind, request, handle) in [
        ("file", "the invoice", "invoice.txt\n"),
        ("tool", "the beta tool", "beta-tool\n"),
        ("pr", "the Windows path fix", "7\n"),
    ] {
        let out = branch_command(&server, &root)
            .args(["pick", "--from", kind, request])
            .output()
            .unwrap();
        assert_eq!(
            (out.status.code(), out.stdout.as_slice()),
            (Some(0), handle.as_bytes()),
            "{kind}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn from_kind_reports_a_bad_recipe_file() {
    let config = tempfile::tempdir().unwrap().keep();
    std::fs::write(
        config.join("kinds.jsonl"),
        "{\"kind\":\"widget\",\"list\":[\"printf\",\"w1\\\\n\"]}\n{\"kind\":\"gadget\"}\n",
    )
    .unwrap();
    let out = common::bin()
        .env("JEVIFY_CONFIG_DIR", &config)
        .args(["--json", "pick", "--from", "widget", "x"])
        .output()
        .unwrap();
    assert_eq!(envelope(&out, 6)["error"]["kind"], "recipe_invalid");
}

#[tokio::test(flavor = "multi_thread")]
async fn unquoted_words_are_one_intent_and_a_leading_dash_is_stdin() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "fix-319"),
        noul: |_, _| 0.9,
    })
    .await;
    for args in [
        vec!["pick", "stops", "meaningless", "figures"],
        vec!["pick", "-", "stops", "meaningless", "figures"],
        vec!["pick", "stops meaningless figures"],
        vec!["pick", "stops", "meaningless", "figures", "-n", "1"],
        vec!["pick", "-", "stops meaningless figures"],
    ] {
        let out = common::jevify(&server)
            .args(&args)
            .write_stdin("main\nfix-319\n")
            .output()
            .unwrap();
        assert_eq!(
            (out.status.code(), out.stdout.as_slice()),
            (Some(0), &b"fix-319\n"[..]),
            "{args:?}"
        );
    }
    for body in bodies(&server).await {
        let request: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(request["state"]["request"], "stops meaningless figures");
    }
    // `-` alone describes nothing.
    let out = common::bin()
        .args(["--json", "pick", "-"])
        .write_stdin("a\n")
        .output()
        .unwrap();
    let value = envelope(&out, 2);
    assert_eq!(value["error"]["kind"], "usage");
    assert!(value["error"]["example"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abstention_prints_nothing_and_names_the_nearest_as_not_chosen() {
    // "a spaceship": the Noul says no. "a rocket": the Noul says yes, and NONE wins the Choice.
    let server = common::mock(ranked(|_, state, options| {
        if options == ["yes", "no"] {
            return if state["request"] == "a rocket" {
                vec![0.9, 0.1]
            } else {
                vec![0.1, 0.9]
            };
        }
        options
            .iter()
            .map(|o| match o.as_str() {
                "L000" => 0.27,
                "L001" => 0.03,
                _ => 0.7,
            })
            .collect()
    }))
    .await;
    let out = common::jevify(&server)
        .args(["pick", "a", "rocket"])
        .write_stdin("alpha\nbeta\n")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    let out = common::jevify(&server)
        .args(["--json", "pick", "a", "spaceship"])
        .write_stdin("alpha\nbeta\n")
        .output()
        .unwrap();
    let value = envelope(&out, 3);
    assert_eq!(value["data"]["matches"], json!([]));
    assert_eq!(value["data"]["shortlist"][0]["text"], "alpha");
    assert_eq!(value["data"]["shortlist"][0]["p"], 0.27);
    assert!(value["data"]["hint"].is_string());
}

#[test]
fn from_outside_a_repository_the_hint_names_the_one_below() {
    let parent = tempfile::tempdir().unwrap().keep().canonicalize().unwrap();
    std::fs::create_dir(parent.join("work")).unwrap();
    std::fs::rename(git_repo(), parent.join("work/hyper")).unwrap();
    let out = common::bin()
        .current_dir(&parent)
        .env("GIT_CEILING_DIRECTORIES", &parent)
        .args(["pick", "--from", "commit", "names", "the", "borrow"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(6));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("try:"), "{stderr}");
    // A -C that names no directory is an input error.
    let out = common::bin()
        .current_dir(&parent)
        .args(["--json", "pick", "-C", "nowhere", "--from", "commit", "x"])
        .output()
        .unwrap();
    let value = envelope(&out, 6);
    assert_eq!(value["error"]["kind"], "input");
    assert!(value["error"]["example"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn stdin_near_tie_abstains_without_printing_a_record() {
    let server = common::mock(ranked(|_, _, options| {
        options
            .iter()
            .map(|o| match o.as_str() {
                "yes" => 0.9,
                "L000" => 0.5,
                "L001" => 0.4,
                _ => 0.1,
            })
            .collect()
    }))
    .await;
    for json in [false, true] {
        let mut cmd = common::jevify(&server);
        if json {
            cmd.arg("--json");
        }
        let out = cmd
            .args(["pick", "-n", "2", "match"])
            .write_stdin("alpha\nbeta\n")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(3));
        if json {
            let value = envelope(&out, 3);
            assert_eq!(value["data"]["reason"], "ambiguous");
            assert_eq!(value["data"]["matches"], json!([]));
        } else {
            assert!(out.stdout.is_empty());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn commit_patch_can_overrule_a_decisive_subject_and_recover_a_zero_score() {
    let repo = git_repo();
    let gold = std::process::Command::new("git")
        .current_dir(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap()
        .stdout;
    let server = common::mock(ranked(|_, state, options| {
        if options == ["yes", "no"] {
            return vec![0.9, 0.1];
        }
        let patch = state.to_string().contains("+type Slab");
        let winner = option_containing(state, options, if patch { "+type Slab" } else { "readme" });
        options
            .iter()
            .map(|o| if *o == winner { 1.0 } else { 0.0 })
            .collect()
    }))
    .await;
    let out = common::jevify(&server)
        .current_dir(&repo)
        .args(["pick", "--from", "commit", "integer buffers"])
        .output()
        .unwrap();
    assert_eq!((out.status.code(), out.stdout), (Some(0), gold));
}

#[tokio::test(flavor = "multi_thread")]
async fn dash_c_lists_the_named_repository_and_prints_a_full_oid() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "integer buffers"),
        noul: |_, _| 0.9,
    })
    .await;
    let parent = tempfile::tempdir().unwrap().keep().canonicalize().unwrap();
    let repo = git_repo();
    let dir = repo.to_str().unwrap();
    for args in [
        vec!["pick", "-C", dir, "--from", "commit", "the", "buffers"],
        // A verb option written before the verb is read as the verb's.
        vec!["--from", "commit", "pick", "--repo", dir, "the buffers"],
    ] {
        let out = common::jevify(&server)
            .current_dir(&parent)
            .env("GIT_CEILING_DIRECTORIES", &parent)
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let oid = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        assert!(
            oid.len() == 40 && oid.bytes().all(|b| b.is_ascii_hexdigit()),
            "{oid}"
        );
    }
}
