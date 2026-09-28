//! The TypeSafe client: what leaves the process (redacted text, the pinned endpoint, no
//! redirects), the limits enforced before a request, HTTP statuses to exit codes and kinds,
//! retries, the deadline and the answer cache.
mod common;
use jevify::jev::client::Client;
use jevify::jev::{Question, Questions};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn one_noul() -> Questions {
    let mut q = Questions::new();
    q.insert("q".into(), Question::noul("Is it?"));
    q
}

fn fake() -> common::FakeJev {
    common::FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.8,
    }
}

#[tokio::test]
async fn all_semantic_text_is_redacted_without_changing_option_identity() {
    let server = common::mock(common::FakeJev {
        choose: |_, _, options| options[0].clone(),
        noul: |_, _| 0.8,
    })
    .await;
    let cfg = common::config(&server);
    let mut qs = Questions::new();
    qs.insert(
        "q".into(),
        Question::noul_with(
            "condition token=abcdefghijk",
            "secret=abcdefghijk",
            "password=abcdefghijk",
        ),
    );
    qs.insert(
        "pick".into(),
        Question::choice(
            "api_key=abcdefghijk",
            [
                (
                    "opaque_token_id".into(),
                    Some("description token=abcdefghijk".into()),
                ),
                ("NONE".into(), None),
            ]
            .into(),
        ),
    );
    let state = serde_json::json!({"request":"token=abcdefghijk", "filename":"token=abcdefghijk.txt", "nested":["token_expiry_seconds = 3600"]});
    Client::new(&cfg).unwrap().ask(&state, &qs).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(!body.to_string().contains("abcdefghijk"));
    assert!(
        body["questions"]["pick"]["criteria"]
            .get("opaque_token_id")
            .is_some()
    );
    assert_eq!(body["state"]["nested"][0], "token_expiry_seconds = 3600");
    assert_eq!(state["request"], "token=abcdefghijk");
}

#[tokio::test]
async fn records_are_redacted_and_the_cache_answers_the_redacted_input() {
    use futures::TryStreamExt;
    let server = common::mock(common::FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |instructions, state| {
            let id: usize = instructions
                .strip_prefix("Judge record id ")
                .and_then(|rest| rest.split(' ').next())
                .and_then(|id| id.parse().ok())
                .unwrap();
            state["items"][id]["text"]
                .as_str()
                .and_then(|text| text.split(' ').next())
                .and_then(|n| n.parse::<f64>().ok())
                .unwrap()
                / 100.0
        },
    })
    .await;
    let mut cfg = common::config(&server);
    cfg.cache_dir = Some(tempfile::tempdir().unwrap().keep());
    let qs = one_noul();
    let records: Vec<String> = (0..45).map(|i| format!("{i} token=abcdefghijk")).collect();
    let client = Client::new(&cfg).unwrap();
    let mut batches: Vec<_> = client.ask_each(&records, &qs).try_collect().await.unwrap();
    batches.sort_by_key(|b| b.0);
    for (i, response) in batches.into_iter().flat_map(|b| b.1).enumerate() {
        assert_eq!(response.noul("q").unwrap(), i as f64 / 100.0);
    }
    let requests = server.received_requests().await.unwrap();
    assert!(!requests.is_empty());
    for request in &requests {
        assert!(!String::from_utf8_lossy(&request.body).contains("abcdefghijk"));
    }
    let redacted: Vec<String> = records.iter().map(|r| jevify::input::redact(r)).collect();
    let second = Client::new(&cfg).unwrap();
    let _: Vec<_> = second.ask_each(&redacted, &qs).try_collect().await.unwrap();
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        requests.len()
    );
    assert_eq!(cfg.meta().cache_hits as usize, requests.len());
}

