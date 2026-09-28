//! The classifier.dev backend: same verbs, same envelope, same exit codes, no key.
mod common;

use common::{FakeJev, option_containing};
use jevify::config::Backend;
use jevify::jev::client::Client;
use jevify::jev::{Question, Questions};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn classifier_config(server: &MockServer) -> jevify::config::Config {
    let mut c = common::config(server);
    c.backend = Backend::Classifier;
    c.key = None;
    c
}

fn one_noul() -> Questions {
    let mut q = Questions::new();
    q.insert("q".into(), Question::noul("Is it?"));
    q
}

fn fake() -> FakeJev {
    FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.8,
    }
}

#[test]
fn backend_configuration_errors_carry_exit_code_kind_and_hint() {
    for (backend, model, code, kind) in [
        ("classifier", Some("jev-1.13.0"), 2, "usage"),
        ("ollama", None, 2, "usage"),
        ("typesafe", None, 5, "missing_api_key"),
    ] {
        let mut command = common::bin();
        command.env("JEVIFY_BACKEND", backend).arg("--json");
        if let Some(model) = model {
            command.args(["--model", model]);
        }
        let output = command
            .args(["pick", "x"])
            .write_stdin("a\nb\n")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{backend}");
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["kind"], kind, "{value}");
        assert!(
            !value["error"]["hint"]
                .as_str()
                .unwrap_or_default()
                .is_empty(),
            "{value}"
        );
    }
}

#[tokio::test]
async fn each_returns_every_record_in_order_under_the_keyless_batch_cap() {
    use futures::TryStreamExt;
    // The fake refuses any request over 1,000 classifications; both inputs need several.
    for (count, dimensions) in [(2500, 1), (100, 2)] {
        let server = common::mock_classifier(FakeJev {
            choose: |_, _, _| "NONE".into(),
            noul: |_, state| state.as_str().unwrap().parse::<f64>().unwrap() / 2500.0,
        })
        .await;
        let cfg = classifier_config(&server);
        let client = Client::new(&cfg).unwrap();
        let qs: Questions = (0..dimensions)
            .map(|i| (format!("q{i}"), Question::noul("Is it?")))
            .collect();
        let records: Vec<String> = (0..count).map(|i| i.to_string()).collect();
        let mut batches: Vec<_> = client.ask_each(&records, &qs).try_collect().await.unwrap();
        batches.sort_by_key(|b| b.0);
        let responses: Vec<_> = batches.into_iter().flat_map(|b| b.1).collect();
        assert_eq!(responses.len(), count);
        for (i, response) in responses.iter().enumerate() {
            for id in qs.keys() {
                assert!((response.noul(id).unwrap() - i as f64 / 2500.0).abs() < 1e-12);
            }
        }
        assert!(server.received_requests().await.unwrap().len() > 1);
    }
}

