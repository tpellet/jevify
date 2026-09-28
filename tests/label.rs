mod common;

use common::FakeJev;
use serde_json::Value;

/// The record under judgment, on either wire: classifier.dev sends the record as the state,
/// TypeSafe sends every record of the batch and names the record in the instructions.
fn record_text(instructions: &str, state: &Value) -> String {
    if let Some(text) = state.as_str() {
        return text.to_string();
    }
    let id: usize = instructions
        .strip_prefix("Judge record id ")
        .and_then(|rest| rest.split(' ').next())
        .and_then(|id| id.parse().ok())
        .expect("TypeSafe questions name their record");
    state["items"][id]["text"].as_str().unwrap().to_string()
}

/// A fake that answers from the record's text: a word names the winning label, `unsure`
/// splits the probability between the first two labels, `close` puts the best label under the
/// winner ratio, `nothing` gives NONE the win, `tie` ties NONE with the best label.
fn fake() -> common::ConfiguredFake {
    FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| unreachable!(),
    }
    .with_probabilities(|instructions, state, labels| {
        let text = record_text(instructions, state);
        let n = labels.len();
        let none = labels.iter().position(|l| l == "NONE").unwrap();
        let first = usize::from(none == 0);
        let second = if none == 1 { 2 } else { first + 1 };
        let mut vector = vec![0.0; n];
        let rest = |v: &mut [f64], top: usize, p: f64| {
            let share = (1.0 - p) / (n - 1) as f64;
            for (i, slot) in v.iter_mut().enumerate() {
                *slot = if i == top { p } else { share };
            }
        };
        if text.contains("unsure") {
            vector[first] = 0.45;
            vector[second] = 0.45;
            vector[none] = 0.1;
        } else if text.contains("close") {
            vector[first] = 0.5;
            vector[second] = 0.4;
            vector[none] = 0.1;
        } else if text.contains("nothing") {
            rest(&mut vector, none, 0.7);
        } else if text.contains("tie") {
            vector[first] = 0.4;
            vector[none] = 0.4;
            vector[second] = 0.2;
        } else if let Some(hit) = labels
            .iter()
            .position(|l| l != "NONE" && text.contains(l.as_str()))
        {
            rest(&mut vector, hit, 0.8);
        } else {
            rest(&mut vector, first, 0.8);
        }
        vector
    })
}

fn envelope(out: &std::process::Output, code: i32) -> Value {
    assert_eq!(
        out.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["exit_code"], code);
    assert_eq!(value["command"], "label");
    value
}

/// `cut -f2-`: the bytes after the first tab of each record of the output.
fn cut_labels(output: &[u8], terminator: u8) -> (Vec<String>, Vec<u8>) {
    let mut labels = Vec::new();
    let mut rest = Vec::new();
    for record in output.split_inclusive(|b| *b == terminator) {
        let tab = record.iter().position(|b| *b == b'\t').unwrap();
        labels.push(String::from_utf8(record[..tab].to_vec()).unwrap());
        rest.extend_from_slice(&record[tab + 1..]);
    }
    (labels, rest)
}

/// Lines: `LABEL<TAB>RECORD` with the record's bytes unchanged (ANSI, CR, tabs, invalid UTF-8)
/// and blank lines dropped; the machine records carry ordinal, p and the lossy mark.
#[tokio::test]
async fn lines_come_back_byte_exact_after_the_first_tab() {
    let server = common::mock_classifier(fake()).await;
    let input = b"a bug report\r\n\n\x1b[31ma feature\x1b[0m\twith a tab\n \t\nanother bug\nquestion \xff here\nbug again\n";
    let without_blank_lines =
        b"a bug report\r\n\x1b[31ma feature\x1b[0m\twith a tab\nanother bug\nquestion \xff here\nbug again\n";
    let out = common::jevify_classifier(&server)
        .args(["label", "bug,feature,question"])
        .write_stdin(input.as_slice())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (labels, rest) = cut_labels(&out.stdout, b'\n');
    assert_eq!(rest, without_blank_lines);
    assert_eq!(labels, ["bug", "feature", "bug", "question", "bug"]);

    let out = common::jevify_classifier(&server)
        .args(["label", "bug,feature,question", "--json"])
        .write_stdin(input.as_slice())
        .output()
        .unwrap();
    let value = envelope(&out, 0);
    assert_eq!(value["data"]["labelled"], 5);
    assert_eq!(value["data"]["total"], 5);
    assert_eq!(value["data"]["unsure"], 0);
    assert_eq!(value["data"]["complete"], true);
    let entries = value["data"]["records"].as_array().unwrap();
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[3]["label"], "question");
    assert_eq!(entries[3]["text"], "question \u{fffd} here\n");
    assert_eq!(entries[3]["lossy"], true);
    assert_eq!(entries[3]["ordinal"], 4);
    assert_eq!(entries[3]["p"], 0.8);
    assert!(entries[0].get("lossy").is_none());
    assert_eq!(entries[0]["text"], "a bug report\r\n");
}

