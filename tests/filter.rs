mod common;

use assert_cmd::cargo::CommandCargoExt;
use common::FakeJev;
use serde_json::Value;
use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

/// The three options of `filter` arrive in key order; the fake answers by their wording.
fn answer(options: &[String], side: &str) -> String {
    options
        .iter()
        .find(|option| option.ends_with(side))
        .cloned()
        .expect("one of the three filter options ends with the side")
}

fn fake() -> FakeJev {
    FakeJev {
        choose: |_, state, options| {
            let side = match state.as_str().unwrap_or_default() {
                "no" => "does not hold",
                "unsure" => "does not say",
                _ => "statement holds",
            };
            answer(options, side)
        },
        noul: |_, _| unreachable!(),
    }
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
    assert_eq!(value["command"], "filter");
    value
}

fn records(count: usize) -> String {
    (0..count).map(|i| format!("record {i}\n")).collect()
}

#[tokio::test]
async fn verdicts_inversion_counts_and_machine_records() {
    let server = common::mock_classifier(fake()).await;
    for (flags, expected, kept) in [
        (vec![], "yes\nunsure\nyes\n", 3),
        (vec!["-v"], "no\nunsure\n", 2),
        (vec!["--strict"], "yes\nyes\n", 2),
        (vec!["-v", "--strict"], "no\n", 1),
        (vec!["-c"], "3\n", 3),
    ] {
        for machine in [false, true] {
            let mut cmd = common::jevify_classifier(&server);
            cmd.args(["filter", "x"]).args(&flags);
            if machine {
                cmd.arg("--json");
            }
            let out = cmd.write_stdin("yes\nno\nunsure\nyes\n").output().unwrap();
            assert_eq!(out.status.code(), Some(0));
            if machine {
                let value = envelope(&out, 0);
                assert_eq!(value["data"]["kept"], kept);
                assert_eq!(value["data"]["unsure"], 1);
            } else {
                assert_eq!(out.stdout, expected.as_bytes());
            }
        }
    }
}

/// Exit 1 when nothing is kept, 3 when only unsure records are; the preflight errors (empty
/// input, too many records, conflicting splits) need no backend and save nothing.
#[tokio::test]
async fn exit_codes_and_preflight_errors() {
    let server = common::mock_classifier(fake()).await;
    for (input, flags, code, kept) in [
        ("no\n", vec![], 1, 0),
        ("unsure\n", vec![], 3, 1),
        ("unsure\n", vec!["--strict"], 3, 0),
        ("no\nunsure\n", vec!["--strict"], 1, 0),
        (" \n\t\r\n", vec![], 1, 0),
    ] {
        let out = common::jevify_classifier(&server)
            .args(["filter", "x", "--json"])
            .args(&flags)
            .write_stdin(input)
            .output()
            .unwrap();
        assert_eq!(
            envelope(&out, code)["data"]["kept"],
            kept,
            "{input:?} {flags:?}"
        );
    }
    for (input, flags, code, kind) in [
        (String::new(), vec![], 6, "empty_input"),
        (records(20_001), vec![], 6, "too_many"),
        ("yes\n".into(), vec!["-0", "--para"], 2, "usage"),
    ] {
        let out = common::bin()
            .args(["filter", "x", "--json", "--no-save"])
            .args(flags)
            .write_stdin(input)
            .output()
            .unwrap();
        let value = envelope(&out, code);
        assert_eq!(value["error"]["kind"], kind);
        assert_eq!(value["meta"]["requests"], 0);
    }
}