#[test]
fn the_endpoint_is_pinned_to_the_selected_backend_host() {
    for (key, endpoint, code) in [
        (true, "https://API.TypeSafe.AI", 0),
        (true, "https://classifier.dev", 2),
        (false, "https://api.typesafe.ai", 2),
        (false, "https://classifier.dev:443", 0),
        (true, "http://api.typesafe.ai", 2),
        (false, "https://unknown.example", 2),
        (true, "https://user:private-password@api.typesafe.ai", 2),
        (true, "http://localhost:1", 0),
        (true, "", 0),
        (false, "not a URL", 2),
    ] {
        let mut cmd = common::bin();
        if key {
            cmd.env("TYPESAFE_API_KEY", "test-key");
        }
        let out = cmd
            .env("JEVIFY_BASE_URL", endpoint)
            .args(["capabilities", "--json"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(code), "{endpoint}");
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["exit_code"], code);
        if code == 2 {
            assert_eq!(value["error"]["kind"], "usage");
        }
        assert!(!String::from_utf8_lossy(&out.stdout).contains("private-password"));
        assert!(!String::from_utf8_lossy(&out.stderr).contains("private-password"));
    }
}

#[tokio::test]
async fn redirects_never_receive_key_or_evidence() {
    let target = MockServer::start().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", target.uri()))
        .expect(1)
        .mount(&server)
        .await;
    let cfg = common::config(&server);
    assert!(
        Client::new(&cfg)
            .unwrap()
            .ask(&serde_json::json!("evidence"), &one_noul())
            .await
            .is_err()
    );
    assert!(target.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn records_are_clipped_to_the_utf16_cap_and_empty_input_sends_nothing() {
    use futures::TryStreamExt;
    for backend in [
        jevify::config::Backend::Classifier,
        jevify::config::Backend::Typesafe,
    ] {
        let server = match backend {
            jevify::config::Backend::Classifier => common::mock_classifier(fake()).await,
            jevify::config::Backend::Typesafe => common::mock(fake()).await,
        };
        let mut cfg = common::config(&server);
        cfg.backend = backend;
        let client = Client::new(&cfg).unwrap();
        let qs = one_noul();
        let empty: Vec<_> = client.ask_each(&[], &qs).try_collect().await.unwrap();
        assert!(empty.is_empty());
        assert!(server.received_requests().await.unwrap().is_empty());
        let records = vec!["😀".repeat(20_000)];
        let batches: Vec<_> = client.ask_each(&records, &qs).try_collect().await.unwrap();
        assert_eq!((batches.len(), batches[0].0, batches[0].1.len()), (1, 0, 1));
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        let item = match backend {
            jevify::config::Backend::Classifier => &body["items"][0],
            jevify::config::Backend::Typesafe => &body["state"]["items"][0]["text"],
        };
        assert_eq!(item.as_str().unwrap().encode_utf16().count(), 32_000);
        assert!(
            client
                .ask_each(&["x".into()], &Questions::new())
                .try_collect::<Vec<_>>()
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn classifier_preflights_every_chunk_before_any_post() {
    let server = MockServer::start().await;
    let mut cfg = common::config(&server);
    cfg.backend = jevify::config::Backend::Classifier;
    let mut qs: Questions = (0..21)
        .map(|i| (format!("q{i:02}"), Question::noul("is it?")))
        .collect();
    qs.insert("q20".into(), Question::noul("x".repeat(4001)));
    let error = Client::new(&cfg)
        .unwrap()
        .ask(&serde_json::json!("x"), &qs)
        .await
        .unwrap_err();
    assert_eq!(error.exit().code(), 6);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn malformed_answers_are_protocol_errors_and_never_cached() {
    for response in [
        ResponseTemplate::new(200).set_body_string("not json"),
        ResponseTemplate::new(200).set_body_json(serde_json::json!({"answers":{}})),
        ResponseTemplate::new(200).set_body_json(serde_json::json!({"answers":{"q":{"noul":1.1}}})),
        ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({"answers":{"q":{"noul":-0.1}}})),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(response)
            .expect(2)
            .mount(&server)
            .await;
        let mut cfg = common::config(&server);
        cfg.cache_dir = Some(tempfile::tempdir().unwrap().keep());
        let client = Client::new(&cfg).unwrap();
        for _ in 0..2 {
            let error = client
                .ask(&serde_json::json!("x"), &one_noul())
                .await
                .unwrap_err();
            assert_eq!((error.kind(), error.exit().code()), ("api_protocol", 4));
        }
    }
}

#[tokio::test]
async fn http_statuses_map_to_exit_codes_and_kinds() {
    let rejected = serde_json::json!({"detail":[{"loc":["body","state"],"msg":"too many tokens","type":"value_error"}]});
    for (status, exit, kind, retried) in [
        (401, 5, "bad_api_key", false),
        (403, 5, "bad_api_key", false),
        (402, 4, "api_unavailable", false),
        (413, 6, "api_rejected_request", false),
        (422, 6, "api_rejected_request", false),
        (418, 4, "api_protocol", false),
        (503, 4, "api_unavailable", true),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("retry-after-ms", "0")
                    .set_body_json(rejected.clone()),
            )
            .mount(&server)
            .await;
        let cfg = common::config(&server);
        let error = Client::new(&cfg)
            .unwrap()
            .ask(&serde_json::json!("x"), &one_noul())
            .await
            .unwrap_err();
        assert_eq!(
            (error.exit().code(), error.kind()),
            (exit, kind),
            "HTTP {status}: {error}"
        );
        assert!(!error.hint().is_empty(), "HTTP {status}");
        let requests = server.received_requests().await.unwrap().len();
        assert_eq!(requests > 1, retried, "HTTP {status}: {requests} requests");
    }
}

#[tokio::test]
async fn spending_errors_distinguish_quota_from_request_size_without_retries() {
    use futures::TryStreamExt;
    use jevify::config::Backend::{Classifier, Typesafe};
    use serde_json::json;
    for (backend, status, body, kind, exit, message, hint) in [
        (
            Typesafe,
            402,
            json!({"detail":{"error_type":"billing_error","message":"no available TypeSafe API credits"}}),
            "quota_exhausted",
            4,
            "TypeSafe account has no credits",
            "credits",
        ),
        (
            Typesafe,
            402,
            json!({"code":"billing_error"}),
            "quota_exhausted",
            4,
            "TypeSafe account has no credits",
            "credits",
        ),
        (
            Typesafe,
            402,
            json!({"detail":"no available TypeSafe API credits"}),
            "quota_exhausted",
            4,
            "TypeSafe account has no credits",
            "credits",
        ),
        (
            Classifier,
            429,
            json!({"code":"free_ip_daily_budget","error":"budget spent"}),
            "quota_exhausted",
            4,
            "00:00 UTC",
            "$0.50",
        ),
        (
            Classifier,
            429,
            json!({"detail":{"error_type":"free_ip_daily_budget"}}),
            "quota_exhausted",
            4,
            "00:00 UTC",
            "TYPESAFE_API_KEY_FILE",
        ),
        (
            Classifier,
            402,
            json!({"code":"request_spending_limit","error":"The daily per-IP budget is spent"}),
            "quota_exhausted",
            4,
            "00:00 UTC",
            "$0.50",
        ),
        (
            Classifier,
            402,
            json!({"message":"Daily IP budget exhausted"}),
            "quota_exhausted",
            4,
            "00:00 UTC",
            "TYPESAFE_API_KEY_FILE",
        ),
        (
            Classifier,
            402,
            json!({"code":"request_spending_limit","message":"Daily budget exhausted for this IP"}),
            "quota_exhausted",
            4,
            "00:00 UTC",
            "$0.50",
        ),
        (
            Classifier,
            402,
            json!({"code":"request_spending_limit","error":"Request exceeds $0.01 of provider cost"}),
            "input_too_large",
            6,
            "$0.01",
            "filter the input",
        ),
        (
            Classifier,
            402,
            json!({"error_type":"request_spending_limit","message":"Request too expensive"}),
            "input_too_large",
            6,
            "$0.01",
            "filter the input",
        ),
        (
            Classifier,
            402,
            json!({}),
            "api_unavailable",
            4,
            "HTTP 402",
            "retry later",
        ),
        (
            Typesafe,
            402,
            json!({"message":"unknown refusal"}),
            "api_unavailable",
            4,
            "HTTP 402",
            "retry later",
        ),
    ] {
        for batched in [false, true] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("retry-after-ms", "0")
                        .set_body_json(body.clone()),
                )
                .expect(1)
                .mount(&server)
                .await;
            let mut cfg = common::config(&server);
            cfg.backend = backend;
            let client = Client::new(&cfg).unwrap();
            let error = if batched {
                client
                    .ask_each(&["x".into()], &one_noul())
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap_err()
            } else {
                client.ask(&json!("x"), &one_noul()).await.unwrap_err()
            };
            assert_eq!((error.kind(), error.exit().code()), (kind, exit), "{body}");
            assert!(error.to_string().contains(message), "{error}");
            assert!(error.hint().contains(hint), "{}", error.hint());
            assert!(!error.hint().contains("20,000"));
            assert_eq!(server.received_requests().await.unwrap().len(), 1, "{body}");
        }
    }
}

#[tokio::test]
async fn a_429_is_retried_and_the_answer_is_cached() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"model":"m","answers":{"q":{"noul":0.7}},"usage":{"input_tokens":5}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let mut cfg = common::config(&server);
    cfg.cache_dir = Some(tempfile::tempdir().unwrap().keep());
    let client = Client::new(&cfg).unwrap();
    for _ in 0..2 {
        let response = client
            .ask(&serde_json::json!("x"), &one_noul())
            .await
            .unwrap();
        assert_eq!(response.noul("q").unwrap(), 0.7);
    }
    assert_eq!(cfg.meta().cache_hits, 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

/// Answers 429 `Retry-After: 1` for the first `limited` requests, then a valid decision, and
/// records when each request arrived.
struct TimedLimiter {
    arrivals: std::sync::Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
    limited: usize,
}

impl wiremock::Respond for TimedLimiter {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let mut arrivals = self.arrivals.lock().unwrap();
        arrivals.push(std::time::Instant::now());
        if arrivals.len() <= self.limited {
            ResponseTemplate::new(429).insert_header("Retry-After", "1")
        } else {
            ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"model":"m","answers":{"q":{"noul":0.7}},"usage":{"input_tokens":5,"output_tokens":1}}),
            )
        }
    }
}

