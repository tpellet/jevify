#![allow(dead_code)]
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Decides answers from the request. `choose` gets (instructions, state, options) and returns the option key.
/// `noul` gets (instructions, state) and returns P(yes).
#[derive(Clone)]
pub struct FakeJev {
    pub choose: fn(&str, &Value, &[String]) -> String,
    pub noul: fn(&str, &Value) -> f64,
}

pub type ProbabilityVector = fn(&str, &Value, &[String]) -> Vec<f64>;
pub type ModelName = fn(&str) -> String;

#[derive(Clone)]
pub struct ConfiguredFake {
    fake: FakeJev,
    probabilities: Option<ProbabilityVector>,
    model: ModelName,
}

impl FakeJev {
    pub fn with_probabilities(self, probabilities: ProbabilityVector) -> ConfiguredFake {
        ConfiguredFake {
            fake: self,
            probabilities: Some(probabilities),
            model: |_| "jev-fake".into(),
        }
    }

    pub fn with_model(self, model: ModelName) -> ConfiguredFake {
        ConfiguredFake {
            fake: self,
            probabilities: None,
            model,
        }
    }

    pub fn quota(code: &str, retry_after: u64) -> ResponseTemplate {
        ResponseTemplate::new(429)
            .insert_header("Retry-After", retry_after.to_string())
            .set_body_json(json!({"error": "Rate limit exceeded", "code": code}))
    }
}

impl ConfiguredFake {
    pub fn with_model(mut self, model: ModelName) -> Self {
        self.model = model;
        self
    }
}

pub trait FakeAnswers: Clone + Send + Sync + 'static {
    fn base(&self) -> &FakeJev;
    fn vector(&self, _instructions: &str, _state: &Value, _options: &[String]) -> Option<Vec<f64>> {
        None
    }
    fn model(&self, _instructions: &str) -> String {
        "jev-fake".into()
    }
}

impl FakeAnswers for FakeJev {
    fn base(&self) -> &FakeJev {
        self
    }
}

impl FakeAnswers for ConfiguredFake {
    fn base(&self) -> &FakeJev {
        &self.fake
    }
    fn vector(&self, instructions: &str, state: &Value, options: &[String]) -> Option<Vec<f64>> {
        self.probabilities.map(|f| f(instructions, state, options))
    }
    fn model(&self, instructions: &str) -> String {
        (self.model)(instructions)
    }
}

impl Respond for ConfiguredFake {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        typesafe_response(self, req)
    }
}

impl Respond for FakeJev {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        typesafe_response(self, req)
    }
}

fn typesafe_response(fake: &impl FakeAnswers, req: &Request) -> ResponseTemplate {
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    let state = &body["state"];
    let mut answers = serde_json::Map::new();
    let mut models = Vec::new();
    for (id, q) in body["questions"].as_object().unwrap() {
        let instr = q["instructions"].as_str().unwrap_or_default();
        let model = fake.model(instr);
        if !models.contains(&model) {
            models.push(model);
        }
        if q["type"] == "noul" {
            let labels = vec!["yes".into(), "no".into()];
            let probability = fake
                .vector(instr, state, &labels)
                .map(|v| {
                    assert_eq!(v.len(), 2);
                    v[0]
                })
                .unwrap_or_else(|| (fake.base().noul)(instr, state));
            answers.insert(id.clone(), json!({ "noul": probability }));
        } else {
            let opts: Vec<String> = q["criteria"].as_object().unwrap().keys().cloned().collect();
            let mut pick = (fake.base().choose)(instr, state, &opts);
            let rest = 0.1 / (opts.len().max(2) - 1) as f64;
            let vector = match fake.vector(instr, state, &opts) {
                Some(vector) => {
                    assert_eq!(vector.len(), opts.len());
                    if let Some((index, _)) =
                        vector.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))
                    {
                        pick = opts[index].clone();
                    }
                    vector
                }
                None => opts
                    .iter()
                    .map(|o| if *o == pick { 0.9 } else { rest })
                    .collect(),
            };
            let probs: serde_json::Map<String, Value> = opts
                .iter()
                .zip(vector)
                .map(|(o, p)| (o.clone(), json!(p)))
                .collect();
            answers.insert(
                id.clone(),
                json!({ "choice": pick, "probabilities": probs, "confidence": 0.8 }),
            );
        }
    }
    ResponseTemplate::new(200).set_body_json(json!({
            "model": models.join(", "), "answers": answers, "usage": { "input_tokens": 100, "output_tokens": 10 }
        }))
}

