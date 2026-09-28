mod common;
use common::FakeJev;

fn refund_answers() -> FakeJev {
    FakeJev {
        choose: |_, _, o| o[0].clone(),
        noul: |_, s| {
            if s.to_string().contains("refund") {
                0.9
            } else if s.to_string().contains("maybe") {
                0.5
            } else {
                0.1
            }
        },
    }
}

/// yes, no and unsure are exit 0, 1 and 3, with the probability in `data.p`. `ok` stays true
/// on all three (it says jevify itself finished without an error of its own); `exit_code` is
/// the field to branch on.
#[tokio::test(flavor = "multi_thread")]
async fn verdicts_are_exit_0_1_3_with_ok_true_and_the_probability() {
    let server = common::mock(refund_answers()).await;
    for (input, code, verdict, p) in [
        ("I want a refund", 0, "yes", 0.9),
        ("hello there", 1, "no", 0.1),
        ("maybe something", 3, "unsure", 0.5),
    ] {
        for machine in [false, true] {
            let mut c = common::jevify(&server);
            if machine {
                c.arg("--json");
            }
            let out = tokio::task::spawn_blocking(move || {
                c.args(["is", "asks for a refund"])
                    .write_stdin(input)
                    .output()
                    .unwrap()
            })
            .await
            .unwrap();
            assert_eq!(out.status.code(), Some(code), "{input}");
            if machine {
                let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(v["exit_code"], code, "{v}");
                assert_eq!(v["ok"], true, "{v}");
                assert!(v["error"].is_null(), "{v}");
                assert_eq!(v["data"]["verdict"], verdict);
                assert_eq!(v["data"]["p"], p);
            } else {
                assert!(out.stdout.is_empty());
            }
        }
    }
}

/// Input past the evidence budget abstains (exit 3) without a request, and says so.
#[tokio::test(flavor = "multi_thread")]
async fn oversized_input_abstains_without_a_request() {
    let server = common::mock(FakeJev {
        choose: |_, _, o| o[0].clone(),
        noul: |_, _| 0.95,
    })
    .await;
    let mut c = common::jevify(&server);
    let input = format!(
        "{}\nrefund\n{}",
        "ordinary text ".repeat(4000),
        "ordinary text ".repeat(4000)
    );
    let out = tokio::task::spawn_blocking(move || {
        c.args(["--json", "is", "contains a refund"])
            .write_stdin(input)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["data"]["p"].is_null());
    assert_eq!(v["data"]["verdict"], "unsure");
    assert_eq!(v["data"]["truncated"], true);
    assert!(server.received_requests().await.unwrap().is_empty());
}

fn statement_answers() -> FakeJev {
    FakeJev {
        choose: |_, _, o| o[0].clone(),
        noul: |question, _| {
            if question.contains("\"no\"") {
                0.1
            } else if question.contains("\"unsure\"") {
                0.5
            } else {
                0.9
            }
        },
    }
}

/// Several statements: one line per statement in input order and the aggregate verdict as
/// the exit; 21 statements fit the keyless backend's chunk of 20 questions per request.
#[tokio::test(flavor = "multi_thread")]
async fn statements_keep_their_order_and_aggregate_the_verdict() {
    let server = common::mock(statement_answers()).await;
    for (statements, code, verdict) in [
        (vec!["yes", "no"], 1, "no"),
        (vec!["yes", "yes", "yes"], 0, "yes"),
        (vec!["no", "unsure", "yes"], 1, "no"),
        (vec!["yes", "unsure", "yes"], 3, "unsure"),
    ] {
        for machine in [false, true] {
            let mut c = common::jevify(&server);
            if machine {
                c.arg("--json");
            }
            c.arg("is").args(&statements);
            let out = tokio::task::spawn_blocking(move || c.write_stdin("text").output().unwrap())
                .await
                .unwrap();
            assert_eq!(out.status.code(), Some(code), "{statements:?}");
            if machine {
                let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(v["data"]["verdict"], verdict);
                let entries = v["data"]["statements"].as_array().unwrap();
                assert_eq!(entries.len(), statements.len());
                for (entry, statement) in entries.iter().zip(&statements) {
                    assert_eq!(entry["statement"], *statement);
                    assert_eq!(entry["verdict"], *statement);
                }
            } else {
                let expected: String = statements.iter().map(|s| format!("{s}\t{s}\n")).collect();
                assert_eq!(out.stdout, expected.as_bytes());
            }
        }
    }
    let server = common::mock_classifier(statement_answers()).await;
    let statements: Vec<String> = (0..21).map(|i| format!("statement {i}")).collect();
    let mut c = common::jevify_classifier(&server);
    c.arg("is").args(&statements);
    let out = tokio::task::spawn_blocking(move || c.write_stdin("text").output().unwrap())
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let expected: String = statements.iter().map(|s| format!("yes\t{s}\n")).collect();
    assert_eq!(out.stdout, expected.as_bytes());
}

/// `--context FILE` is the input, read like stdin (lossy, trimmed), and stdin is then ignored.
/// A missing or empty context is an input error, an oversized one abstains, empty stdin is
/// an input error: none of them makes a request.
#[tokio::test(flavor = "multi_thread")]
async fn context_file_replaces_stdin_and_its_errors_make_no_requests() {
    let server = common::mock(statement_answers()).await;
    let dir = tempfile::tempdir().unwrap().keep();
    let path = dir.join("context");
    let text = b"\x1b[31mtext\x1b[0m  \r\ninvalid \xff\n";
    std::fs::write(&path, text).unwrap();
    let mut results = Vec::new();
    for file in [false, true] {
        let mut c = common::jevify(&server);
        c.args(["--json", "is", "yes", "no"]);
        if file {
            c.arg("--context").arg(&path);
        }
        let out = tokio::task::spawn_blocking(move || {
            c.write_stdin(if file {
                &b"unrelated stdin"[..]
            } else {
                &text[..]
            })
            .output()
            .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        results.push(value["data"].clone());
    }
    assert_eq!(results[0], results[1]);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);

    for (name, contents, code) in [
        ("missing", None, 6),
        ("empty", Some(String::new()), 6),
        ("huge", Some("x".repeat(96_001)), 3),
    ] {
        let path = dir.join(name);
        if let Some(contents) = &contents {
            std::fs::write(&path, contents).unwrap();
        }
        let mut c = common::jevify(&server);
        c.args(["--json", "is", "yes", "no", "--context"])
            .arg(&path);
        let out = tokio::task::spawn_blocking(move || c.write_stdin("ignored").output().unwrap())
            .await
            .unwrap();
        assert_eq!(out.status.code(), Some(code), "{name}");
    }
    let mut c = common::jevify(&server);
    let out = tokio::task::spawn_blocking(move || {
        c.args(["is", "yes"]).write_stdin("").output().unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(6));
    assert!(out.stdout.is_empty());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
