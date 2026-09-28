use crate::cli::GlobalOpts;
use crate::exit::JevifyError;
use crate::jev::client::Stats;
use crate::output::Meta;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Which service answers the questions. classifier.dev translates Noul into a two-label
/// Choice; its relative scores and TypeSafe's absolute Noul have different semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Typesafe,
    Classifier,
}

impl Backend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Typesafe => "typesafe",
            Self::Classifier => "classifier",
        }
    }
    pub const fn default_base_url(self) -> &'static str {
        match self {
            Self::Typesafe => "https://api.typesafe.ai",
            Self::Classifier => "https://classifier.dev",
        }
    }
    /// How many items one Choice may offer, NONE excluded. TypeSafe accepts 255 options and
    /// jevify windows at 200; classifier.dev caps a dimension at 100 labels, so NONE takes the
    /// hundredth slot. Only the number of windows changes, never what a window asks.
    pub const fn window(self) -> usize {
        match self {
            Self::Typesafe => 200,
            Self::Classifier => 99,
        }
    }
    /// Character budget for the state a caller may build. classifier.dev rejects an input over
    /// 32,000 characters (`input_too_long`); 30,000 leaves room for the JSON around the items.
    pub const fn max_state_chars(self) -> usize {
        match self {
            Self::Typesafe => usize::MAX,
            Self::Classifier => 30_000,
        }
    }
    /// 8 in flight is ~20 req/s, under TypeSafe's 1,200/min. classifier.dev is free and shared
    /// per IP, so jevify stays at 4 there by default.
    pub const fn default_concurrency(self) -> usize {
        match self {
            Self::Typesafe => 8,
            Self::Classifier => 4,
        }
    }
}

pub struct Config {
    pub backend: Backend,
    pub key: Option<String>,
    /// Read lazily by `api_key`: `capabilities` and `init` need no key, so a bad
    /// path must not break them.
    pub key_file: Option<PathBuf>,
    pub base_url: String,
    pub model: String,
    pub threshold: f64,
    pub concurrency: usize,
    pub cache_dir: Option<PathBuf>,
    pub stats: Arc<Stats>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn save_dir(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            directories::ProjectDirs::from("", "", "jevify")
                .map(|dirs| dirs.cache_dir().to_path_buf())
        })
}

/// Where `why` and `filter` save raw input, or `None` when they must not: the `--no-save` flag
/// for one call, `JEVIFY_NO_SAVE` for every call of a fleet that never wants raw input on disk.
/// The saving switch is its own, as the store is: `JEVIFY_NO_CACHE` governs answers only.
pub fn saved_input_dir(no_save: bool) -> Option<PathBuf> {
    if no_save || env("JEVIFY_NO_SAVE").is_some() {
        return None;
    }
    save_dir(env("JEVIFY_CACHE_DIR").as_deref())
}

/// The directory of the user's own configuration (`kinds.jsonl`): the value of
/// `JEVIFY_CONFIG_DIR` when it is set and not blank, else the platform configuration directory.
pub fn config_dir(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            directories::ProjectDirs::from("", "", "jevify")
                .map(|dirs| dirs.config_dir().to_path_buf())
        })
}

/// The verb's overall budget, `JEVIFY_DEADLINE` in whole seconds (default 600). Read when a
/// `Client` is built, so `capabilities` and `init` never need it; zero is a
/// usage error.
pub(crate) fn deadline() -> Result<std::time::Duration, JevifyError> {
    let seconds: u64 = parse("JEVIFY_DEADLINE", 600)?;
    if seconds == 0 {
        return Err(JevifyError::Usage(
            "JEVIFY_DEADLINE=0 is not valid; give the verb's budget in whole seconds".into(),
        ));
    }
    Ok(std::time::Duration::from_secs(seconds))
}

fn parse<T: std::str::FromStr>(name: &str, default: T) -> Result<T, JevifyError> {
    match env(name) {
        None => Ok(default),
        Some(v) => v
            .parse()
            .map_err(|_| JevifyError::Usage(format!("{name}={v} is not valid"))),
    }
}

