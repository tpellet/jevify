use crate::exit::{Exit, JevifyError};

pub mod add;
pub mod agent;
pub mod fill;
pub mod filter;
pub mod is;
pub mod label;
pub mod pick;
pub mod run;
pub mod sort;
pub mod why;

/// The gate scores of a ranking: the two top Choice probabilities, P(NONE) and the Noul.
pub(crate) fn gate_of(ranking: &crate::tournament::Ranking) -> crate::output::Gate {
    crate::output::Gate {
        best: ranking.candidates.first().map(|c| c.p),
        next: ranking.candidates.get(1).map(|c| c.p),
        none: Some(ranking.none),
        any: Some(ranking.any),
        fails: None,
    }
}

/// The closest candidates of an abstention, best first, at most three, each clipped to 80
/// characters: what `data.closest` holds and the stderr line names.
pub(crate) fn closest(
    ranking: &crate::tournament::Ranking,
    text: impl Fn(usize) -> String,
) -> Vec<(String, f64)> {
    ranking
        .candidates
        .iter()
        .take(3)
        .map(|c| {
            let full = text(c.index);
            let clipped: String = full.chars().take(80).collect();
            let clipped = if clipped.len() < full.len() {
                format!("{clipped}…")
            } else {
                clipped
            };
            (clipped, c.p)
        })
        .collect()
}

/// The next move after an abstention, from its scores: what a caller can change to get an
/// answer, never a promise that it will. `advice` is the verb's own way to rephrase.
pub(crate) fn abstain_hint(
    ranking: &crate::tournament::Ranking,
    threshold: f64,
    advice: &str,
) -> String {
    match ranking.candidates.first() {
        None => format!("no candidate scored: {advice}"),
        Some(best) if best.p <= ranking.none => format!(
            "nothing fits better than none ({:.2}): {advice}",
            ranking.none
        ),
        Some(_) if ranking.any < threshold => format!(
            "the check that any candidate fits scored {:.2}, below the threshold {threshold:.2}: {advice}",
            ranking.any
        ),
        Some(_) => {
            "the nearest are too close to tell apart: add the detail that separates them".into()
        }
    }
}

/// The rephrasing advice of `pick` and `fill`.
pub(crate) const DESCRIBE_THE_RECORD: &str =
    "describe what the record itself says (its words, not your goal), or list other candidates";

/// The stderr line of an abstention: the nearest candidates with their scores, labelled as not
/// chosen so that no caller takes one for the answer, then the hint.
pub(crate) fn abstain_line(
    verb: &str,
    status: &str,
    closest: &[(String, f64)],
    hint: &str,
) -> String {
    let named = closest
        .iter()
        .map(|(text, p)| format!("{} ({p:.2})", crate::output::status_escape(text)))
        .collect::<Vec<_>>()
        .join(", ");
    let named = if named.is_empty() {
        "none".into()
    } else {
        named
    };
    format!("jevify {verb}: {status}; nearest (not chosen): {named}; hint: {hint}")
}

/// Result of a verb: exit code, machine data, and the exact human stdout text.
pub struct Outcome {
    pub exit: Exit,
    pub data: serde_json::Value,
    pub human: Vec<u8>,
    pub exec: Option<Exec>,
}

pub struct Exec {
    pub argv: Vec<std::ffi::OsString>,
    pub stdin_null: bool,
}

/// Asks on /dev/tty so it works when stdout is piped. `Ok(None)` means there is no TTY
/// (never act); `Ok(Some(false))` means the user declined. Shared by `run` and `add`.
pub fn confirm_tty(prompt: &str) -> Result<Option<bool>, JevifyError> {
    use std::io::{BufRead, Write};
    let Ok(tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return Ok(None);
    };
    let mut w = &tty;
    let _ = write!(w, "{prompt}");
    let _ = w.flush();
    let mut line = String::new();
    std::io::BufReader::new(&tty)
        .read_line(&mut line)
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    Ok(Some(matches!(line.trim(), "y" | "Y" | "yes")))
}
