use std::sync::Mutex;
use std::sync::atomic::AtomicU32;

#[derive(Debug)]
pub struct Stats {
    /// The instant the verb started: `Config::load` builds the `Stats` before any evidence is
    /// read, so the overall deadline counts from here, not from the first request.
    pub started: Instant,
    pub cache_hits: AtomicU32,
    pub model: Mutex<Option<String>>,
    /// `x-typesafe-request-id` of the last response seen, success or failure (surfaced in `meta`).
    pub request_id: Mutex<Option<String>>,
    telemetry: Mutex<crate::output::Telemetry>,
    /// The scores of every decision the verb made, in decision order (surfaced in `meta`).
    gates: Mutex<Vec<crate::output::Gate>>,
    /// Round one of every tournament the verb ran, in decision order (surfaced in `meta`).
    round_one: Mutex<Vec<crate::output::RoundOne>>,
}

#[derive(Clone, Copy)]
pub(crate) enum AttemptKind {
    Inference,
    Health,
    Prewarm,
    Semantic,
}

impl AttemptKind {
    fn counts(self, t: &mut crate::output::Telemetry) -> &mut crate::output::AttemptCounts {
        match self {
            Self::Inference => &mut t.inference_posts,
            Self::Health => &mut t.health_gets,
            Self::Prewarm => &mut t.prewarm_gets,
            Self::Semantic => &mut t.semantic_calls,
        }
    }
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            cache_hits: AtomicU32::default(),
            model: Mutex::default(),
            request_id: Mutex::default(),
            telemetry: Mutex::default(),
            gates: Mutex::default(),
            round_one: Mutex::default(),
        }
    }
}

impl Stats {
    pub fn telemetry(&self) -> crate::output::Telemetry {
        self.telemetry.lock().unwrap().clone()
    }

    /// Records the scores of one decision at its gate.
    pub fn gate(&self, gate: crate::output::Gate) {
        self.gates.lock().unwrap().push(gate);
    }

    pub fn gates(&self) -> Vec<crate::output::Gate> {
        self.gates.lock().unwrap().clone()
    }

    /// Records round one of a tournament: what the shortlist already computed, no request.
    pub fn round_one(&self, round: crate::output::RoundOne) {
        self.round_one.lock().unwrap().push(round);
    }

    pub fn rounds_one(&self) -> Vec<crate::output::RoundOne> {
        self.round_one.lock().unwrap().clone()
    }

    pub(crate) fn start(self: &Arc<Self>, kind: AttemptKind) -> AttemptGuard {
        let mut t = self.telemetry.lock().unwrap();
        let counts = kind.counts(&mut t);
        counts.attempted += 1;
        counts.in_flight += 1;
        if matches!(kind, AttemptKind::Inference) {
            let usage = &mut t.usage;
            for usage in [&mut usage.input_tokens, &mut usage.output_tokens] {
                usage.unknown_attempts += 1;
                usage.complete = false;
            }
        }
        AttemptGuard {
            stats: self.clone(),
            kind,
            finished: false,
        }
    }

    fn record_usage(&self, raw: &[u8]) {
        let value: serde_json::Value = serde_json::from_slice(raw).unwrap_or_default();
        let mut t = self.telemetry.lock().unwrap();
        for field in ["input_tokens", "output_tokens"] {
            let Some(tokens) = value["usage"][field].as_u64() else {
                continue;
            };
            let usage = if field == "input_tokens" {
                &mut t.usage.input_tokens
            } else {
                &mut t.usage.output_tokens
            };
            // An unrepresentable subtotal must not wrap or poison the cancellation ledger.
            let Some(subtotal) = usage.reported_subtotal.checked_add(tokens) else {
                continue;
            };
            usage.reported_subtotal = subtotal;
            usage.reported_attempts += 1;
            usage.unknown_attempts -= 1;
            usage.complete = usage.unknown_attempts == 0;
        }
    }
}

/// Dropping an unfinished future accounts for cancellation, including sibling cancellation.
pub(crate) struct AttemptGuard {
    stats: Arc<Stats>,
    kind: AttemptKind,
    finished: bool,
}

