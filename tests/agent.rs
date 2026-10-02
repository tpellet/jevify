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
    assert_eq!(
        d["exit_codes"].as_array().unwrap().len(),
        jevify::exit::Exit::ALL.len()
    );
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
    Mock::given(method("POST"))
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

/// Health requires a fresh classification on either backend, even with caching enabled.
#[tokio::test(flavor = "multi_thread")]
async fn health_is_5_without_a_key_and_0_after_uncached_classification() {
    common::bin()
        .env("JEVIFY_BACKEND", "typesafe")
        .args(["health", "--json"])
        .assert()
        .code(5);
    let fake = common::FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.8,
    };
    for classifier in [false, true] {
        let server = if classifier {
            common::mock_classifier(fake.clone()).await
        } else {
            common::mock(fake.clone()).await
        };
        let cache = tempfile::tempdir().unwrap().keep();
        for _ in 0..2 {
            let mut command = if classifier {
                common::jevify_classifier(&server)
            } else {
                common::jevify(&server)
            };
            command
                .env_remove("JEVIFY_NO_CACHE")
                .env("JEVIFY_CACHE_DIR", &cache);
            let out = tokio::task::spawn_blocking(move || {
                command.args(["health", "--json"]).output().unwrap()
            })
            .await
            .unwrap();
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(value["exit_code"], 0);
            assert_eq!(
                value["data"]["backend"],
                if classifier { "classifier" } else { "typesafe" }
            );
            assert_eq!(value["data"]["api"], "answered");
            assert_eq!(value["data"]["key_url"], jevify::config::KEY_URL);
            assert_eq!(value["data"]["model"], "jev-fake");
            assert!(value["data"]["latency_ms"].is_u64());
            assert_eq!(value["meta"]["requests"], 1);
            assert_eq!(value["meta"]["cache_hits"], 0);
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            2,
            "one real classification per health invocation"
        );
        assert!(requests.iter().all(|r| r.method == "POST"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn health_reports_billing_quota_and_auth_errors() {
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    for (backend, status, body, exit, kind) in [
        (
            "typesafe",
            402,
            json!({"detail":{"error_type":"billing_error","message":"no available TypeSafe API credits"}}),
            4,
            "quota_exhausted",
        ),
        (
            "classifier",
            402,
            json!({"code":"request_spending_limit","error":"The daily per-IP budget is spent"}),
            4,
            "quota_exhausted",
        ),
        (
            "classifier",
            429,
            json!({"code":"free_ip_daily_budget","error":"budget spent"}),
            4,
            "quota_exhausted",
        ),
        (
            "typesafe",
            401,
            json!({"error":"invalid key"}),
            5,
            "bad_api_key",
        ),
        (
            "typesafe",
            403,
            json!({"error":"invalid key"}),
            5,
            "bad_api_key",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let mut command = common::jevify(&server);
        command.env("JEVIFY_BACKEND", backend);
        let out = tokio::task::spawn_blocking(move || {
            command.args(["health", "--json"]).output().unwrap()
        })
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(out.status.code(), Some(exit), "{value}");
        assert_eq!(value["exit_code"], exit);
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["kind"], kind);
        let hint = value["error"]["hint"].as_str().unwrap();
        assert!(!hint.is_empty());
        if kind == "quota_exhausted" {
            assert!(hint.contains(jevify::config::KEY_URL), "{hint}");
        }
    }
}

#[test]
fn init_agents_prints_one_screen_of_instructions_in_human_and_json() {
    let human = common::bin().args(["init", "agents"]).output().unwrap();
    assert!(human.status.success());
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains("jevify capabilities --json"));
    assert!(text.contains("JEVIFY_STATUS_FILE"));
    assert!(text.lines().count() <= 25);
    assert!(text.split_whitespace().count() <= 250);
    for command in [
        "cargo test 2>&1 | jevify why",
        "jevify fill --dry-run -- git show '@{commit:what it did}'",
        "jevify pick --from commit",
        "jevify filter 'reports a failed assertion'",
        "jevify filter --label bug,feature,question",
        "jevify is 'reports a failure' < build.log",
        "cargo test -- --list 2>/dev/null | sed -n 's/: test$//p' | jevify fill -- cargo test '@{-:",
        "-- --exact",
    ] {
        assert!(text.contains(command), "{command}");
    }
    for contract in [
        "branch",
        "file",
        "pr",
        "ci-run",
        "Single-quote",
        "error.example",
        "data.shortlist",
        "quota_exhausted",
        "stop for today",
    ] {
        assert!(text.contains(contract), "{contract}");
    }
    let out = common::bin()
        .args(["init", "agents", "--robot"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["command"], "init");
    assert_eq!(value["exit_code"], 0);
    assert_eq!(value["data"]["script"], text);
    assert_eq!(String::from_utf8(out.stdout).unwrap().lines().count(), 1);
}

/// The replacement for tool routing uses the caller's inventory, enriches finalists without
/// executing them, and writes only the selected handle to human stdout.
#[tokio::test(flavor = "multi_thread")]
async fn tool_kind_uses_inventory_and_man_evidence_without_running_the_tool() {
    use std::os::unix::fs::PermissionsExt;
    let server = common::mock(common::FakeJev {
        choose: |_, state, options| common::option_containing(state, options, "macho-inspect"),
        noul: |_, _| 0.95,
    })
    .await;
    let dir = tempfile::tempdir().unwrap().keep();
    let inventory = dir.join("inventory.json");
    std::fs::write(
        &inventory,
        r#"[{"name":"macho-inspect","summary":"inspect binaries"}]"#,
    )
    .unwrap();
    for (name, body) in [
        ("macho-inspect", "printf started > sentinel"),
        (
            "man",
            "printf 'DESCRIPTION\\n    MAN-EVIDENCE: inspect Mach-O load commands.\\n'",
        ),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut command = common::jevify(&server);
    command
        .current_dir(&dir)
        .env("PATH", &dir)
        .env("JEVIFY_INVENTORY_FILE", &inventory);
    let out = tokio::task::spawn_blocking(move || {
        command
            .args(["pick", "--from", "tool", "inspect Mach-O load commands"])
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
    assert_eq!(out.stdout, b"macho-inspect\n");
    assert!(!dir.join("sentinel").exists());
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests.iter().any(|request| {
            request.method == "POST"
                && String::from_utf8_lossy(&request.body).contains("MAN-EVIDENCE")
        }),
        "finalists must carry manual evidence"
    );

    for contents in [Some("[]"), Some("not json"), None] {
        let path = if let Some(contents) = contents {
            std::fs::write(&inventory, contents).unwrap();
            inventory.clone()
        } else {
            dir.join("missing.json")
        };
        let out = common::bin()
            .env("JEVIFY_INVENTORY_FILE", path)
            .args(["pick", "--from", "tool", "inspect", "--json"])
            .output()
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let expected = if contents == Some("[]") { 3 } else { 6 };
        assert_eq!(out.status.code(), Some(expected), "{value}");
        assert_eq!(value["meta"]["requests"], 0);
        if expected == 6 {
            assert_eq!(value["error"]["kind"], "input");
        }
    }
}
