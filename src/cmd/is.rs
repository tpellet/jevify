use crate::cmd::Outcome;
use crate::config::Config;
use crate::exit::{Exit, JevifyError};
use crate::jev::client::Client;
use crate::jev::{Question, Questions};

/// ~24k tokens of typical text at ~4 chars/token; keeps state + question under the 32k limit.
/// Denser text (non-Latin scripts) can exceed it and surfaces as `api_rejected_request` (exit 6).
const MAX_CHARS: usize = 96_000;

pub async fn run(
    ctx: &Config,
    statements: &[String],
    context: Option<&std::path::Path>,
    band: f64,
) -> Result<Outcome, JevifyError> {
    // Above 0.5 the "no" verdict becomes unreachable at the default threshold.
    if !(0.0..=0.5).contains(&band) {
        return Err(JevifyError::Usage(format!(
            "--band {band} must be within 0..=0.5"
        )));
    }
    let client = Client::new(ctx)?;
    let lines = if let Some(path) = context {
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            if path.as_os_str().to_string_lossy().contains('\n') {
                return Err(JevifyError::Input(
                    "--context takes a file path, not the text: pipe the text on stdin instead"
                        .into(),
                ));
            }
            let file = std::fs::File::open(&path).map_err(|e| {
                JevifyError::Input(format!("{}: {e}{}", path.display(), near_paths(&path)))
            })?;
            let mut bytes = Vec::new();
            file.take(crate::input::MAX_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| JevifyError::Input(e.to_string()))?;
            if bytes.len() > crate::input::MAX_BYTES {
                return Err(JevifyError::InputTooLarge("context exceeds 64 MiB".into()));
            }
            let lines = crate::input::split_lines(&String::from_utf8_lossy(&bytes));
            if lines.iter().all(|line| line.trim().is_empty()) {
                return Err(JevifyError::EmptyInput("context was empty"));
            }
            Ok(lines)
        })
        .await
        .map_err(|e| JevifyError::Input(e.to_string()))??
    } else {
        crate::input::read_stdin_async().await?
    };
    let text = crate::input::redact(&lines.join("\n"));
    let max_chars = MAX_CHARS.min(client.backend().max_state_chars());
    // Backend evidence budgets count Unicode characters, not UTF-8 bytes.
    let truncated = text.chars().count() > max_chars;
    if truncated {
        eprintln!("jevify is: input exceeds the evidence budget; whole input not judged");
        let mut data = serde_json::json!({ "p": null, "verdict": "unsure", "truncated": true, "reason": "input exceeds the evidence budget; whole input not judged" });
        let mut human = Vec::new();
        if statements.len() > 1 {
            data["statements"] = statements
                .iter()
                .map(|statement| {
                    human.extend_from_slice(format!("unsure\t{statement}\n").as_bytes());
                    serde_json::json!({"statement": statement, "verdict": "unsure", "p": null})
                })
                .collect();
        }
        return Ok(Outcome {
            exit: Exit::Abstain,
            data,
            human,
            exec: None,
        });
    }
    let mut qs = Questions::new();
    for (i, condition) in statements.iter().enumerate() {
        qs.insert(
            if statements.len() == 1 {
                "is".into()
            } else {
                format!("is_{i}")
            },
            Question::noul_with(
                format!("Does the text in the state satisfy this condition: \"{condition}\"?"),
                "The text clearly satisfies the condition",
                "The text does not satisfy the condition",
            ),
        );
    }
    let answer = client.ask(&serde_json::Value::String(text), &qs).await?;
    if statements.len() == 1 {
        let p = answer.noul("is")?;
        ctx.stats.gate(crate::output::Gate::noul(p));
        let (exit, verdict) = band_verdict(p, ctx.threshold, band);
        if exit == Exit::Abstain {
            eprintln!(
                "jevify is: unsure (p {p:.2}, between {:.2} and {:.2}); hint: state one fact the text would say literally, or split the statement into several quoted ones",
                (ctx.threshold - band).max(0.0),
                (ctx.threshold + band).min(1.0)
            );
        }
        return Ok(Outcome {
            exit,
            data: serde_json::json!({ "p": p, "verdict": verdict, "truncated": truncated }),
            human: Vec::new(),
            exec: None,
        });
    }
    let mut exit = Exit::Ok;
    let mut verdict = "yes";
    let mut entries = Vec::with_capacity(statements.len());
    let mut human = Vec::new();
    for (i, statement) in statements.iter().enumerate() {
        let p = answer.noul(&format!("is_{i}"))?;
        ctx.stats.gate(crate::output::Gate::noul(p));
        let (item_exit, item_verdict) = band_verdict(p, ctx.threshold, band);
        if item_exit == Exit::No || (item_exit == Exit::Abstain && exit == Exit::Ok) {
            exit = item_exit;
            verdict = item_verdict;
        }
        entries.push(serde_json::json!({"statement": statement, "verdict": item_verdict, "p": p}));
        human.extend_from_slice(format!("{item_verdict}\t{statement}\n").as_bytes());
    }
    Ok(Outcome {
        exit,
        data: serde_json::json!({ "statements": entries, "verdict": verdict, "truncated": truncated }),
        human,
        exec: None,
    })
}

/// `; nearby: a, b` naming up to three entries of the missing path's directory whose names are
/// closest to its own (within half its length), or nothing when none is close.
fn near_paths(path: &std::path::Path) -> String {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return String::new();
    };
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return String::new();
    };
    let mut near: Vec<(usize, String)> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .map(|entry| (crate::cmd::pick::distance(&name, &entry), entry))
        .filter(|(d, _)| *d <= name.chars().count().div_ceil(2))
        .collect();
    near.sort();
    near.truncate(3);
    if near.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = near
        .into_iter()
        .map(|(_, entry)| dir.join(entry).display().to_string())
        .collect();
    format!("; nearby: {}", shown.join(", "))
}

/// yes at or above `threshold + band`, no below `threshold - band`, unsure in between.
pub fn band_verdict(p: f64, threshold: f64, band: f64) -> (Exit, &'static str) {
    if p >= (threshold + band).min(1.0) {
        (Exit::Ok, "yes")
    } else if p < (threshold - band).max(0.0) {
        (Exit::No, "no")
    } else {
        (Exit::Abstain, "unsure")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_bands_around_the_threshold() {
        let cases = [
            (0.65, 0.5, 0.15, Exit::Ok),
            (0.64, 0.5, 0.15, Exit::Abstain),
            (0.35, 0.5, 0.15, Exit::Abstain),
            (0.34, 0.5, 0.15, Exit::No),
            // band 0: a plain threshold, no unsure verdict
            (0.5, 0.5, 0.0, Exit::Ok),
            (0.49, 0.5, 0.0, Exit::No),
            // the bands are clamped to 0..=1
            (1.0, 1.0, 0.15, Exit::Ok),
            (0.0, 0.0, 0.15, Exit::Abstain),
        ];
        for (p, t, band, exit) in cases {
            assert_eq!(band_verdict(p, t, band).0, exit, "p={p} t={t} band={band}");
        }
    }
}