impl AttemptGuard {
    pub(crate) fn finish(&mut self, succeeded: bool) {
        let mut t = self.stats.telemetry.lock().unwrap();
        let counts = self.kind.counts(&mut t);
        counts.in_flight -= 1;
        if succeeded {
            counts.succeeded += 1;
        } else {
            counts.failed += 1;
        }
        self.finished = true;
    }
}

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        if !self.finished {
            let mut t = self.stats.telemetry.lock().unwrap();
            let counts = self.kind.counts(&mut t);
            counts.in_flight -= 1;
            counts.cancelled += 1;
        }
    }
}

struct RetrySleep<'a> {
    stats: &'a Stats,
    start: std::time::Instant,
}

impl Drop for RetrySleep<'_> {
    fn drop(&mut self) {
        let mut t = self.stats.telemetry.lock().unwrap();
        t.retry_waits += 1;
        t.retry_sleep_ms += self.start.elapsed().as_millis().min(u64::MAX as u128) as u64;
    }
}

use super::cache::{DiskCache, key as cache_key};
use super::{Questions, Response, classifier};
use crate::config::{Backend, Config};
use crate::exit::JevifyError;
use futures::future::try_join_all;
use futures::stream::FuturesUnordered;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Clone, Copy)]
enum Caller {
    Ask,
    Each,
}

pub struct Client {
    http: reqwest::Client,
    backend: Backend,
    base: String,
    /// `None` on classifier.dev, which needs no key at all.
    key: Option<String>,
    model: String,
    sem: Arc<Semaphore>,
    cache: Option<DiskCache>,
    stats: Arc<Stats>,
    /// The verb's overall deadline, `JEVIFY_DEADLINE` seconds after the verb started
    /// (`Stats::started`): the evidence read counts, a client built past it is refused, a
    /// request queued or in flight past it is cancelled, and no retry wait reaches beyond it.
    deadline: Instant,
    /// The budget the deadline was built from, for the message that names it.
    budget: Duration,
}

impl Client {
    pub fn new(cfg: &Config) -> Result<Self, JevifyError> {
        let base = crate::config::base_url(cfg.backend, Some(&cfg.base_url))?;
        let key = match cfg.backend {
            Backend::Typesafe => Some(cfg.api_key()?),
            Backend::Classifier => None,
        };
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("jevify/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| JevifyError::Unavailable(e.to_string()))?;
        let budget = crate::config::deadline()?;
        let client = Self {
            http,
            backend: cfg.backend,
            base,
            key,
            model: cfg.model.clone(),
            sem: Arc::new(Semaphore::new(cfg.concurrency)),
            cache: cfg.cache_dir.clone().and_then(|d| DiskCache::new(d).ok()),
            stats: cfg.stats.clone(),
            deadline: cfg.stats.started + budget,
            budget,
        };
        // Evidence read past the budget (a blocked stdin, a slow mount) ends the verb here,
        // before any request.
        client.check_deadline()?;
        Ok(client)
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// The overall budget, injected, counted from now: tests never touch the process
    /// environment, and a sub-second budget must not pay for the TLS setup of `new`.
    pub fn with_budget(mut self, budget: Duration) -> Self {
        self.deadline = Instant::now() + budget;
        self.budget = budget;
        self
    }

    /// The deadline error once the deadline has passed.
    fn check_deadline(&self) -> Result<(), JevifyError> {
        if Instant::now() >= self.deadline {
            return Err(self.deadline_error());
        }
        Ok(())
    }

    fn deadline_error(&self) -> JevifyError {
        let seconds = if self.budget.subsec_nanos() == 0 {
            self.budget.as_secs().to_string()
        } else {
            format!("{:.1}", self.budget.as_secs_f64())
        };
        JevifyError::Deadline(seconds)
    }

    /// Opens the TLS connection while local work runs; the pooled connection is reused by `ask`.
    /// Only `run` calls this: it reads its tool inventory (~1 s of CPU) before its first request,
    /// so the handshake overlaps real work. On the current-thread runtime the spawned task only
    /// progresses while the caller is parked in an `.await`, so the local work must go through
    /// `spawn_blocking` (see `run::load_tools`), or prewarm races `ask` and buys nothing.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn prewarm(&self) {
        let req = self.auth(self.http.get(format!(
            "{}{}",
            self.base,
            match self.backend {
                Backend::Typesafe => "/v1/models",
                Backend::Classifier => "/v1/health",
            }
        )));
        let stats = self.stats.clone();
        tokio::spawn(async move {
            let mut attempt = stats.start(AttemptKind::Prewarm);
            // Read the body too: hyper returns an HTTP/1.1 connection to the pool only once the
            // response is consumed, and the point of prewarm is that `ask` reuses it.
            let succeeded = match req.send().await {
                Ok(r) => {
                    let ok = r.status().as_u16() == 200;
                    r.bytes().await.is_ok() && ok
                }
                Err(_) => false,
            };
            attempt.finish(succeeded);
        });
    }

    /// The bearer token, on the backends that have one.
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }

    pub async fn ask(
        &self,
        state: &serde_json::Value,
        questions: &Questions,
    ) -> Result<Response, JevifyError> {
        let mut attempt = self.stats.start(AttemptKind::Semantic);
        self.stats.telemetry.lock().unwrap().semantic_questions += questions.len() as u64;
        let result = self.ask_inner(state, questions).await;
        attempt.finish(result.is_ok());
        result
    }

    async fn ask_inner(
        &self,
        state: &serde_json::Value,
        questions: &Questions,
    ) -> Result<Response, JevifyError> {
        let state = crate::input::redact_value(state);
        let questions = redact_questions(questions);
        // The cache key names the backend: the same questions get the same model but a
        // different wire shape, and an entry must never cross from one to the other.
        let canonical = serde_json::json!({
            "decision_contract": 4, "endpoint": self.base,
            "backend": self.backend.as_str(), "model": self.model, "state": state, "questions": questions
        });
        let k = cache_key(
            &serde_json::to_vec(&canonical).map_err(|e| JevifyError::Protocol(e.to_string()))?,
        );
        if let Some(hit) = self.cache.as_ref().and_then(|c| c.get(&k)) {
            hit.validate(&questions)?;
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            self.record_model(&hit.model);
            return Ok(hit);
        }
        let resp = match self.backend {
            Backend::Typesafe => self.ask_typesafe(&state, &questions).await?,
            Backend::Classifier => self.ask_classifier(&state, &questions).await?,
        };
        resp.validate(&questions)?;
        self.record_model(&resp.model);
        if let Some(c) = &self.cache {
            c.put(&k, &resp);
        }
        Ok(resp)
    }

    fn record_model(&self, model: &str) {
        let mut seen = self.stats.model.lock().unwrap();
        super::join_models(seen.get_or_insert_with(String::new), model);
    }