#[tokio::test]
async fn paragraphs_and_nul_records_follow_the_first_tab_unchanged() {
    let server = common::mock_classifier(fake()).await;
    let paragraph = b"a bug\nsecond\tline\r\n";
    let input = format!(
        "{}\nfeature one\n\n\nquestion\n",
        String::from_utf8_lossy(paragraph)
    );
    let out = common::jevify_classifier(&server)
        .args(["label", "bug,feature,question", "--para"])
        .write_stdin(input.as_bytes())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let tab = out.stdout.iter().position(|b| *b == b'\t').unwrap();
    assert_eq!(&out.stdout[..tab], b"bug");
    let first_record = paragraph.len() + 1;
    assert_eq!(
        &out.stdout[tab + 1..tab + 1 + first_record],
        b"a bug\nsecond\tline\r\n\n"
    );
    assert_eq!(
        &out.stdout[tab + 1 + first_record..],
        b"feature\tfeature one\n\nquestion\tquestion\n"
    );

    let out = common::jevify_classifier(&server)
        .args(["label", "bug,feature,question", "-0"])
        .write_stdin(b"bug\tone\0\0feature\0\xff question\0tie\0".as_slice())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let (labels, rest) = cut_labels(&out.stdout, 0);
    assert_eq!(labels, ["bug", "feature", "question", "?"]);
    assert_eq!(rest, b"bug\tone\0feature\0\xff question\0tie\0");
}

