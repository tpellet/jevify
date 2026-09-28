use serde::Serialize;

/// Escape line breaks in status text without changing dry-run shell quoting.
pub fn status_escape(text: &str) -> String {
    text.replace('\r', "\\r").replace('\n', "\\n")
}

pub fn shell_quote(argv: &[std::ffi::OsString]) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut output = Vec::new();
    for (index, arg) in argv.iter().enumerate() {
        if index > 0 {
            output.push(b' ');
        }
        output.push(b'\'');
        for &byte in arg.as_bytes() {
            if byte == b'\'' {
                output.extend_from_slice(b"'\\''");
            } else {
                output.push(byte);
            }
        }
        output.push(b'\'');
    }
    output
}

#[derive(Serialize, Default, Debug, Clone)]
pub struct AttemptCounts {
    pub attempted: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub in_flight: u64,
}

#[derive(Serialize, Debug, Clone)]
pub struct TokenAccounting {
    pub reported_subtotal: u64,
    pub reported_attempts: u64,
    pub unknown_attempts: u64,
    pub complete: bool,
}

impl Default for TokenAccounting {
    fn default() -> Self {
        Self {
            reported_subtotal: 0,
            reported_attempts: 0,
            unknown_attempts: 0,
            complete: true,
        }
    }
}

#[derive(Serialize, Default, Debug, Clone)]
pub struct UsageAccounting {
    pub input_tokens: TokenAccounting,
    pub output_tokens: TokenAccounting,
}

#[derive(Serialize, Debug, Clone)]
pub struct CostEstimate {
    pub basis: &'static str,
    pub input_price_per_mtok: f64,
    pub reported_input_subtotal_usd: f64,
    pub complete: bool,
}

#[derive(Serialize, Default, Debug, Clone)]
pub struct Telemetry {
    pub inference_posts: AttemptCounts,
    pub health_gets: AttemptCounts,
    pub prewarm_gets: AttemptCounts,
    pub semantic_calls: AttemptCounts,
    pub semantic_questions: u64,
    pub retry_sends: u64,
    /// Retry waits started, completed or interrupted; `retry_sleep_ms` is their elapsed sum.
    pub retry_waits: u64,
    pub retry_sleep_ms: u64,
    pub usage: UsageAccounting,
    /// The client cannot infer logical rounds from physical requests.
    pub logical_rounds: Option<u64>,
    pub cost_estimate: Option<CostEstimate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Human,
    Json,
    Jsonl,
    Toon,
}

/// Retry waits: how many, and their elapsed milliseconds in total.
#[derive(Serialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct Waited {
    pub count: u64,
    pub total_ms: u64,
}

/// Reported service usage: `None` when any inference attempt left it unknown, so an unknown
/// count is never read as a measured zero.
#[derive(Serialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
}

/// What the verb spent, at a glance: inference POSTs attempted and succeeded (retries and
/// failures included), the retry waits, the answers served from the local cache (never a
/// request), and the service-reported tokens or null.
#[derive(Serialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub attempted: u64,
    pub succeeded: u64,
    pub waited: Waited,
    pub cache_hits: u32,
    pub tokens: Tokens,
}

/// The scores of one decision at the gate, as the verb uses them: `best` and `next` are the two
/// top Choice probabilities, `none` is P(NONE) of that Choice, `any` is the Noul (the absolute
/// score of a selection, or the whole answer of a yes/no question), `fails` is P(the record
/// says the statement does not hold) of `filter`'s three-way Choice, the side that produces a
/// "no". A score the verb does not use is null. Compared to `threshold` as the code compares
/// them, without a calibration.
#[derive(Serialize, Default, Debug, Clone, PartialEq)]
pub struct Gate {
    pub best: Option<f64>,
    pub next: Option<f64>,
    pub none: Option<f64>,
    pub any: Option<f64>,
    pub fails: Option<f64>,
}

impl Gate {
    /// A yes/no question: the Noul alone.
    pub fn noul(p: f64) -> Self {
        Self {
            any: Some(p),
            ..Self::default()
        }
    }
}

/// The model the request named and the model the service says answered, kept apart. The free
/// backend chooses its model, so `requested` is null there; `answering` is `unknown` until a
/// response names one.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct DecisionModel {
    pub requested: Option<String>,
    pub answering: String,
}

impl Default for DecisionModel {
    fn default() -> Self {
        Self {
            requested: None,
            answering: "unknown".into(),
        }
    }
}

/// One candidate as round one ranked it: `index` is the verb's own number for the item (`why`
/// the 1-based line, `pick` the 1-based record, `pick --from` and `fill` the 1-based position
/// in the listing), `p` its Choice probability in its window.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct RoundOneCandidate {
    pub index: usize,
    pub p: f64,
}

/// One window of round one: every candidate by rank, NONE included as `none`, and the window's
/// Noul.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct RoundOneWindow {
    pub ranks: Vec<RoundOneCandidate>,
    pub none: f64,
    pub any: f64,
}

