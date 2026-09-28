//! `label a,b,c`: tag each stdin record with one of the caller's labels.
//!
//! Output is `LABEL<TAB>RECORD` in input order, `?` for an unsure record. The labels are
//! validated by the parser (`cli::Labels`); their count against the backend's window is
//! checked here, after `Config::load`, where the backend is known.
//!
//! One Choice per record, the labels plus an internal `NONE`, through the record pipeline of
//! `records`. The `?` decision is `tournament::decide` over a `Ranking` built from the
//! record's answer with `any = 1.0`, so the threshold plays no part: `NONE` winning or tying
//! the best label and the winner ratio do. `label` saves nothing: every record comes out.

use crate::{
    cmd::Outcome,
    config::Config,
    exit::{Exit, JevifyError},
    jev::{Question, Questions},
    records::{self, Split, write_record},
    tournament::{Candidate, Decision, Ranking, decide},
};
use std::collections::BTreeMap;

pub struct LabelFlags {
    pub split: Split,
    pub files: bool,
}

const UNSURE: &str = "?";
const EXAMPLE: &str = "head -n 20000 input | jevify label bug,feature";

pub async fn run(
    ctx: &Config,
    labels: Vec<String>,
    flags: LabelFlags,
    machine: bool,
) -> Result<Outcome, JevifyError> {
    let window = ctx.backend.window();
    if labels.len() > window {
        return Err(JevifyError::Usage(format!(
            "{} labels given; at most {window} on the {} backend",
            labels.len(),
            ctx.backend.as_str()
        )));
    }
    let mut input = records::read("label", flags.split, machine, true, EXAMPLE).await?;
    input.excerpts(flags.files).await?;
    let questions = Questions::from([("label".into(), question(&labels))]);
    let mut labelled = 0;
    let mut unsure = 0;
    let mut entries = Vec::new();
    let mut stdout = std::io::stdout().lock();
    let completed = input
        .judge(
            ctx,
            &questions,
            (UNSURE.to_string(), 0.0),
            |response| {
                let (label, p, gate) = verdict(&labels, response.probs("label")?, ctx.threshold);
                ctx.stats.gate(gate);
                Ok((label, p))
            },
            |index, (label, p)| {
                if label == UNSURE {
                    unsure += 1;
                } else {
                    labelled += 1;
                }
                if machine {
                    entries.push(input.entry(index, |entry| {
                        entry["label"] = label.as_str().into();
                        entry["p"] = (*p).into();
                    }));
                    return Ok(true);
                }
                let raw = input.raw(index);
                let mut line = Vec::with_capacity(label.len() + 1 + raw.len());
                line.extend_from_slice(label.as_bytes());
                line.push(b'\t');
                line.extend_from_slice(raw);
                write_record(&mut stdout, &line)
            },
        )
        .await?;
    let total = input.records.len();
    let exit = if completed && total > 0 && unsure == total {
        Exit::Abstain
    } else {
        Exit::Ok
    };
    input.status(
        ctx,
        &format!("labelled {labelled} of {total}, {unsure} unsure"),
    );
    Ok(Outcome {
        exit,
        data: serde_json::json!({
            "records": entries, "labelled": labelled, "total": total, "unsure": unsure,
            "complete": completed, "excerpts_withheld": input.unread.count
        }),
        human: Vec::new(),
        exec: None,
    })
}

/// The one Choice: the caller's labels, and NONE for a record none of them fits.
fn question(labels: &[String]) -> Question {
    let mut criteria: BTreeMap<String, Option<String>> =
        labels.iter().map(|l| (l.clone(), None)).collect();
    criteria.insert(
        "NONE".into(),
        Some("none of the labels fits this record".into()),
    );
    Question::choice(
        format!(
            "judge this one record alone and give it the one label that fits it best: {}",
            labels.join(", ")
        ),
        criteria,
    )
}

/// The label of one record and the probability behind it: the winner's, or the best label's
/// when the answer is `?`; with the gate scores (`any` is null: the Noul plays no part).
fn verdict(
    labels: &[String],
    probs: &BTreeMap<String, f64>,
    threshold: f64,
) -> (String, f64, crate::output::Gate) {
    let mut candidates: Vec<Candidate> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| Candidate {
            index,
            p: probs.get(label).copied().unwrap_or(0.0),
        })
        .collect();
    candidates.sort_by(|a, b| b.p.total_cmp(&a.p));
    let best = candidates.first().map_or(0.0, |c| c.p);
    let ranking = Ranking {
        candidates,
        any: 1.0,
        none: probs.get("NONE").copied().unwrap_or(0.0),
        windows: 1,
        n: labels.len(),
    };
    let gate = crate::output::Gate {
        any: None,
        ..super::gate_of(&ranking)
    };
    match decide(&ranking, threshold) {
        Decision::Found(winner) => (labels[winner.index].clone(), winner.p, gate),
        Decision::NoMatch | Decision::Ambiguous(_) => (UNSURE.into(), best, gate),
    }
}