#[tokio::test]
async fn quota_refusals_end_at_exit_4_and_a_short_minute_wait_is_retried() {
    use futures::TryStreamExt;
    for (code, seconds) in [
        ("rate_limit_day", 1),
        ("rate_limit_minute", 61),
        ("rate_limit_hour", 3600),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(FakeJev::quota(code, seconds))
            .expect(1)
            .mount(&server)
            .await;
        let cfg = classifier_config(&server);
        let client = Client::new(&cfg).unwrap();
        let error = client
            .ask_each(&["record".into()], &one_noul())
            .try_collect::<Vec<_>>()
            .await
            .unwrap_err();
        assert_eq!(
            (error.exit().code(), error.kind()),
            (4, "api_unavailable"),
            "{code}: {error}"
        );
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(FakeJev::quota("rate_limit_minute", 1))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(common::FakeClassifier(fake()))
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    let cfg = classifier_config(&server);
    let client = Client::new(&cfg).unwrap();
    let _: Vec<_> = client
        .ask_each(&["record".into()], &one_noul())
        .try_collect()
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn pick_answers_without_a_key_on_the_named_and_the_default_backend() {
    let server = common::mock_classifier(FakeJev {
        choose: |_, s, o| option_containing(s, o, "invoice"),
        noul: |_, _| 0.9,
    })
    .await;
    // What a new user gets: `cargo install jevify` and nothing else. Only the base URL is
    // overridden, so the backend choice itself is the one jevify makes from an empty environment.
    let mut unnamed = common::bin();
    unnamed
        .env("JEVIFY_BASE_URL", server.uri())
        .env("JEVIFY_NO_CACHE", "1");
    for mut c in [common::jevify_classifier(&server), unnamed] {
        let out = tokio::task::spawn_blocking(move || {
            c.args(["--json", "pick", "the bill"])
                .write_stdin("notes.txt\ninvoice-march.pdf\nphoto.jpg\n")
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(out.status.code(), Some(0));
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["data"]["matches"][0]["text"], "invoice-march.pdf");
        assert_eq!(v["meta"]["backend"], "classifier");
        assert_eq!(v["meta"]["model"], "jev-fake");
    }
}

#[tokio::test]
async fn questions_past_the_dimension_limit_are_split_across_requests() {
    // The fake refuses any request over 20 dimensions.
    let server = common::mock_classifier(FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.75,
    })
    .await;
    let qs: Questions = (0..41)
        .map(|i| (format!("q{i:03}"), Question::noul("Is it?")))
        .collect();
    let r = Client::new(&classifier_config(&server))
        .unwrap()
        .ask(&serde_json::json!({ "x": 1 }), &qs)
        .await
        .unwrap();
    for id in qs.keys() {
        assert_eq!(r.noul(id).unwrap(), 0.75);
    }
    assert!(server.received_requests().await.unwrap().len() > 1);
}

#[tokio::test]
async fn a_rejected_body_is_an_input_error_and_an_outage_is_retried_then_unavailable() {
    for (response, exit, kind, retried) in [
        (
            ResponseTemplate::new(400).set_body_json(
                serde_json::json!({"error":"input exceeds 32000 characters","code":"input_too_long"}),
            ),
            6,
            "api_rejected_request",
            false,
        ),
        // 502 `typesafe` is what classifier.dev answers when Jev itself is down.
        (
            ResponseTemplate::new(502)
                .insert_header("retry-after-ms", "0")
                .set_body_json(serde_json::json!({"error":"provider failed","code":"typesafe"})),
            4,
            "api_unavailable",
            true,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(response)
            .mount(&server)
            .await;
        let error = Client::new(&classifier_config(&server))
            .unwrap()
            .ask(&serde_json::json!("x"), &one_noul())
            .await
            .unwrap_err();
        assert_eq!((error.exit().code(), error.kind()), (exit, kind), "{error}");
        assert_eq!(server.received_requests().await.unwrap().len() > 1, retried);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_window_never_offers_more_options_than_the_service_accepts() {
    // 250 lines is over classifier.dev's 100-label limit; the fake asserts the limit on every
    // request.
    let server = common::mock_classifier(FakeJev {
        choose: |_, s, o| option_containing(s, o, "invoice"),
        noul: |_, _| 0.9,
    })
    .await;
    let mut lines: Vec<String> = (0..250).map(|i| format!("file-{i:03}.txt")).collect();
    lines.push("invoice-march.pdf".into());
    let mut c = common::jevify_classifier(&server);
    let out = tokio::task::spawn_blocking(move || {
        c.args(["--json", "pick", "the bill"])
            .write_stdin(lines.join("\n"))
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["matches"][0]["text"], "invoice-march.pdf");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_typesafe_key_never_reaches_the_keyless_backend() {
    let server = common::mock_classifier(FakeJev {
        choose: |_, s, o| option_containing(s, o, "invoice"),
        noul: |_, _| 0.9,
    })
    .await;
    let mut c = common::jevify_classifier(&server);
    c.env("TYPESAFE_API_KEY", "test-key");
    let out = tokio::task::spawn_blocking(move || {
        c.args(["--json", "pick", "the bill"])
            .write_stdin("notes.txt\ninvoice-march.pdf\n")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let requests = server.received_requests().await.unwrap();
    assert!(!requests.is_empty());
    for request in &requests {
        assert!(request.headers.get("authorization").is_none());
        let leaked = request
            .headers
            .iter()
            .any(|(_, value)| value.to_str().unwrap_or_default().contains("test-key"))
            || String::from_utf8_lossy(&request.body).contains("test-key");
        assert!(!leaked, "{:?}", request.headers);
    }
}
