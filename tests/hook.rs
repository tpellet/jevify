//! `jevify why --hook`: the contract of a coding agent's tool hook. The hook reads the agent's
//! tool-result payload on stdin and prints hook JSON with the pointed line, or nothing; it
//! always exits 0, since a hook that fails breaks the session it serves.
mod common;
use common::{FakeJev, option_containing};
use serde_json::{Value, json};
use wiremock::MockServer;

/// A failed command's output: `lines` lines, the cause at `cause` (1-based).
fn output(lines: usize, cause: usize) -> String {
    (1..=lines)
        .map(|i| {
            if i == cause {
                "error: cause of the failure".to_owned()
            } else {
                format!("step {i} ran")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

async fn cause_server() -> MockServer {
    common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "error: cause"),
        noul: |_, _| 0.95,
    })
    .await
}

/// `jevify why --no-save --hook ARGS` with `stdin`, built here and run off the runtime.
async fn run(server: &MockServer, args: &[&str], stdin: String) -> std::process::Output {
    let mut cmd = common::jevify(server);
    cmd.args(["why", "--no-save", "--hook"]).args(args);
    tokio::task::spawn_blocking(move || cmd.write_stdin(stdin).output().unwrap())
        .await
        .unwrap()
}

/// The hook's answer: exit 0 always; stdout parsed as the hook JSON when there is one.
fn answer(out: &std::process::Output) -> Option<Value> {
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    (!out.stdout.is_empty()).then(|| serde_json::from_slice(&out.stdout).unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_failing_output_becomes_hook_json_with_the_pointed_line() {
    let server = cause_server().await;
    // Claude Code: PostToolUseFailure carries the output in `error` behind the exit code line.
    let payload = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test"},
        "error": format!("Exit code 101\n{}", output(100, 42)),
        "is_interrupt": false,
    });
    let out = run(&server, &["claude"], payload.to_string()).await;
    let v = answer(&out).expect("hook JSON on stdout");
    let specific = &v["hookSpecificOutput"];
    assert_eq!(specific["hookEventName"], "PostToolUseFailure");
    let context = specific["additionalContext"].as_str().unwrap();
    assert!(
        context.starts_with(
            "jevify why points at line 42 of the 100 output lines of this failed command as the cause (output not saved):\n"
        ),
        "{context}"
    );
    assert!(
        context.contains(">    42 │ error: cause of the failure"),
        "{context}"
    );
    assert!(context.contains("    41 │ step 41 ran"), "{context}");
    // Only the hook JSON reaches stdout: one object, nothing before or after it.
    assert_eq!(v.as_object().unwrap().len(), 1);
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .trim_start()
            .starts_with('{')
    );

    // PostToolUse (Claude Code and Codex) carries stdout, stderr and the exit code; the
    // output is stdout then stderr, and the event name is echoed.
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test"},
        "tool_response": {"stdout": output(60, 1), "stderr": output(40, 30), "exit_code": 1, "interrupted": false},
    });
    let out = run(&server, &["codex"], payload.to_string()).await;
    let v = answer(&out).expect("hook JSON on stdout");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    let context = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("of the 100 output lines"), "{context}");
    assert!(
        context.contains("│ error: cause of the failure"),
        "{context}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_short_output_prints_nothing_and_sends_nothing() {
    let server = cause_server().await;
    let payload = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "error": format!("Exit code 1\n{}", output(79, 3)),
        "is_interrupt": false,
    })
    .to_string();
    let out = run(&server, &["claude"], payload.clone()).await;
    assert_eq!(answer(&out), None);
    assert_eq!(server.received_requests().await.unwrap().len(), 0);

    // --min-lines sets the threshold; 79 lines then qualify.
    let out = run(&server, &["claude", "--min-lines", "10"], payload).await;
    assert!(answer(&out).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_payload_with_nothing_to_judge_prints_nothing_and_exits_zero() {
    let server = cause_server().await;
    let long = format!("Exit code 1\n{}", output(100, 42));
    let payloads: Vec<String> = vec![
        // Not JSON at all, truncated JSON, an empty object, an empty stdin.
        "not json".into(),
        "{\"tool_name\": \"Bash\", \"error\": \"".into(),
        "{}".into(),
        String::new(),
        // Another tool, an interrupted command, a command that succeeded, a response of an
        // unexpected shape, a response with no text, an error that is not text.
        json!({"tool_name": "Read", "error": long, "is_interrupt": false}).to_string(),
        json!({"tool_name": "Bash", "error": long, "is_interrupt": true}).to_string(),
        json!({"tool_name": "Bash", "tool_response": {"stdout": output(100, 42), "stderr": "", "exit_code": 0, "interrupted": false}}).to_string(),
        json!({"tool_name": "Bash", "tool_response": {"stdout": output(100, 42), "stderr": "", "exit_code": 1, "interrupted": true}}).to_string(),
        json!({"tool_name": "Bash", "tool_response": 42}).to_string(),
        json!({"tool_name": "Bash", "tool_response": {"stdout": "", "stderr": "", "exit_code": 1}}).to_string(),
        json!({"tool_name": "Bash", "error": 7}).to_string(),
    ];
    for payload in payloads {
        let out = run(&server, &["claude"], payload.clone()).await;
        assert_eq!(answer(&out), None, "payload {payload}");
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abstention_or_a_backend_failure_prints_nothing_and_exits_zero() {
    // The backend sees no failure: `why` abstains, the hook adds nothing.
    let abstaining = common::mock(FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.05,
    })
    .await;
    let payload = json!({
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "error": format!("Exit code 1\n{}", output(100, 42)),
        "is_interrupt": false,
    })
    .to_string();
    let out = run(&abstaining, &["claude"], payload.clone()).await;
    assert_eq!(answer(&out), None);

    // The backend answers nothing useful (every request 404): still silent, still 0.
    let broken = MockServer::start().await;
    let out = run(&broken, &["claude"], payload.clone()).await;
    assert_eq!(answer(&out), None);

    // No key at all on the TypeSafe backend: the auth error stays inside the hook.
    let mut cmd = common::jevify(&abstaining);
    cmd.env_remove("TYPESAFE_API_KEY")
        .args(["why", "--no-save", "--hook", "claude"]);
    let out = tokio::task::spawn_blocking(move || cmd.write_stdin(payload).output().unwrap())
        .await
        .unwrap();
    assert_eq!(answer(&out), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hook_flag_rejects_an_unknown_host_and_the_json_envelope() {
    let server = cause_server().await;
    for args in [
        vec!["why", "--hook", "cursor"],
        vec!["--json", "why", "--hook", "claude"],
        vec!["why", "--hook"],
    ] {
        let mut cmd = common::jevify(&server);
        cmd.args(&args);
        let out = tokio::task::spawn_blocking(move || cmd.write_stdin("{}").output().unwrap())
            .await
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}