/// Round one of a tournament, as the shortlist computed it: the windows in input order, and
/// `finalists`, the items the finals request then held, in its order: the shortlist's picks
/// (`n` per window, by rank then window), widened by `fill`, joined by `why`'s panic lines or
/// capped by `route`; empty when round one alone decided.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct RoundOne {
    pub windows: Vec<RoundOneWindow>,
    pub finalists: Vec<usize>,
    pub n: usize,
}

/// What every decision of the verb was made with: one structure per envelope, one gate per
/// decision (a marker, a statement, a record, a hunk, a file, or the one pick), and, under
/// `JEVIFY_DECISION=round_one`, one `round_one` per tournament the verb ran. The field is
/// absent otherwise: a 1,000-line `why` would carry every line's score.
#[derive(Serialize, Default, Debug, Clone)]
pub struct Decision {
    pub verb: String,
    pub backend: &'static str,
    pub model: DecisionModel,
    pub threshold: f64,
    pub gates: Vec<Gate>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub round_one: Vec<RoundOne>,
}

/// `Default` is the meta of a command that never reached a backend (a usage error before
/// `Config` loaded): `backend` is then the empty string, since nothing answered.
#[derive(Serialize, Default, Debug, Clone)]
pub struct Meta {
    /// Which API answered: `typesafe` or `classifier`. Both run Jev; `model` says which build.
    pub backend: &'static str,
    pub model: Option<String>,
    pub elapsed_ms: u128,
    pub requests: u64,
    pub cache_hits: u32,
    pub input_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub threshold: f64,
    /// `x-typesafe-request-id` of the last TypeSafe response seen (success or failure); what
    /// TypeSafe support asks for. `null` until a request was made.
    pub request_id: Option<String>,
    pub usage: Usage,
    pub telemetry: Telemetry,
    pub decision: Decision,
}

#[derive(Serialize, Debug)]
pub struct ErrorBody {
    pub kind: &'static str,
    pub message: String,
    /// The next move, built from the caller's argv when jevify can correct it.
    pub hint: String,
    /// A runnable command, the caller's own corrected when jevify can.
    pub example: String,
}

#[derive(Serialize, Debug)]
pub struct Envelope<'a> {
    pub ok: bool,
    pub command: &'a str,
    pub version: &'static str,
    pub exit_code: i32,
    pub data: serde_json::Value,
    pub meta: Meta,
    pub error: Option<ErrorBody>,
}

pub fn render(format: Format, env: &Envelope) -> anyhow::Result<String> {
    let v = serde_json::to_value(env)?;
    Ok(match format {
        Format::Json => serde_json::to_string_pretty(&v)?,
        Format::Jsonl => serde_json::to_string(&v)?,
        Format::Toon => {
            toon_format::encode_default(&v).map_err(|e| anyhow::anyhow!(e.to_string()))?
        }
        Format::Human => unreachable!("human output is rendered by each command"),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn newline_quoting_and_status_escaping_differ() {
        assert_eq!(super::shell_quote(&["a\nb".into()]), b"'a\nb'");
        assert_eq!(super::status_escape("a\nb\rc"), "a\\nb\\rc");
    }
    #[test]
    fn presentation_quotes_each_posix_shell_token_as_bytes() {
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(
            super::shell_quote(&[
                "cp".into(),
                "report copy.txt".into(),
                "it's;$HOME".into(),
                "".into(),
                std::ffi::OsString::from_vec(vec![0xff, b'\n']),
            ]),
            b"'cp' 'report copy.txt' 'it'\\''s;$HOME' '' '\xff\n'"
        );
        assert!(super::shell_quote(&[]).is_empty());
    }
    use super::*;
    fn envelope() -> Envelope<'static> {
        Envelope {
            ok: true,
            command: "pick",
            version: "0.0.0",
            exit_code: 0,
            data: serde_json::json!({ "matches": [] }),
            meta: Meta {
                backend: "classifier",
                ..Meta::default()
            },
            error: None,
        }
    }
    #[test]
    fn json_and_jsonl_carry_the_same_envelope() {
        let env = envelope();
        let pretty: serde_json::Value =
            serde_json::from_str(&render(Format::Json, &env).unwrap()).unwrap();
        let line = render(Format::Jsonl, &env).unwrap();
        assert!(!line.contains('\n'), "jsonl is one line");
        assert_eq!(
            pretty,
            serde_json::from_str::<serde_json::Value>(&line).unwrap()
        );
        assert_eq!(pretty["command"], "pick");
        assert_eq!(pretty["error"], serde_json::Value::Null);
        // Which API answered is part of the envelope, not just of `-v` output.
        assert_eq!(pretty["meta"]["backend"], "classifier");
        // Nothing answered: the answering model is unknown, never an empty string.
        assert_eq!(pretty["meta"]["decision"]["model"]["answering"], "unknown");
        assert_eq!(pretty["meta"]["decision"]["gates"], serde_json::json!([]));
    }
    #[test]
    fn a_noul_gate_leaves_the_choice_scores_null() {
        let v = serde_json::to_value(Gate::noul(0.7)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "best": null, "next": null, "none": null, "any": 0.7, "fails": null })
        );
    }
    #[test]
    fn toon_renders_the_fields() {
        let s = render(Format::Toon, &envelope()).unwrap();
        assert!(s.contains("ok: true") && s.contains("command: pick"), "{s}");
    }
}
