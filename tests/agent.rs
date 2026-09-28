mod common;

/// One schema check on the machine interface: every clap verb is listed with usage, data and
/// exit codes; the exit-code table, the env list and the envelope are there; `error_kinds` is
/// exactly the table in `src/exit.rs`; the kinds list holds the coded kinds and a user recipe
/// from `JEVIFY_CONFIG_DIR`, and a bad recipe file lands in `kinds_error` without failing the
/// verb. A bad key file does not break a verb that needs no key.
#[test]
fn capabilities_publish_verbs_exit_codes_env_error_kinds_and_kinds() {
    use clap::CommandFactory;
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::write(
        dir.join("kinds.jsonl"),
        "{\"kind\":\"widget\",\"list\":[\"printf\",\"w1\\\\n\"],\"ordered\":true}\n",
    )
    .unwrap();
    let out = common::bin()
        .env("TYPESAFE_API_KEY_FILE", "/nonexistent/jevify-key")
        .env("JEVIFY_CONFIG_DIR", &dir)
        .args(["capabilities", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let d = &v["data"];
    let parser = jevify::cli::Cli::command();
    let verbs: Vec<_> = parser.get_subcommands().map(|c| c.get_name()).collect();
    let commands = d["commands"].as_array().unwrap();
    assert_eq!(
        commands
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        verbs
    );
    for command in commands {
        assert!(!command["usage"].as_str().unwrap().is_empty());
        assert!(!command["data"].as_str().unwrap().is_empty());
        let codes = command["exit"].as_array().unwrap();
        assert!(!codes.is_empty() && codes.iter().all(|code| code.is_u64()));
    }
    assert_eq!(d["exit_codes"].as_array().unwrap().len(), 9);
    let env = d["env"].as_array().unwrap();
    assert!(env.iter().any(|e| e["name"] == "TYPESAFE_API_KEY"));
    assert!(env.iter().any(|e| e["name"] == "JEVIFY_CONFIG_DIR"));
    assert_eq!(
        d["envelope"]["branch_on"],
        "exit_code, which equals the process exit code; then read data"
    );
    let published: Vec<(String, i64)> = d["error_kinds"]
        .as_array()
        .expect("capabilities enumerates error.kind")
        .iter()
        .map(|e| {
            (
                e["kind"].as_str().unwrap().to_string(),
                e["exit"].as_i64().unwrap(),
            )
        })
        .collect();
    let table: Vec<(String, i64)> = jevify::exit::JevifyError::KINDS
        .iter()
        .map(|(kind, exit)| (kind.to_string(), i64::from(exit.code())))
        .collect();
    assert_eq!(published, table, "capabilities publishes exit.rs's table");
    let kinds = d["kinds"].as_array().unwrap();
    let names: Vec<&str> = kinds.iter().map(|k| k["name"].as_str().unwrap()).collect();
    for name in [
        "-", "branch", "commit", "file", "dir", "tool", "pod", "widget", "one", "flag",
    ] {
        assert!(names.contains(&name), "{name}");
    }
    let widget = kinds.iter().find(|k| k["name"] == "widget").unwrap();
    assert_eq!(widget["origin"], "user");
    assert_eq!(widget["list"], serde_json::json!(["printf", "w1\\n"]));
    assert!(d["kinds_error"].is_null());

    std::fs::write(dir.join("kinds.jsonl"), "{\"kind\":\"widget\"}\n").unwrap();
    let out = common::bin()
        .env("JEVIFY_CONFIG_DIR", &dir)
        .args(["capabilities", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let kinds = v["data"]["kinds"].as_array().unwrap();
    assert!(kinds.iter().any(|k| k["name"] == "pod"));
    assert!(kinds.iter().all(|k| k["name"] != "widget"));
    assert!(
        v["data"]["kinds_error"]
            .as_str()
            .unwrap()
            .contains("line 1"),
        "{}",
        v["data"]["kinds_error"]
    );
}

/// The base URL is pinned: a redirect is exit 4 and the target is never contacted.
#[tokio::test]
async fn health_never_follows_redirects() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let target = MockServer::start().await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", target.uri()))
        .expect(1)
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let out = tokio::task::spawn_blocking(move || {
        common::bin()
            .env("TYPESAFE_API_KEY", "test-key")
            .env("JEVIFY_BASE_URL", endpoint)
            .args(["health", "--json"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(4));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], 4);
    assert!(target.received_requests().await.unwrap().is_empty());
}

/// Only the backend that needs a key can fail for want of one (`tests/classifier.rs` covers
/// the keyless backend); a reachable backend is exit 0 without an inference request.
#[tokio::test(flavor = "multi_thread")]
async fn health_is_5_without_a_key_and_0_against_a_reachable_backend() {
    common::bin()
        .env("JEVIFY_BACKEND", "typesafe")
        .args(["health", "--json"])
        .assert()
        .code(5);
    let server = common::mock(common::FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.8,
    })
    .await;
    let out = tokio::task::spawn_blocking(move || {
        common::jevify(&server)
            .args(["health", "--json"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["meta"]["requests"], 0);
}

#[test]
fn robot_docs_topics() {
    for t in ["guide", "commands", "exit-codes", "examples", "privacy"] {
        common::bin().args(["robot-docs", t]).assert().success();
    }
    common::bin().args(["robot-docs", "nope"]).assert().code(2);
}
