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
async fn spending_limit_is_exit_4_and_not_retried() {
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
    assert_eq!(envelope(&out, 4)["error"]["kind"], "api_unavailable");
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
