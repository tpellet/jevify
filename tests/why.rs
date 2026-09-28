mod common;
use common::{FakeJev, option_containing};

#[tokio::test(flavor = "multi_thread")]
async fn ci_evidence_is_clean_and_each_job_keeps_its_original_cause() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| {
            let evidence = s.to_string();
            assert!(!evidence.contains("PRIVATE_JOB"));
            assert!(!evidence.contains("PRIVATE_STEP"));
            assert!(!evidence.contains("2026-09-28"));
            assert!(!evidence.contains("\\u001b"));
            assert!(!evidence.contains("^[["));
            option_containing(s, o, "error: cause")
        },
        noul: |_, _| 0.95,
    })
    .await;
    // Interleaved jobs with identical diagnostics must not deduplicate each other.
    let a = "PRIVATE_JOB_A\tPRIVATE_STEP\t2026-09-28T10:00:00Z \x1b[31merror: cause\x1b[0m  ";
    let b = "PRIVATE_JOB_B\tPRIVATE_STEP\t2026-09-28T10:00:00Z ^[[31merror: cause^[[0m  ";
    let log = format!(
        "{a}\n{b}\nPRIVATE_JOB_A\tPRIVATE_STEP\t\tcontinued detail\nPRIVATE_JOB_A\tPRIVATE_STEP\t2026-09-28T10:00:01Z Process completed with exit code 1\n"
    );
    for json in [true, false] {
        let mut cmd = common::jevify(&server);
        cmd.args(["why", "--no-save", "-C", "0"]);
        if json {
            cmd.arg("--json");
        }
        let input = log.clone();
        let out = tokio::task::spawn_blocking(move || cmd.write_stdin(input).output().unwrap())
            .await
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if json {
            let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            let causes = v["data"]["causes"].as_array().unwrap();
            assert_eq!(causes.len(), 2);
            assert_eq!(causes[0]["line"], 1);
            assert_eq!(causes[0]["text"], a);
            assert_eq!(causes[1]["line"], 2);
            assert_eq!(causes[1]["text"], b);
        } else {
            let stdout = String::from_utf8(out.stdout).unwrap();
            assert!(stdout.contains(a) && stdout.contains(b));
            assert!(stdout.find(a) < stdout.find(b));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn summary_includes_the_first_shortlisted_diagnostic_block() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "Found 2 not formatted files"),
        noul: |_, _| 0.95,
    })
    .await;
    let mut cmd = common::jevify(&server);
    let out = tokio::task::spawn_blocking(move || {
        cmd.args(["--json", "why", "--no-save", "-C", "0"])
            .write_stdin(
                "from src/main.ts:\n  1|+ formatted text\n\nFound 2 not formatted files.\n",
            )
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["causes"][0]["line"], 4);
    let context = v["data"]["causes"][0]["context"].as_array().unwrap();
    assert_eq!(
        context
            .iter()
            .map(|l| l["line"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 4]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn late_tap_failure_survives_a_large_log_of_signals() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| {
            // The nearest-failure context repeats the TAP text on other candidates.
            // Choose the candidate itself, in both rounds, rather than its context.
            let mut candidates = s.clone();
            for item in candidates["items"].as_array_mut().unwrap() {
                *item = item.as_str().unwrap().lines().next().unwrap().into();
            }
            option_containing(&candidates, o, "not ok 43 - test/version-from-tgz.js")
        },
        noul: |_, _| 0.95,
    })
    .await;
    let mut lines: Vec<_> = (0..13_000)
        .map(|i| format!("warning: expected error {i}"))
        .collect();
    lines[7859] = "not ok 43 - test/version-from-tgz.js".into();
    let mut cmd = common::jevify(&server);
    let out = tokio::task::spawn_blocking(move || {
        cmd.args(["--json", "why", "--no-save"])
            .write_stdin(lines.join("\n"))
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["causes"][0]["line"], 7860);
}

#[tokio::test(flavor = "multi_thread")]
async fn gate_keeps_failure_evidence_and_exposes_a_shortlist_on_abstention() {
    // Either round can establish a failure; NONE winning or both Nouls low abstains.
    for mode in [
        "tie",
        "bom_tie",
        "close_none",
        "none",
        "low_finals",
        "low_any",
    ] {
        let probabilities: common::ProbabilityVector = match mode {
            "tie" | "bom_tie" => |_, _, opts| {
                opts.iter()
                    .map(|o| match o.as_str() {
                        "yes" => 0.95,
                        "L000" | "L001" => 0.4,
                        "L002" => 0.15,
                        _ => 0.05,
                    })
                    .collect()
            },
            "close_none" => |_, _, opts| {
                opts.iter()
                    .map(|o| match o.as_str() {
                        "yes" => 0.95,
                        "no" => 0.05,
                        "L000" => 0.5,
                        "NONE" => 0.3,
                        _ => 0.1,
                    })
                    .collect()
            },
            "none" => |_, _, opts| {
                opts.iter()
                    .map(|o| match o.as_str() {
                        "yes" => 0.95,
                        "no" => 0.05,
                        "L000" => 0.3,
                        "NONE" => 0.5,
                        _ => 0.1,
                    })
                    .collect()
            },
            "low_finals" => |_, s, opts| {
                if opts == ["yes", "no"] {
                    return if s.to_string().contains("context only") {
                        vec![0.1, 0.9]
                    } else {
                        vec![0.95, 0.05]
                    };
                }
                opts.iter()
                    .map(|o| if o == "L000" { 0.85 } else { 0.05 })
                    .collect()
            },
            _ => |_, _, opts| {
                if opts == ["yes", "no"] {
                    return vec![0.1, 0.9];
                }
                opts.iter()
                    .map(|o| if o == "L000" { 0.85 } else { 0.05 })
                    .collect()
            },
        };
        let server = common::mock(
            FakeJev {
                choose: |_, _, _| "L000".into(),
                noul: |_, _| 0.95,
            }
            .with_probabilities(probabilities),
        )
        .await;
        let mut cmd = common::jevify(&server);
        let log = if mode == "bom_tie" {
            "job\tstep\t\u{feff}2026-09-28T10:00:00Z error: alpha\njob\tstep\t2026-09-28T10:00:01Z error: beta\njob\tstep\t2026-09-28T10:00:02Z error: gamma\n"
        } else {
            "error: alpha\nerror: beta\nerror: gamma\n"
        };
        let out = tokio::task::spawn_blocking(move || {
            cmd.args(["--json", "why", "--no-save", "-n", "3"])
                .write_stdin(log)
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        let accepted = matches!(mode, "tie" | "bom_tie" | "close_none" | "low_finals");
        assert_eq!(
            out.status.code(),
            Some(if accepted { 0 } else { 3 }),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        if accepted {
            assert_eq!(v["data"]["causes"][0]["line"], 1);
            if mode == "bom_tie" {
                assert_eq!(v["data"]["causes"].as_array().unwrap().len(), 3);
                assert_eq!(v["data"]["causes"][0]["text"], log.lines().next().unwrap());
            }
            continue;
        }
        assert!(v["data"]["causes"].as_array().unwrap().is_empty());
        let shortlist = v["data"]["shortlist"].as_array().unwrap();
        assert_eq!(shortlist.len(), 3);
        assert_eq!(shortlist[0]["line"], 1);
        assert_eq!(shortlist[0]["text"], "error: alpha");
        assert!(shortlist.iter().all(|c| c["p"].is_number()));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("jevify why: nearest (not chosen):"));
        for c in shortlist {
            assert!(stderr.contains(&format!(
                "line {}: {}",
                c["line"],
                c["text"].as_str().unwrap()
            )));
        }
    }
}

/// The cause is the line Jev picks, with `-C` lines of context around it; the human output
/// numbers the lines and marks the cause.
#[tokio::test(flavor = "multi_thread")]
async fn points_at_the_root_cause_with_context() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "E0432"),
        noul: |_, _| 0.93,
    })
    .await;
    let log = "   Compiling foo v0.1.0\nerror[E0432]: unresolved import `bar`\n --> src/main.rs:1:5\nerror: could not compile `foo`\n";
    let mut c = common::jevify(&server);
    let out = tokio::task::spawn_blocking(move || {
        c.args(["--json", "why", "-C", "1"])
            .write_stdin(log)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["causes"][0]["line"], 2);
    assert_eq!(
        v["data"]["causes"][0]["text"],
        "error[E0432]: unresolved import `bar`"
    );
    assert_eq!(
        v["data"]["causes"][0]["context"].as_array().unwrap().len(),
        3
    );
    let mut c = common::jevify(&server);
    let out = tokio::task::spawn_blocking(move || {
        c.args(["why", "--no-save"])
            .write_stdin(log)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains(">     2 │ error[E0432]"));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_signal_on_stdin_is_exit_3_with_a_hint() {
    let server = common::mock(FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.05,
    })
    .await;
    let mut c = common::jevify(&server);
    let out = tokio::task::spawn_blocking(move || {
        c.args(["--json", "why"])
            .write_stdin("   Compiling foo v0.1.0\n   Compiling bar v0.2.0\n")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(v["data"]["hint"].as_str().unwrap().contains("2>&1"));
}

/// The saved input: exact bytes, one file per distinct input, its path in `data.saved_input`
/// with `complete` true; `--no-save`, `JEVIFY_NO_SAVE=1` and an unwritable store give
/// `complete` false and no path; a save prunes the store's own files past retention and
/// nothing else.
#[tokio::test(flavor = "multi_thread")]
async fn saved_input_is_exact_once_optional_and_pruned() {
    use std::os::unix::fs::PermissionsExt;
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "error"),
        noul: |_, _| 0.93,
    })
    .await;
    let root = tempfile::tempdir().unwrap().keep();
    let outputs = root.join("outputs");
    std::fs::create_dir_all(&outputs).unwrap();
    let stale = outputs.join("0123456789abcdef.log");
    let outside = root.join("keep.log");
    for path in [&stale, &outside] {
        std::fs::write(path, b"aged").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 60 * 60);
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
    }
    let input = b"before\r\nerror: bad \xff\r\nafter\r\n";
    let mut paths = Vec::new();
    for _ in 0..2 {
        let mut cmd = common::jevify(&server);
        cmd.env("JEVIFY_CACHE_DIR", &root);
        let out = tokio::task::spawn_blocking(move || {
            cmd.args(["--json", "why"])
                .write_stdin(input.as_slice())
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(out.status.code(), Some(0));
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["data"]["complete"], true);
        let path = value["data"]["saved_input"].as_str().unwrap().to_owned();
        assert!(std::path::Path::new(&path).starts_with(&outputs));
        assert_eq!(std::fs::read(&path).unwrap(), input);
        paths.push(path);
    }
    assert_eq!(paths[0], paths[1]);
    assert!(!stale.exists(), "a saved input past retention is deleted");
    assert_eq!(std::fs::read(&outside).unwrap(), b"aged");
    assert_eq!(std::fs::read_dir(&outputs).unwrap().count(), 1);

    let locked = tempfile::tempdir().unwrap().keep();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
    for (dir, flags, env) in [
        (&root, vec!["--no-save"], None),
        (&root, vec![], Some("1")),
        (&locked, vec![], None),
    ] {
        let mut cmd = common::jevify(&server);
        cmd.env("JEVIFY_CACHE_DIR", dir)
            .args(["--json", "why"])
            .args(&flags);
        if let Some(value) = env {
            cmd.env("JEVIFY_NO_SAVE", value);
        }
        let out =
            tokio::task::spawn_blocking(move || cmd.write_stdin("error: boom\n").output().unwrap())
                .await
                .unwrap();
        assert_eq!(out.status.code(), Some(0), "{flags:?} {env:?}");
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["data"]["complete"], false, "{flags:?} {env:?}");
        assert!(value["data"]["saved_input"].is_null(), "{flags:?} {env:?}");
        assert_eq!(value["data"]["causes"][0]["text"], "error: boom");
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(std::fs::read_dir(&outputs).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(&locked).unwrap().count(), 0);
}