    /// Batches finish as available; responses inside each batch retain record order.
    /// Dropping the stream cancels its pending requests.
    pub fn ask_each<'a>(
        &'a self,
        records: &'a [String],
        questions: &'a Questions,
    ) -> impl futures::Stream<Item = Result<(usize, Vec<Response>), JevifyError>> + 'a {
        let size = batch_size(self.backend, questions.len());
        records
            .chunks(size)
            .enumerate()
            .map(move |(index, records)| async move {
                let mut attempt = self.stats.start(AttemptKind::Semantic);
                self.stats.telemetry.lock().unwrap().semantic_questions +=
                    (records.len() * questions.len()) as u64;
                let result = self.ask_batch(records, questions).await;
                attempt.finish(result.is_ok());
                result.map(|responses| (index, responses))
            })
            .collect::<FuturesUnordered<_>>()
    }

    async fn ask_batch(
        &self,
        records: &[String],
        questions: &Questions,
    ) -> Result<Vec<Response>, JevifyError> {
        if questions.is_empty() {
            return Err(JevifyError::Usage("ask_each requires a question".into()));
        }
        let records: Vec<String> = records
            .iter()
            .map(|record| {
                let redacted = crate::input::redact(record);
                let mut units = 0;
                redacted
                    .chars()
                    .take_while(|c| {
                        units += c.len_utf16();
                        units <= classifier::MAX_INPUT_CHARS
                    })
                    .collect()
            })
            .collect();
        let questions = redact_questions(questions);
        let expanded = record_questions(records.len(), &questions);
        let canonical = serde_json::json!({
            "decision_contract": 4, "endpoint": self.base,
            "backend": self.backend.as_str(), "model": self.model,
            "records": records, "questions": questions
        });
        let k = cache_key(
            &serde_json::to_vec(&canonical).map_err(|e| JevifyError::Protocol(e.to_string()))?,
        );
        if let Some(hit) = self.cache.as_ref().and_then(|c| c.get(&k)) {
            hit.validate(&expanded)?;
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            self.record_model(&hit.model);
            return unpack_batch(hit, records.len(), &questions);
        }
        let response = match self.backend {
            Backend::Typesafe => {
                let items: Vec<_> = records
                    .iter()
                    .enumerate()
                    .map(|(i, text)| serde_json::json!({"id": i, "text": text}))
                    .collect();
                let body = serde_json::json!({"model": self.model, "state": {"items": items}, "questions": expanded});
                let bytes =
                    serde_json::to_vec(&body).map_err(|e| JevifyError::Protocol(e.to_string()))?;
                let raw = self
                    .post(&format!("{}/v1/systemone", self.base), bytes, Caller::Each)
                    .await?;
                serde_json::from_slice::<Response>(&raw)
                    .map_err(|e| JevifyError::Protocol(e.to_string()))?
            }
            Backend::Classifier => {
                let body = classifier::request_body(&records, &questions)?;
                let bytes =
                    serde_json::to_vec(&body).map_err(|e| JevifyError::Protocol(e.to_string()))?;
                let raw = self
                    .post(&format!("{}/v1/classify", self.base), bytes, Caller::Each)
                    .await?;
                let responses = classifier::parse_each(&raw, &questions, records.len())?;
                let mut batch = Response {
                    model: String::new(),
                    answers: Default::default(),
                    usage: Default::default(),
                };
                for (i, response) in responses.into_iter().enumerate() {
                    super::join_models(&mut batch.model, &response.model);
                    for (id, answer) in response.answers {
                        batch.answers.insert(format!("{i}:{id}"), answer);
                    }
                    // Per-record provenance survives the batch cache without attributing a
                    // neighbouring record's model to this record.
                    batch.answers.insert(
                        format!("model:{i}"),
                        super::Answer {
                            choice: Some(response.model),
                            ..Default::default()
                        },
                    );
                }
                batch
            }
        };
        response.validate(&expanded)?;
        self.record_model(&response.model);
        if let Some(cache) = &self.cache {
            cache.put(&k, &response);
        }
        unpack_batch(response, records.len(), &questions)
    }

    async fn ask_typesafe(
        &self,
        state: &serde_json::Value,
        questions: &Questions,
    ) -> Result<Response, JevifyError> {
        let body =
            serde_json::json!({ "model": self.model, "state": state, "questions": questions });
        let bytes = serde_json::to_vec(&body).map_err(|e| JevifyError::Protocol(e.to_string()))?;
        let raw = self
            .post(&format!("{}/v1/systemone", self.base), bytes, Caller::Ask)
            .await?;
        serde_json::from_slice(&raw).map_err(|e| JevifyError::Protocol(e.to_string()))
    }

    /// classifier.dev takes at most 20 dimensions per request, so a larger `Questions` map goes
    /// out as concurrent requests, with answers merged in question order.
    async fn ask_classifier(
        &self,
        state: &serde_json::Value,
        questions: &Questions,
    ) -> Result<Response, JevifyError> {
        let url = format!("{}/v1/classify", self.base);
        let ids: Vec<&String> = questions.keys().collect();
        let mut merged = Response {
            model: String::new(),
            answers: std::collections::BTreeMap::new(),
            usage: super::Usage::default(),
        };
        let mut prepared = Vec::new();
        for chunk in ids.chunks(classifier::MAX_DIMENSIONS) {
            let part: Questions = chunk
                .iter()
                .map(|id| ((*id).clone(), questions[*id].clone()))
                .collect();
            let bytes = serde_json::to_vec(&classifier::request_body(
                &[classifier::item_text(state)],
                &part,
            )?)
            .map_err(|e| JevifyError::Protocol(e.to_string()))?;
            prepared.push((part, bytes));
        }
        let chunks = try_join_all(prepared.into_iter().map(|(part, bytes)| {
            let url = &url;
            async move {
                let raw = self.post(url, bytes, Caller::Ask).await?;
                Ok::<_, JevifyError>(classifier::parse_each(&raw, &part, 1)?.remove(0))
            }
        }))
        .await?;
        for r in chunks {
            super::join_models(&mut merged.model, &r.model);
            merged.answers.extend(r.answers);
        }
        Ok(merged)
    }

    /// One POST with the shared retry policy, returning the 200 body. Both backends answer
    /// errors the same way as far as jevify is concerned: auth, a rejected body, or something
    /// worth retrying. The whole exchange, permit wait and retry waits included, is cancelled
    /// at the overall deadline: exit 4, the message names the deadline.
    async fn post(
        &self,
        url: &str,
        bytes: Vec<u8>,
        caller: Caller,
    ) -> Result<Vec<u8>, JevifyError> {
        // Nothing is sent past the deadline: `timeout_at` polls the request once before the
        // timer, which would open the connection.
        self.check_deadline()?;
        let deadline = tokio::time::Instant::from_std(self.deadline);
        match tokio::time::timeout_at(deadline, self.post_within(url, bytes, caller)).await {
            Ok(result) => result,
            Err(_) => Err(self.deadline_error()),
        }
    }

    async fn post_within(
        &self,
        url: &str,
        bytes: Vec<u8>,
        caller: Caller,
    ) -> Result<Vec<u8>, JevifyError> {
        let _permit = self.sem.acquire().await.expect("semaphore open");
        let mut last = String::new();
        let mut wait = None;
        for attempt in 0..4u32 {
            if attempt > 0 {
                // The server's `retry-after(-ms)` replaces the backoff; it never adds to it.
                let backoff = Duration::from_millis(250 * 2u64.pow(attempt - 1));
                let wait = wait.take().unwrap_or(backoff);
                // A wait that would end past the deadline is not started: nothing is sent
                // before the server's named wait ends, and the sum of waits stays under the
                // deadline.
                if Instant::now() + wait >= self.deadline {
                    return Err(self.deadline_error());
                }
                let sleep = RetrySleep {
                    stats: &self.stats,
                    start: Instant::now(),
                };
                tokio::time::sleep(wait).await;
                drop(sleep);
                self.stats.telemetry.lock().unwrap().retry_sends += 1;
            }
            let mut accounting = self.stats.start(AttemptKind::Inference);
            let res = self
                .auth(self.http.post(url))
                .header("content-type", "application/json")
                .body(bytes.clone())
                .send()
                .await;
            let r = match res {
                Ok(r) => r,
                Err(e) => {
                    accounting.finish(false);
                    last = e.to_string();
                    if e.is_timeout() || e.is_connect() {
                        continue;
                    }
                    return Err(JevifyError::Unavailable(last));
                }
            };
            // Stored now, and again by the two arms that await an error body before returning:
            // under `try_join_all` a sibling window's response can land during that await and
            // would otherwise replace the failing request's id.
            let rid = r
                .headers()
                .get("x-typesafe-request-id")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            if rid.is_some() {
                *self.stats.request_id.lock().unwrap() = rid.clone();
            }
            let status = r.status().as_u16();
            let response_wait = retry_after(r.headers());
            let body = match r.bytes().await {
                Ok(body) => body.to_vec(),
                Err(e) if status == 200 => {
                    accounting.finish(false);
                    return Err(JevifyError::Protocol(e.to_string()));
                }
                // An unreadable error body cannot replace the status's auth/input/retry
                // classification. Its usage remains unknown.
                Err(_) => Vec::new(),
            };
            self.stats.record_usage(&body);
            accounting.finish(status == 200);
            // 413/422 = the request body was rejected: in practice state over the token budget,
            // occasionally a malformed request (a jevify bug). An input problem (exit 6), not an
            // outage (exit 4); the error kind and hint keep the two readings apart.
            // classifier.dev says the same thing with 400 (`input_too_long`, `too_many_labels`,
            // `too_many_decisions`, ...), and its `code` names which. Only there: a TypeSafe 400
            // keeps falling through to `api_protocol`.
            let body_rejected = matches!(status, 413 | 422)
                || (status == 400 && matches!(self.backend, Backend::Classifier));
            if matches!(status, 402 | 429) {
                if let Some(error) = spending_error(self.backend, &body) {
                    return Err(error);
                }
            }
            match status {
                200 => {
                    return Ok(body);
                }
                401 | 403 => return Err(JevifyError::BadKey(status)),
                402 => {
                    let text = String::from_utf8_lossy(&body);
                    if rid.is_some() {
                        *self.stats.request_id.lock().unwrap() = rid;
                    }
                    return Err(JevifyError::Unavailable(format!(
                        "HTTP 402: {}",
                        rejection_message(&text)
                    )));
                }
                _ if body_rejected => {
                    let text = String::from_utf8_lossy(&body);
                    if rid.is_some() {
                        *self.stats.request_id.lock().unwrap() = rid;
                    }
                    return Err(JevifyError::RejectedRequest(
                        status,
                        rejection_message(&text),
                    ));
                }
                // 408 request timeout, 429 rate limit, 5xx (incl. 529 overloaded): retry after the
                // server's wait if it named one, else with backoff.
                408 | 429 | 500..=599 => {
                    let parsed: serde_json::Value =
                        serde_json::from_slice(&body).unwrap_or_default();
                    let code = if status == 429 {
                        parsed["code"].as_str()
                    } else {
                        None
                    };
                    let policy = if status == 429 { caller } else { Caller::Ask };
                    wait = wait_decision(response_wait, code, policy)?;
                    last = format!("HTTP {status}");
                }
                _ => {
                    let text = String::from_utf8_lossy(&body);
                    if rid.is_some() {
                        *self.stats.request_id.lock().unwrap() = rid;
                    }
                    return Err(JevifyError::Protocol(format!(
                        "HTTP {status}: {}",
                        text.chars().take(300).collect::<String>()
                    )));
                }
            }
        }
        Err(JevifyError::Unavailable(last))
    }
}