/// A record with no clear label (split, close, NONE ahead, tied with NONE) is `?`; exit 3
/// when every record is unsure.
#[tokio::test]
async fn unsure_records_ties_and_none_are_question_marks() {
    let server = common::mock_classifier(fake()).await;
    for (input, expected, code) in [
        ("bug\nunsure\nclose\nnothing\ntie\n", "bug\t?\t?\t?\t?\t", 0),
        ("bug\nfeature\n", "bug\tfeature\t", 0),
        ("unsure\nnothing\n", "?\t?\t", 3),
    ] {
        for machine in [false, true] {
            let mut cmd = common::jevify_classifier(&server);
            cmd.args(["label", "bug,feature"]);
            if machine {
                cmd.arg("--json");
            }
            let out = cmd.write_stdin(input).output().unwrap();
            let unsure = expected.matches("?\t").count();
            if machine {
                let value = envelope(&out, code);
                assert_eq!(value["data"]["unsure"], unsure);
                let labels: String = value["data"]["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| format!("{}\t", e["label"].as_str().unwrap()))
                    .collect();
                assert_eq!(labels, expected);
            } else {
                assert_eq!(out.status.code(), Some(code));
                let (labels, rest) = cut_labels(&out.stdout, b'\n');
                assert_eq!(rest, input.as_bytes());
                assert_eq!(
                    labels.iter().map(|l| format!("{l}\t")).collect::<String>(),
                    expected
                );
            }
        }
    }
}

/// The label count is checked against the backend's window (99 keyless, 200 on TypeSafe)
/// before any request; at the window the run goes through.
#[tokio::test]
async fn label_count_is_checked_against_the_window_before_any_request() {
    for classifier in [true, false] {
        let server = if classifier {
            common::mock_classifier(fake()).await
        } else {
            common::mock(fake()).await
        };
        let window = if classifier { 99 } else { 200 };
        for count in [window, window + 1] {
            let labels: Vec<String> = (0..count).map(|i| format!("label{i}")).collect();
            let mut cmd = if classifier {
                common::jevify_classifier(&server)
            } else {
                common::jevify(&server)
            };
            let out = cmd
                .args(["label", &labels.join(","), "--json"])
                .write_stdin("label7 here\nlabel0 there\n")
                .output()
                .unwrap();
            let value = envelope(&out, if count > window { 2 } else { 0 });
            if count > window {
                assert_eq!(value["error"]["kind"], "usage");
                assert_eq!(value["meta"]["requests"], 0);
            } else {
                let entries = value["data"]["records"].as_array().unwrap();
                assert_eq!(entries[0]["label"], "label7");
                assert_eq!(entries[1]["label"], "label0");
            }
        }
        let posts = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count();
        assert_eq!(posts, 1);
    }
}

/// Empty input, too many records and conflicting splits fail before any request; duplicates
/// are judged once and come out at every position.
#[tokio::test]
async fn preflight_errors_and_duplicates_judged_once() {
    let server = common::mock_classifier(fake()).await;
    for (input, flags, code, kind) in [
        (String::new(), vec![], 6, "empty_input"),
        (
            (0..20_001)
                .map(|i| format!("record {i}\n"))
                .collect::<String>(),
            vec![],
            6,
            "too_many",
        ),
        ("bug\n".into(), vec!["-0", "--para"], 2, "usage"),
    ] {
        let out = common::jevify_classifier(&server)
            .args(["label", "bug,feature", "--json"])
            .args(flags)
            .write_stdin(input)
            .output()
            .unwrap();
        let value = envelope(&out, code);
        assert_eq!(value["error"]["kind"], kind);
        assert_eq!(value["meta"]["requests"], 0);
    }
    let out = common::jevify_classifier(&server)
        .args(["label", "bug,feature"])
        .write_stdin("bug\nfeature\nbug\nbug\n")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        out.stdout,
        b"bug\tbug\nfeature\tfeature\nbug\tbug\nbug\tbug\n"
    );
    let requests = server.received_requests().await.unwrap();
    let posts: Vec<_> = requests
        .iter()
        .filter(|r| r.url.path() == "/v1/classify")
        .collect();
    assert_eq!(posts.len(), 1);
    let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(body["items"], serde_json::json!(["bug", "feature"]));
}

/// A file jevify cannot read is not judged on its name: it comes out `?` with p 0, counted
/// with the withheld excerpts, and is never sent; nothing readable is exit 3 without a request.
#[tokio::test]
async fn unreadable_files_are_left_unsure_and_never_sent() {
    let server = common::mock_classifier(fake()).await;
    let root = tempfile::tempdir().unwrap().keep();
    std::fs::write(root.join("readable.md"), "VISIBLE_EXCERPT feature").unwrap();
    std::fs::create_dir(root.join("adir")).unwrap();
    let out = common::jevify_classifier(&server)
        .current_dir(&root)
        .args(["label", "bug,feature", "-0", "--files", "--json"])
        .write_stdin(b"./readable.md\0adir\0missing.md\0".as_slice())
        .output()
        .unwrap();
    let value = envelope(&out, 0);
    let records = value["data"]["records"].as_array().unwrap();
    assert_eq!(records[0]["label"], "feature");
    assert!(records[0].get("unreadable").is_none());
    assert_eq!(records[1]["label"], "?");
    assert_eq!(records[1]["p"], 0.0);
    assert_eq!(records[1]["unreadable"], "is a directory");
    assert_eq!(records[2]["label"], "?");
    assert!(
        records[2]["unreadable"]
            .as_str()
            .unwrap()
            .contains("No such file")
    );
    assert_eq!(value["data"]["excerpts_withheld"], 2);
    assert_eq!(value["data"]["unsure"], 2);
    assert_eq!(value["data"]["labelled"], 1);
    for request in server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST")
    {
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("VISIBLE_EXCERPT"), "{body}");
        assert!(
            !body.contains("adir") && !body.contains("missing.md"),
            "{body}"
        );
    }

    let before = server.received_requests().await.unwrap().len();
    let out = common::jevify_classifier(&server)
        .current_dir(&root)
        .args(["label", "bug,feature", "-0", "--files"])
        .write_stdin(b"adir\0missing.md\0".as_slice())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"?\tadir\0?\tmissing.md\0");
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}