pub(crate) fn base_url(backend: Backend, value: Option<&str>) -> Result<String, JevifyError> {
    let value = value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(backend.default_base_url());
    // Do not echo the value: a rejected URL can contain credentials.
    let invalid = || {
        JevifyError::Usage("invalid API endpoint URL: JEVIFY_BASE_URL requires the backend's HTTPS host on port 443, or localhost/127.0.0.1; userinfo is forbidden".into())
    };
    let url = reqwest::Url::parse(value).map_err(|_| invalid())?;
    // URL parsing discards empty userinfo, so also inspect the original authority.
    let has_userinfo = value.split_once(':').is_some_and(|(_, rest)| {
        rest.trim_start_matches(['/', '\\'])
            .split(['/', '?', '#', '\\'])
            .next()
            .is_some_and(|authority| authority.contains('@'))
    });
    if has_userinfo || !url.username().is_empty() || url.password().is_some() {
        return Err(invalid());
    }
    let host = url.host_str().ok_or_else(invalid)?;
    let local = host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1";
    let expected = match backend {
        Backend::Typesafe => "api.typesafe.ai",
        Backend::Classifier => "classifier.dev",
    };
    if !local
        && (url.scheme() != "https"
            || !host.eq_ignore_ascii_case(expected)
            || url.port().is_some_and(|port| port != 443))
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

impl Config {
    pub fn load(g: &GlobalOpts) -> Result<Self, JevifyError> {
        let threshold = g.threshold.unwrap_or(0.5);
        if !(0.0..=1.0).contains(&threshold) {
            return Err(JevifyError::Usage(format!(
                "threshold {threshold} must be within 0..=1"
            )));
        }
        let cache_dir = if g.no_cache || env("JEVIFY_NO_CACHE").is_some() {
            None
        } else if let Some(d) = env("JEVIFY_CACHE_DIR") {
            Some(PathBuf::from(d))
        } else {
            directories::ProjectDirs::from("", "", "jevify").map(|p| p.cache_dir().to_path_buf())
        };
        let key = env("TYPESAFE_API_KEY");
        let key_file = env("TYPESAFE_API_KEY_FILE").map(PathBuf::from);
        let backend = match env("JEVIFY_BACKEND").as_deref() {
            None => {
                // No key, no prompt and no signup path: jevify answers out of the box through
                // classifier.dev, and uses your own TypeSafe quota as soon as a key is there.
                if key.is_some() || key_file.is_some() {
                    Backend::Typesafe
                } else {
                    Backend::Classifier
                }
            }
            Some("typesafe") => Backend::Typesafe,
            Some("classifier") => Backend::Classifier,
            Some(other) => {
                return Err(JevifyError::Usage(format!(
                    "JEVIFY_BACKEND={other} is not valid; use typesafe or classifier"
                )));
            }
        };
        if backend == Backend::Classifier && g.model.is_some() {
            return Err(JevifyError::Usage(
                "--model/JEVIFY_MODEL requires the typesafe backend; classifier chooses its model"
                    .into(),
            ));
        }
        Ok(Self {
            backend,
            key,
            key_file,
            base_url: base_url(backend, env("JEVIFY_BASE_URL").as_deref())?,
            // TypeSafe requests pin this model. classifier.dev chooses its own model and the
            // resolved response identity is reported in meta.model.
            model: g.model.clone().unwrap_or_else(|| "jev-1.13.0".into()),
            threshold,
            // 16 in flight at ~0.4 s each is ~40 req/s, twice the 1,200/min budget; 8 stays under it.
            concurrency: parse("JEVIFY_CONCURRENCY", backend.default_concurrency())?.max(1),
            cache_dir,
            stats: Arc::new(Stats::default()),
        })
    }

    /// The key from `TYPESAFE_API_KEY`, else the trimmed contents of `TYPESAFE_API_KEY_FILE`.
    pub fn api_key(&self) -> Result<String, JevifyError> {
        if let Some(k) = &self.key {
            return Ok(k.clone());
        }
        let Some(p) = &self.key_file else {
            return Err(JevifyError::MissingKey);
        };
        let k = std::fs::read_to_string(p).map_err(|e| {
            JevifyError::Input(format!("TYPESAFE_API_KEY_FILE {}: {e}", p.display()))
        })?;
        let k = k.trim().to_string();
        if k.is_empty() {
            return Err(JevifyError::MissingKey);
        }
        Ok(k)
    }

    pub fn meta(&self) -> Meta {
        let s = &self.stats;
        let telemetry = s.telemetry();
        let usage = &telemetry.usage.input_tokens;
        let tokens = usage.reported_subtotal;
        let input_tokens = usage.complete.then_some(tokens);
        let output = &telemetry.usage.output_tokens;
        let output_tokens = output.complete.then_some(output.reported_subtotal);
        let answering = s.model.lock().unwrap().clone();
        Meta {
            backend: self.backend.as_str(),
            model: answering.clone(),
            decision: crate::output::Decision {
                verb: String::new(),
                backend: self.backend.as_str(),
                model: crate::output::DecisionModel {
                    // classifier.dev chooses its model: nothing was requested there.
                    requested: (self.backend == Backend::Typesafe).then(|| self.model.clone()),
                    answering: answering.unwrap_or_else(|| "unknown".into()),
                },
                threshold: self.threshold,
                gates: s.gates(),
                round_one: s.rounds_one(),
            },
            elapsed_ms: 0,
            requests: telemetry.inference_posts.attempted,
            cache_hits: s.cache_hits.load(Ordering::Relaxed),
            input_tokens,
            threshold: self.threshold,
            request_id: s.request_id.lock().unwrap().clone(),
            usage: crate::output::Usage {
                attempted: telemetry.inference_posts.attempted,
                succeeded: telemetry.inference_posts.succeeded,
                waited: crate::output::Waited {
                    count: telemetry.retry_waits,
                    total_ms: telemetry.retry_sleep_ms,
                },
                cache_hits: s.cache_hits.load(Ordering::Relaxed),
                tokens: crate::output::Tokens {
                    input: input_tokens,
                    output: output_tokens,
                },
            },
            telemetry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base_urls_are_bound_to_the_backend() {
        for (backend, input, expected) in [
            (
                Backend::Typesafe,
                "https://API.TypeSafe.AI",
                Some("https://api.typesafe.ai"),
            ),
            (
                Backend::Typesafe,
                "https://api.typesafe.ai:443",
                Some("https://api.typesafe.ai"),
            ),
            (
                Backend::Classifier,
                "https://classifier.dev:443",
                Some("https://classifier.dev"),
            ),
            (Backend::Typesafe, "https://api.typesafe.ai.", None),
            (Backend::Classifier, "https://classifier.dev.", None),
            (Backend::Typesafe, "https://api.typesafe.ai:8443", None),
            (Backend::Classifier, "https://classifier.dev:8443", None),
            (
                Backend::Typesafe,
                "https://api.typesafe.ai.evil.example",
                None,
            ),
            (Backend::Classifier, "https://api.typesafe.ai", None),
            (Backend::Typesafe, "https://classifier.dev", None),
            (Backend::Typesafe, "http://api.typesafe.ai", None),
            (Backend::Classifier, "http://classifier.dev", None),
            (Backend::Classifier, "https://user:pw@classifier.dev", None),
            (Backend::Typesafe, "https://@api.typesafe.ai", None),
            (Backend::Typesafe, "https:/@api.typesafe.ai", None),
            (Backend::Typesafe, "https:///@api.typesafe.ai", None),
            (Backend::Typesafe, "not a URL", None),
        ] {
            let result = base_url(backend, Some(input));
            match expected {
                Some(expected) => assert_eq!(result.unwrap(), expected, "{input}"),
                None => assert_eq!(result.unwrap_err().exit().code(), 2, "{input}"),
            }
        }
        for backend in [Backend::Typesafe, Backend::Classifier] {
            for value in [None, Some(""), Some(" \t ")] {
                assert_eq!(
                    base_url(backend, value).unwrap(),
                    backend.default_base_url()
                );
            }
            for input in [
                "http://127.0.0.1:1234",
                "http://localhost:1234",
                "ftp://localhost:1234",
            ] {
                assert_eq!(base_url(backend, Some(input)).unwrap(), input);
            }
        }
    }
}