/// Keyless, the finals are one classifier.dev request of at most 99 labels plus NONE, whatever
/// the shortlist and the panic lines its finalists bring along (the mock rejects more).
#[tokio::test(flavor = "multi_thread")]
async fn keyless_finals_never_exceed_the_window() {
    let server = common::mock_classifier(FakeJev {
        choose: |_, s, o| option_containing(s, o, "panicked at"),
        noul: |_, _| 0.95,
    })
    .await;
    let windows = 14;
    let mut lines: Vec<String> = Vec::new();
    for w in 0..windows {
        for i in 0..92 {
            lines.push(format!("error step {w}.{i}"));
        }
        lines.push(format!(
            "thread 'case_{w}' (1) panicked at tests/case.rs:{}:9:",
            w + 1
        ));
        for m in 0..6 {
            lines.push(format!(
                "assertion `left == right` failed: case {w} part {m}"
            ));
        }
    }
    let input = format!("{}\n", lines.join("\n"));
    let mut cmd = common::jevify_classifier(&server);
    let out = tokio::task::spawn_blocking(move || {
        cmd.args(["--json", "why", "--no-save"])
            .write_stdin(input)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["data"]["causes"][0]["text"]
            .as_str()
            .unwrap()
            .contains("panicked at"),
        "{v}"
    );
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    let labels = body["dimensions"]["pick"]["labels"].as_array().unwrap();
    assert!(labels.len() <= 100, "{} labels", labels.len());
}