/// The same answers over classifier.dev's wire format: one result per item, one dimension per question,
/// `scores` per label. Built from a `FakeJev` so a test can point either backend at the same
/// expectations and compare.
#[derive(Clone)]
pub struct FakeClassifier<F = FakeJev>(pub F);

impl<F: FakeAnswers> Respond for FakeClassifier<F> {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let items = body["items"].as_array().unwrap();
        let n = body["dimensions"].as_object().unwrap().len();
        assert!(!items.is_empty() && items.len() * n <= 1000);
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            let text = item.as_str().unwrap_or_default();
            assert!(!text.is_empty() && text.encode_utf16().count() <= 32_000);
            assert!(body["dimensions"].to_string().encode_utf16().count() <= 16_000);
            // jevify serializes a non-string state as compact JSON and sends a string state as
            // itself; this reverses that so the shared `choose`/`noul` closures see the state.
            let state = match serde_json::from_str::<Value>(text) {
                Ok(v) if v.is_object() || v.is_array() => v,
                _ => Value::String(text.to_string()),
            };
            let mut dims = serde_json::Map::new();
            for (id, d) in body["dimensions"].as_object().unwrap() {
                assert!(!id.trim().is_empty() && id.encode_utf16().count() <= 64);
                let instr = d["instructions"].as_str().unwrap_or_default();
                assert!(instr.encode_utf16().count() <= 4_000);
                let labels: Vec<String> = d["labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| l.as_str().unwrap().to_string())
                    .collect();
                assert!(
                    labels.iter().all(
                        |label| !label.trim().is_empty() && label.encode_utf16().count() <= 200
                    )
                );
                assert!(
                    labels.len() >= 2 && labels.len() <= 100,
                    "classifier.dev takes 2..=100 labels, got {}",
                    labels.len()
                );
                // A Noul arrives as a two-label dimension whose first label is the `true` side;
                // every jevify Choice carries NONE, so that tells the two apart.
                let is_noul = labels.len() == 2 && !labels.iter().any(|l| l == "NONE");
                let (pick, top) = if is_noul {
                    let p = (self.0.base().noul)(instr, &state);
                    (labels[usize::from(p < 0.5)].clone(), p.max(1.0 - p))
                } else {
                    ((self.0.base().choose)(instr, &state, &labels), 0.9)
                };
                let rest = (1.0 - top) / (labels.len().max(2) - 1) as f64;
                let vector = self.0.vector(instr, &state, &labels);
                if let Some(v) = &vector {
                    assert_eq!(v.len(), labels.len());
                }
                let scores: serde_json::Map<String, Value> = labels
                    .iter()
                    .enumerate()
                    .map(|(i, l)| {
                        (
                            l.clone(),
                            json!(if let Some(v) = &vector {
                                v[i]
                            } else if *l == pick {
                                top
                            } else if is_noul {
                                1.0 - top
                            } else {
                                rest
                            }),
                        )
                    })
                    .collect();
                let (pick, top) = if vector.is_some() {
                    let (label, score) = scores
                        .iter()
                        .max_by(|a, b| a.1.as_f64().unwrap().total_cmp(&b.1.as_f64().unwrap()))
                        .unwrap();
                    (label.clone(), score.as_f64().unwrap())
                } else {
                    (pick, top)
                };
                dims.insert(
                    id.clone(),
                    json!({
                        "label": pick, "confidence": top, "scores": scores,
                        "model": self.0.model(instr), "ms": 1
                    }),
                );
            }
            results.push(json!({"dimensions": dims}));
        }
        assert!(
            n <= 20,
            "classifier.dev takes at most 20 dimensions, got {n}"
        );
        ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-fake",
            "results": results,
            "usage": { "items": items.len(), "dimensions": n, "classifications": items.len() * n }
        }))
    }
}