/// Records come out byte for byte (ANSI, CR, invalid UTF-8), in input order, under every
/// split; the machine records carry the ordinal and the lossy mark.
#[tokio::test]
async fn bytes_splits_and_order_are_exact() {
    let server = common::mock_classifier(fake()).await;
    for (input, flag, expected) in [
        (
            b"\x1b[31myes\x1b[0m\r\n\n\xff\r\n\x1b[31myes\x1b[0m\r\n".as_slice(),
            "",
            b"\x1b[31myes\x1b[0m\r\n\xff\r\n\x1b[31myes\x1b[0m\r\n".as_slice(),
        ),
        (b"yes\0\0\xff\0yes\0", "-0", b"yes\0\xff\0yes\0"),
        (
            b"yes\nline\n\nother\r\n\r\n",
            "--para",
            b"yes\nline\n\nother\r\n\r\n",
        ),
        (b"a\0b\nlast", "", b"a\0b\nlast"),
    ] {
        for machine in [false, true] {
            let mut cmd = common::jevify_classifier(&server);
            cmd.args(["filter", "x"]);
            if !flag.is_empty() {
                cmd.arg(flag);
            }
            if machine {
                cmd.arg("--json");
            }
            let out = cmd.write_stdin(input).output().unwrap();
            assert_eq!(out.status.code(), Some(0));
            if machine {
                let value = envelope(&out, 0);
                let entries = value["data"]["records"].as_array().unwrap();
                let joined: String = entries
                    .iter()
                    .map(|e| e["text"].as_str().unwrap())
                    .collect();
                assert_eq!(joined, String::from_utf8_lossy(expected));
                if input.contains(&255) {
                    assert_eq!(entries[1]["lossy"], true);
                    assert_eq!(entries[1]["ordinal"], 2);
                }
            } else {
                assert_eq!(out.stdout, expected);
            }
        }
    }
}

/// The saved input: exact bytes at `data.saved_input` with `complete` true, one file per
/// distinct input; `--no-save` gives `complete` false and no path.
#[tokio::test]
async fn saved_input_is_exact_and_optional() {
    let server = common::mock_classifier(fake()).await;
    let root = tempfile::tempdir().unwrap().keep();
    for no_save in [false, false, true] {
        let mut cmd = common::jevify_classifier(&server);
        cmd.env("JEVIFY_CACHE_DIR", &root)
            .args(["filter", "x", "--json"]);
        if no_save {
            cmd.arg("--no-save");
        }
        let out = cmd.write_stdin("yes\nunsure\nno\n").output().unwrap();
        let value = envelope(&out, 0);
        if no_save {
            assert_eq!(value["data"]["complete"], false);
            assert!(value["data"]["saved_input"].is_null());
        } else {
            assert_eq!(value["data"]["complete"], true);
            let saved = value["data"]["saved_input"].as_str().unwrap();
            assert_eq!(std::fs::read(saved).unwrap(), b"yes\nunsure\nno\n");
        }
    }
    assert_eq!(std::fs::read_dir(root.join("outputs")).unwrap().count(), 1);
}

/// `--files` judges an excerpt of each file; a secret file's excerpt never leaves the machine
/// and is counted as withheld.
#[tokio::test]
async fn file_excerpts_are_judged_and_secret_files_are_withheld() {
    let server = common::mock_classifier(fake()).await;
    let root = tempfile::tempdir().unwrap().keep();
    std::fs::write(root.join("source.rs"), "VISIBLE_EXCERPT").unwrap();
    std::fs::write(root.join(".npmrc"), "PRIVATE_EXCERPT").unwrap();
    let out = common::jevify_classifier(&server)
        .current_dir(&root)
        .args(["filter", "x", "-0", "--files", "--json"])
        .write_stdin(b"./source.rs\0.npmrc\0".as_slice())
        .output()
        .unwrap();
    let value = envelope(&out, 0);
    assert_eq!(value["data"]["records"][0]["text"], "./source.rs\0");
    assert_eq!(value["data"]["excerpts_withheld"], 1);
    assert_eq!(value["data"]["kept"], 2);
    for request in server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST")
    {
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("VISIBLE_EXCERPT"));
        assert!(!body.contains("PRIVATE_EXCERPT"));
    }
}

#[derive(Clone)]
struct Batches {
    requests: Arc<AtomicUsize>,
    quota_requests: Arc<AtomicUsize>,
    quota: Option<&'static str>,
    delay_first: bool,
    delay_last: bool,
}

impl Respond for Batches {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let first = body["items"][0].as_str().unwrap();
        if first == "record 1020" {
            if let Some(code) = self.quota {
                self.quota_requests.fetch_add(1, Ordering::SeqCst);
                return FakeJev::quota(code, 1);
            }
        }
        let response = common::FakeClassifier(fake()).respond(request);
        if (self.delay_last && first != "record 0") || (self.delay_first && first == "record 0") {
            response.set_delay(Duration::from_secs(3))
        } else {
            response
        }
    }
}