fn spending_error(backend: Backend, body: &[u8]) -> Option<JevifyError> {
    let parsed: serde_json::Value = serde_json::from_slice(body).ok()?;
    let parts = [
        &parsed,
        &parsed["detail"],
        &parsed["error"],
        &parsed["detail"]["error"],
    ];
    let has_code = |code| {
        parts.iter().any(|part| {
            ["error_type", "code"]
                .iter()
                .any(|key| part[*key].as_str() == Some(code))
        })
    };
    let message = parts
        .iter()
        .flat_map(|part| {
            [
                part.as_str(),
                part["message"].as_str(),
                part["error"].as_str(),
            ]
            .into_iter()
            .flatten()
        })
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    match backend {
        Backend::Typesafe
            if has_code("billing_error")
                || message.contains("no available typesafe api credits") =>
        {
            Some(JevifyError::quota_exhausted(
                "TypeSafe account has no credits",
                "add credits in the TypeSafe console, or set TYPESAFE_API_KEY_FILE to a funded account's key",
            ))
        }
        Backend::Classifier
            if has_code("free_ip_daily_budget")
                || (message.contains("budget")
                    && (message.contains("daily")
                        || message.contains("per day")
                        || message.contains("per utc day"))
                    && !message.contains("global")
                    && !message.contains("across everyone")
                    && ["spent", "exhaust", "exceed", "reached", "used up"]
                        .iter()
                        .any(|word| message.contains(word))) =>
        {
            Some(JevifyError::quota_exhausted(
                "the free per-IP budget is spent until 00:00 UTC; set TYPESAFE_API_KEY_FILE",
                "classifier.dev allows $0.50 per IP per UTC day; wait until 00:00 UTC or set TYPESAFE_API_KEY_FILE",
            ))
        }
        Backend::Classifier if has_code("request_spending_limit") => {
            Some(JevifyError::InputTooLarge(
                "the request exceeds classifier.dev's $0.01 free per-request spending limit".into(),
            ))
        }
        _ => None,
    }
}