#[tokio::test]
async fn no_request_is_sent_before_retry_after_ends() {
    let arrivals = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(TimedLimiter {
            arrivals: arrivals.clone(),
            limited: 2,
        })
        .mount(&server)
        .await;
    let cfg = common::config(&server);
    let response = Client::new(&cfg)
        .unwrap()
        .ask(&serde_json::json!("x"), &one_noul())
        .await
        .unwrap();
    assert_eq!(response.noul("q").unwrap(), 0.7);
    let arrivals = arrivals.lock().unwrap();
    assert_eq!(arrivals.len(), 3);
    assert!(arrivals[1].duration_since(arrivals[0]) >= std::time::Duration::from_secs(1));
    assert!(arrivals[2].duration_since(arrivals[1]) >= std::time::Duration::from_secs(1));
}

#[tokio::test]
async fn a_deadline_shorter_than_the_wait_ends_without_another_request() {
    let arrivals = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(TimedLimiter {
            arrivals: arrivals.clone(),
            limited: usize::MAX,
        })
        .mount(&server)
        .await;
    let cfg = common::config(&server);
    let client = Client::new(&cfg)
        .unwrap()
        .with_budget(std::time::Duration::from_millis(900));
    let start = std::time::Instant::now();
    let error = client
        .ask(&serde_json::json!("x"), &one_noul())
        .await
        .unwrap_err();
    assert_eq!((error.exit().code(), error.kind()), (4, "api_deadline"));
    // The 1 s wait would end past the deadline: it is not started and the verb ends at once.
    assert!(start.elapsed() < std::time::Duration::from_millis(900));
    assert_eq!(arrivals.lock().unwrap().len(), 1, "{error}");
}

#[tokio::test]
async fn the_deadline_cancels_a_request_in_flight() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(3)))
        .mount(&server)
        .await;
    let cfg = common::config(&server);
    let client = Client::new(&cfg)
        .unwrap()
        .with_budget(std::time::Duration::from_millis(200));
    let start = std::time::Instant::now();
    let error = client
        .ask(&serde_json::json!("x"), &one_noul())
        .await
        .unwrap_err();
    assert_eq!((error.exit().code(), error.kind()), (4, "api_deadline"));
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}