async fn batches(
    quota: Option<&'static str>,
    delay_first: bool,
    delay_last: bool,
) -> (MockServer, Batches) {
    let server = MockServer::start().await;
    let responder = Batches {
        requests: Arc::new(AtomicUsize::new(0)),
        quota_requests: Arc::new(AtomicUsize::new(0)),
        quota,
        delay_first,
        delay_last,
    };
    Mock::given(method("POST"))
        .and(path("/v1/classify"))
        .respond_with(responder.clone())
        .mount(&server)
        .await;
    (server, responder)
}

/// Four batches in flight and the first one slowest: every record still comes out in input
/// order, duplicates at their original positions.
#[tokio::test]
async fn later_batches_wait_and_duplicates_keep_their_original_positions() {
    let (server, _) = batches(None, true, false).await;
    let input = format!("{}record 0\nrecord 1499\n", records(1500));
    let out = common::jevify_classifier(&server)
        .env("JEVIFY_CONCURRENCY", "4")
        .args(["filter", "x"])
        .write_stdin(input.clone())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, input.as_bytes());
}

/// HTTP 402 is API unavailable and not retried: one request per batch.
#[tokio::test]
async fn per_request_spending_limit_is_an_input_error_and_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/classify"))
        .respond_with(ResponseTemplate::new(402).set_body_json(serde_json::json!({
            "error": "request exceeds the free spending limit", "code": "request_spending_limit"
        })))
        .mount(&server)
        .await;
    let out = common::jevify_classifier(&server)
        .args(["filter", "x", "--json"])
        .write_stdin(records(70))
        .output()
        .unwrap();
    assert_eq!(envelope(&out, 6)["error"]["kind"], "input_too_large");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