/// Records per `ask_each` request. classifier.dev takes no key, so every request stays under
/// its keyless spending limit ([`classifier::KEYLESS_DECISIONS`] decisions); TypeSafe shares
/// one state between 20 records.
pub fn batch_size(backend: Backend, questions: usize) -> usize {
    match backend {
        Backend::Classifier => (classifier::KEYLESS_DECISIONS / questions.max(1)).max(1),
        Backend::Typesafe => 20,
    }
}

/// The wait the server asked for: `retry-after-ms` (milliseconds) beats `retry-after` (whole
/// seconds; an HTTP-date is ignored). `None` means use the backoff.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let num = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    num("retry-after-ms")
        .map(Duration::from_millis)
        .or_else(|| num("retry-after").map(Duration::from_secs))
}

fn wait_decision(
    wait: Option<Duration>,
    code: Option<&str>,
    caller: Caller,
) -> Result<Option<Duration>, JevifyError> {
    if code == Some("rate_limit_day") {
        return Err(JevifyError::Unavailable(
            "daily quota of the free backend reached".into(),
        ));
    }
    match caller {
        Caller::Ask => Ok(wait.map(|d| d.min(Duration::from_secs(10)))),
        Caller::Each if wait.is_some_and(|d| d > Duration::from_secs(60)) => {
            Err(JevifyError::Unavailable(format!(
                "{}: Retry-After exceeds 60 seconds",
                code.unwrap_or("HTTP 429")
            )))
        }
        Caller::Each => Ok(wait),
    }
}

