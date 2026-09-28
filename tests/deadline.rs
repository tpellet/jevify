//! The overall budget (`JEVIFY_DEADLINE`) covers the whole verb, the evidence read included:
//! a record source that blocks past the budget ends the verb at exit 4 with kind
//! `api_deadline`, and no request is sent.

mod common;

use assert_cmd::cargo::CommandCargoExt;
use serde_json::Value;
use std::io::Write;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Runs the verb with a one-second budget while stdin stays open for `hold`: the evidence
/// read blocks that long before the first request could be built. Returns the envelope, the
/// exit code and the number of requests sent.
async fn blocked_read(verb: &[&str], hold: Duration) -> (Value, Option<i32>, usize) {
    let server = common::mock_classifier(common::FakeJev {
        choose: |_, _, options| {
            options
                .iter()
                .find(|option| option.ends_with("statement holds"))
                .unwrap_or(&options[0])
                .clone()
        },
        noul: |_, _| 0.9,
    })
    .await;
    let mut cmd = std::process::Command::cargo_bin("jevify").unwrap();
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
        .env("JEVIFY_DEADLINE", "1")
        .args(verb)
        .arg("--json")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"Cargo.toml\n").unwrap();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(hold);
        drop(stdin);
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    let posts = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method == "POST")
        .count();
    let parsed = serde_json::from_slice(&out.stdout);
    assert!(
        parsed.is_ok(),
        "{:?}: {}\n{}",
        parsed.as_ref().err(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (parsed.unwrap(), out.status.code(), posts)
}

/// `filter --files` reads the records and their excerpts before it builds the client, and
/// `pick` builds the client first: the budget counts from the verb's start in both.
#[tokio::test]
async fn a_blocked_evidence_read_past_the_budget_ends_at_exit_4_without_a_request() {
    for verb in [
        &["filter", "--files", "--no-save", "x"][..],
        &["pick", "the manifest"][..],
    ] {
        let start = Instant::now();
        let (value, code, posts) = blocked_read(verb, Duration::from_millis(1500)).await;
        assert_eq!((code, posts), (Some(4), 0), "{verb:?}: {value}");
        assert_eq!(value["exit_code"], 4, "{verb:?}: {value}");
        // Exit 4 covers three situations a caller answers differently; the kind says which.
        assert_eq!(value["error"]["kind"], "api_deadline", "{verb:?}: {value}");
        assert!(
            value["error"]["hint"]
                .as_str()
                .unwrap_or_default()
                .contains("JEVIFY_DEADLINE"),
            "{verb:?}: {value}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "{verb:?}: {:?}",
            start.elapsed()
        );
    }
}

/// A read that ends inside the budget is judged as usual.
#[tokio::test]
async fn a_read_inside_the_budget_is_judged() {
    let (value, code, posts) = blocked_read(
        &["filter", "--files", "--no-save", "x"],
        Duration::from_millis(100),
    )
    .await;
    assert_eq!((code, posts), (Some(0), 1), "{value}");
}