pub async fn mock(fake: impl Respond + 'static) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(fake)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[]})))
        .mount(&server)
        .await;
    server
}

/// A mock classifier.dev: the classify endpoint and the health probe.
pub async fn mock_classifier(fake: impl FakeAnswers) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/classify"))
        .respond_with(FakeClassifier(fake))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;
    server
}

/// The binary with every jevify variable removed. A developer's shell may export them
/// (`JEVIFY_THRESHOLD=2` turns even `capabilities` into exit 2, since `Config::load` runs for
/// every verb); the contract tests must not depend on it. Every raw binary invocation from
/// Task 3 onward starts here; `cli_basics.rs` (Task 1, env-immune by construction) and
/// `live.rs` (needs the inherited key) are the two raw `cargo_bin` exceptions.
pub fn bin() -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("jevify").unwrap();
    for var in [
        "TYPESAFE_API_KEY",
        "TYPESAFE_API_KEY_FILE",
        "JEVIFY_BACKEND",
        "JEVIFY_BASE_URL",
        "JEVIFY_THRESHOLD",
        "JEVIFY_MODEL",
        "JEVIFY_CONCURRENCY",
        "JEVIFY_DEADLINE",
        "JEVIFY_CACHE_DIR",
        "JEVIFY_NO_CACHE",
        "JEVIFY_PRICE_PER_MTOK",
        "JEVIFY_INVENTORY_FILE",
        "JEVIFY_STATUS_FILE",
    ] {
        c.env_remove(var);
    }
    // Removing the variable is not enough: jevify then reads the platform configuration
    // directory, which holds the developer's own recipes. A fresh empty directory per call,
    // kept on disk; a test with its own recipes overrides the variable after `bin()`.
    c.env("JEVIFY_CONFIG_DIR", tempfile::tempdir().unwrap().keep());
    c
}

/// The binary, pointed at the mock server, no cache, no key file.
pub fn jevify(server: &MockServer) -> assert_cmd::Command {
    let mut c = bin();
    c.env("JEVIFY_BASE_URL", server.uri())
        .env("JEVIFY_CACHE_DIR", tempfile::tempdir().unwrap().keep())
        .env("TYPESAFE_API_KEY", "test-key")
        .env("JEVIFY_NO_CACHE", "1");
    c
}

/// The binary, pointed at a mock classifier.dev, with no key at all.
pub fn jevify_classifier(server: &MockServer) -> assert_cmd::Command {
    let mut c = bin();
    c.env("JEVIFY_BACKEND", "classifier")
        .env("JEVIFY_CACHE_DIR", tempfile::tempdir().unwrap().keep())
        .env("JEVIFY_BASE_URL", server.uri())
        .env("JEVIFY_NO_CACHE", "1");
    c
}

/// A library `Config` for in-process tests, built literally so no test touches the process env.
pub fn config(server: &MockServer) -> jevify::config::Config {
    jevify::config::Config {
        backend: jevify::config::Backend::Typesafe,
        key: Some("test-key".into()),
        key_file: None,
        base_url: server.uri(),
        model: "jev-1.13.0".into(),
        threshold: 0.5,
        concurrency: 8,
        cache_dir: None,
        stats: std::sync::Arc::new(jevify::jev::client::Stats::default()),
    }
}

/// The option whose line text (from state.items) contains `needle`, else NONE.
pub fn option_containing(state: &Value, opts: &[String], needle: &str) -> String {
    let items = state["items"].as_array().cloned().unwrap_or_default();
    for it in items {
        let s = it.as_str().unwrap_or_default();
        if s.contains(needle) {
            if let Some(id) = s.strip_prefix('[').and_then(|r| r.split(']').next()) {
                if opts.iter().any(|o| o == id) {
                    return id.to_string();
                }
            }
        }
    }
    "NONE".into()
}