/// The daily quota mid-run: the answered prefix reaches stdout in order, then exit 4 with the
/// kind under `--json`; the limit is asked once, never retried.
#[tokio::test]
async fn daily_quota_keeps_the_answered_prefix() {
    for machine in [false, true] {
        let (server, responder) = batches(Some("rate_limit_day"), false, false).await;
        let mut cmd = common::jevify_classifier(&server);
        cmd.env("JEVIFY_CONCURRENCY", "1").args(["filter", "x"]);
        if machine {
            cmd.arg("--json");
        }
        let out = cmd.write_stdin(records(2500)).output().unwrap();
        if machine {
            assert_eq!(envelope(&out, 4)["error"]["kind"], "api_unavailable");
        } else {
            assert_eq!(
                out.status.code(),
                Some(4),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(out.stdout, records(1020).as_bytes());
        }
        assert_eq!(responder.quota_requests.load(Ordering::SeqCst), 1);
    }
}

fn process(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("jevify").unwrap();
    for var in [
        "TYPESAFE_API_KEY",
        "TYPESAFE_API_KEY_FILE",
        "JEVIFY_THRESHOLD",
        "JEVIFY_MODEL",
        "JEVIFY_INVENTORY_FILE",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("JEVIFY_BACKEND", "classifier")
        .env("JEVIFY_BASE_URL", server.uri())
        .env("JEVIFY_CACHE_DIR", tempfile::tempdir().unwrap().keep())
        .env("JEVIFY_CONFIG_DIR", tempfile::tempdir().unwrap().keep())
        .env("JEVIFY_NO_CACHE", "1")
        .env("JEVIFY_CONCURRENCY", "1")
        .args(["filter", "x", "--no-save"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Two keyless batches of 60 records, the second delayed 3 s: the first line reaches stdout
/// within 2 s while the child still runs, so answers flow before the last one arrives. A
/// reader that closes the pipe stops the run before every batch is sent.
#[tokio::test]
async fn stdout_flows_before_last_answer_and_closed_pipe_cancels() {
    let (server, _) = batches(None, false, true).await;
    let mut child = process(&server).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(records(120).as_bytes()).unwrap());
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        tx.send(first.clone()).unwrap();
        reader.read_to_string(&mut first).unwrap();
        first
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "record 0\n"
    );
    assert!(child.try_wait().unwrap().is_none());
    assert!(child.wait().unwrap().success());
    writer.join().unwrap();
    assert_eq!(reader.join().unwrap(), records(120));

    let (server, responder) = batches(None, false, true).await;
    let mut child = process(&server).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(records(10_074).as_bytes()).unwrap());
    let stdout = child.stdout.take().unwrap();
    let mut reader = std::io::BufReader::new(stdout);
    for i in 0..3 {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, format!("record {i}\n"));
    }
    drop(reader);
    assert_eq!(child.wait().unwrap().code(), Some(0));
    writer.join().unwrap();
    assert!(responder.requests.load(Ordering::SeqCst) < 11);
}

/// `filter --label a,b,c`: the labelling mode. One Choice per record over the labels and
/// NONE; `LABEL<TAB>RECORD` out, `?` when unsure; the envelope carries `labelled`.
mod label {
    use crate::common::{self, FakeJev};
    use serde_json::Value;

    /// The record under judgment, on either wire: classifier.dev sends the record as the
    /// state, TypeSafe sends every record of the batch and names the record in the
    /// instructions.
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
    /// splits the probability between the first two labels, `close` puts the best label under
    /// the winner ratio, `nothing` gives NONE the win, `tie` ties NONE with the best label.
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
        assert_eq!(value["command"], "filter");
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

    /// Lines: `LABEL<TAB>RECORD` with the record's bytes unchanged (ANSI, CR, tabs, invalid
    /// UTF-8) and blank lines dropped; the machine records carry ordinal, p and the lossy mark.
    #[tokio::test]
    async fn lines_come_back_byte_exact_after_the_first_tab() {
        let server = common::mock_classifier(fake()).await;
        let input = b"a bug report\r\n\n\x1b[31ma feature\x1b[0m\twith a tab\n \t\nanother bug\nquestion \xff here\nbug again\n";
        let without_blank_lines =
            b"a bug report\r\n\x1b[31ma feature\x1b[0m\twith a tab\nanother bug\nquestion \xff here\nbug again\n";
        let out = common::jevify_classifier(&server)
            .args(["filter", "--label", "bug,feature,question"])
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
            .args(["filter", "--label", "bug,feature,question", "--json"])
            .write_stdin(input.as_slice())
            .output()
            .unwrap();
        let value = envelope(&out, 0);
        assert_eq!(value["data"]["labelled"], 5);
        assert_eq!(value["data"]["total"], 5);
        assert_eq!(value["data"]["unsure"], 0);
        assert_eq!(value["data"]["complete"], true);
        assert!(value["data"].get("kept").is_none());
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
            .args(["filter", "--label", "bug,feature,question", "--para"])
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
            .args(["filter", "--label", "bug,feature,question", "-0"])
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
                cmd.args(["filter", "--label", "bug,feature"]);
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
                    .args(["filter", "--label", &labels.join(","), "--json"])
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

    /// Empty input, too many records, conflicting splits and the statement-mode flags fail
    /// before any request; duplicates are judged once and come out at every position; the
    /// input is not saved.
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
            ("bug\n".into(), vec!["-v"], 2, "usage"),
            ("bug\n".into(), vec!["-c"], 2, "usage"),
            ("bug\n".into(), vec!["--strict"], 2, "usage"),
            ("bug\n".into(), vec!["is a bug"], 2, "usage"),
        ] {
            let out = common::jevify_classifier(&server)
                .args(["filter", "--label", "bug,feature", "--json"])
                .args(flags)
                .write_stdin(input)
                .output()
                .unwrap();
            let value = envelope(&out, code);
            assert_eq!(value["error"]["kind"], kind);
            assert_eq!(value["meta"]["requests"], 0);
        }
        let root = tempfile::tempdir().unwrap().keep();
        let out = common::jevify_classifier(&server)
            .env("JEVIFY_CACHE_DIR", &root)
            .args(["filter", "--label", "bug,feature"])
            .write_stdin("bug\nfeature\nbug\nbug\n")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(
            out.stdout,
            b"bug\tbug\nfeature\tfeature\nbug\tbug\nbug\tbug\n"
        );
        assert!(!root.join("outputs").exists());
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
    /// with the withheld excerpts, and is never sent; nothing readable is exit 3 without a
    /// request.
    #[tokio::test]
    async fn unreadable_files_are_left_unsure_and_never_sent() {
        let server = common::mock_classifier(fake()).await;
        let root = tempfile::tempdir().unwrap().keep();
        std::fs::write(root.join("readable.md"), "VISIBLE_EXCERPT feature").unwrap();
        std::fs::create_dir(root.join("adir")).unwrap();
        let out = common::jevify_classifier(&server)
            .current_dir(&root)
            .args([
                "filter",
                "--label",
                "bug,feature",
                "-0",
                "--files",
                "--json",
            ])
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
            .args(["filter", "--label", "bug,feature", "-0", "--files"])
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
}
