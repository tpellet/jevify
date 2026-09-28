use crate::exit::JevifyError;
use regex::Regex;
use std::io::{IsTerminal, Read};
use std::sync::LazyLock;

pub const MAX_BYTES: usize = 64 * 1024 * 1024;

static ANSI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)").unwrap()
});

static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)((?:(?:api[_-]?key|token|secret|passw(?:or)?d|authorization)\b["']?\s*[:=]\s*["']?|\bbearer\s+))[^\s"',;]{8,}|\b(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9-]{10,}|eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,})"#).unwrap()
});

/// Redact semantic text while preserving object keys and non-text values.
pub fn redact_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(redact(s)),
        serde_json::Value::Array(values) => values.iter().map(redact_value).collect(),
        serde_json::Value::Object(values) => values
            .iter()
            .map(|(key, value)| (key.clone(), redact_value(value)))
            .collect(),
        other => other.clone(),
    }
}

pub fn strip_ansi(s: &str) -> String {
    ANSI.replace_all(s, "").into_owned()
}

/// Best-effort masking of obvious secrets in text that is about to leave the machine (CI logs
/// routinely echo tokens). Not a guarantee; PRIVACY.md says so. Local output keeps the original.
pub fn redact(s: &str) -> String {
    SECRET
        .replace_all(s, |c: &regex::Captures| {
            format!("{}[REDACTED]", c.get(1).map_or("", |m| m.as_str()))
        })
        .into_owned()
}

pub fn split_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| {
            let l = l.rsplit('\r').next().unwrap_or(l);
            strip_ansi(l).trim_end().to_string()
        })
        .collect()
}

pub fn read_stdin() -> Result<Vec<String>, JevifyError> {
    let buf = read_stdin_bytes()?;
    let lines = split_lines(&String::from_utf8_lossy(&buf));
    if lines.iter().all(|l| l.trim().is_empty()) {
        return Err(JevifyError::EmptyInput("stdin was empty"));
    }
    Ok(lines)
}

fn check_terminal(is_terminal: bool) -> Result<(), JevifyError> {
    if is_terminal {
        return Err(JevifyError::EmptyInput("pipe text into jevify"));
    }
    Ok(())
}

pub fn read_stdin_bytes() -> Result<Vec<u8>, JevifyError> {
    let stdin = std::io::stdin();
    check_terminal(stdin.is_terminal())?;
    read_bytes(stdin.lock())
}

fn read_bytes(reader: impl Read) -> Result<Vec<u8>, JevifyError> {
    let mut buf = Vec::new();
    reader
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    if buf.len() > MAX_BYTES {
        return Err(JevifyError::InputTooLarge(format!(
            "more than {} MiB on stdin",
            MAX_BYTES / 1024 / 1024
        )));
    }
    if buf.is_empty() {
        return Err(JevifyError::EmptyInput("stdin was empty"));
    }
    Ok(buf)
}

/// Reads stdin on the blocking pool so a slow producer never stalls the runtime.
pub async fn read_stdin_async() -> Result<Vec<String>, JevifyError> {
    tokio::task::spawn_blocking(read_stdin)
        .await
        .map_err(|e| JevifyError::Input(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_check_requires_a_pipe() {
        assert!(check_terminal(false).is_ok());
        assert!(matches!(
            check_terminal(true),
            Err(JevifyError::EmptyInput(_))
        ));
    }
    #[test]
    fn secrets_are_redacted_and_ordinary_tokens_survive() {
        for (input, expected) in [
            (
                "export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123",
                "export GITHUB_TOKEN=[REDACTED]",
            ),
            (
                "Authorization: Bearer abcdefghijklmnop",
                "Authorization: Bearer [REDACTED]",
            ),
            (
                "sk-1234567890abcdef sk-1234567890abcde",
                "[REDACTED] sk-1234567890abcde",
            ),
            (
                "-token_expiry_seconds=3600\n+token_expiry_seconds=7200",
                "-token_expiry_seconds=3600\n+token_expiry_seconds=7200",
            ),
            ("token count: 12", "token count: 12"),
            ("", ""),
        ] {
            assert_eq!(redact(input), expected);
        }
        // Telemetry: string values are redacted, keys and non-text values are untouched.
        let value = serde_json::json!({
            "opaque_token_id": "token=abcdefghijk",
            "attempt": 2,
            "nested": ["secret=abcdefghijk", true, null]
        });
        let redacted = redact_value(&value);
        assert_eq!(redacted["opaque_token_id"], "token=[REDACTED]");
        assert_eq!(redacted["attempt"], 2);
        assert_eq!(redacted["nested"][0], "secret=[REDACTED]");
        assert_eq!(redacted["nested"][1], true);
    }
}