fn redact_questions(questions: &Questions) -> Questions {
    let mut questions = questions.clone();
    for question in questions.values_mut() {
        match question {
            super::Question::Noul {
                instructions,
                criteria,
            } => {
                *instructions = crate::input::redact(instructions);
                if let Some(c) = criteria {
                    c.yes = crate::input::redact(&c.yes);
                    c.no = crate::input::redact(&c.no);
                }
            }
            super::Question::Choice {
                instructions,
                criteria,
            } => {
                *instructions = crate::input::redact(instructions);
                for text in criteria.values_mut().flatten() {
                    *text = crate::input::redact(text);
                }
            }
        }
    }
    questions
}

fn record_questions(count: usize, questions: &Questions) -> Questions {
    (0..count)
        .flat_map(|i| {
            questions.iter().map(move |(id, question)| {
                let mut question = question.clone();
                let (super::Question::Noul { instructions, .. }
                | super::Question::Choice { instructions, .. }) = &mut question;
                *instructions = format!("Judge record id {i} alone. {instructions}");
                (format!("{i}:{id}"), question)
            })
        })
        .collect()
}

fn unpack_batch(
    mut batch: Response,
    count: usize,
    questions: &Questions,
) -> Result<Vec<Response>, JevifyError> {
    (0..count)
        .map(|i| {
            let mut answers = std::collections::BTreeMap::new();
            for id in questions.keys() {
                let answer = batch.answers.remove(&format!("{i}:{id}")).ok_or_else(|| {
                    JevifyError::Protocol(format!("missing record {i} answer `{id}`"))
                })?;
                answers.insert(id.clone(), answer);
            }
            let model = batch
                .answers
                .remove(&format!("model:{i}"))
                .and_then(|a| a.choice)
                .unwrap_or_else(|| batch.model.clone());
            Ok(Response {
                model,
                answers,
                usage: super::Usage::default(),
            })
        })
        .collect()
}

/// The readable part of a rejected request's body. TypeSafe answers 422 in FastAPI's shape,
/// `{"detail":[{"loc":["body","state"],"msg":"..."}]}`, which becomes `state: ...`;
/// classifier.dev answers 400 as `{"error":"...","code":"..."}`, which becomes `code: ...`;
/// any other body is passed through. Clipped to 300 chars.
fn rejection_message(text: &str) -> String {
    if let Some(m) = classifier::error_message(text) {
        return m.chars().take(300).collect();
    }
    let flattened = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            let parts: Vec<String> = v
                .get("detail")?
                .as_array()?
                .iter()
                .filter_map(|e| {
                    let msg = e.get("msg")?.as_str()?;
                    let path: Vec<String> = e
                        .get("loc")
                        .and_then(|l| l.as_array())
                        .map(|l| {
                            l.iter()
                                .filter_map(|x| match x {
                                    serde_json::Value::String(s) if s != "body" => Some(s.clone()),
                                    serde_json::Value::Number(n) => Some(n.to_string()),
                                    _ => None,
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(if path.is_empty() {
                        msg.to_string()
                    } else {
                        format!("{}: {msg}", path.join("."))
                    })
                })
                .collect();
            (!parts.is_empty()).then(|| parts.join("; "))
        });
    flattened
        .unwrap_or_else(|| text.to_string())
        .chars()
        .take(300)
        .collect()
}
