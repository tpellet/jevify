//! `filter '<statement>'`: keep the stdin records a statement holds for; `filter --label
//! a,b,c`: tag each stdin record with one of the caller's labels.
//!
//! Both read the record pipeline of `records`, one Choice per distinct record, and write in
//! input order. `label` is the second half of this file.

use crate::{
    cmd::Outcome,
    config::Config,
    exit::{Exit, JevifyError},
    jev::{Question, Questions},
    records::{self, Split, write_record},
    tournament::{Candidate, Decision, Ranking, decide},
};
use std::collections::BTreeMap;

const EXAMPLE: &str = "head -n 20000 input | jevify filter 'x'";

pub struct FilterFlags {
    pub invert: bool,
    pub count: bool,
    pub strict: bool,
    pub split: Split,
    pub files: bool,
    pub no_save: bool,
}

pub async fn run(
    ctx: &Config,
    statement: &str,
    flags: FilterFlags,
    machine: bool,
) -> Result<Outcome, JevifyError> {
    let mut input = records::read("filter", flags.split, machine, flags.no_save, EXAMPLE).await?;
    input.excerpts(flags.files).await?;
    let questions = Questions::from([("filter".into(), question(statement))]);
    let mut kept = 0;
    let mut unsure = 0;
    let mut entries = Vec::new();
    let mut stdout = std::io::stdout().lock();
    let completed = input
        .judge(
            ctx,
            &questions,
            (0.0, "unsure"),
            |response| {
                let scores = response.probs("filter")?;
                let (p, fails, silent) = (scores[HOLDS], scores[FAILS], scores[SILENT]);
                // The three sides of the Choice, so a "no" can be read back from the envelope.
                ctx.stats.gate(crate::output::Gate {
                    any: Some(p),
                    none: Some(silent),
                    fails: Some(fails),
                    ..Default::default()
                });
                Ok((p, verdict(p, fails, ctx.threshold, 0.15)))
            },
            |index, &(p, verdict)| {
                unsure += usize::from(verdict == "unsure");
                if !selected(verdict, flags.invert, flags.strict) {
                    return Ok(true);
                }
                kept += 1;
                if machine {
                    entries.push(input.entry(index, |entry| {
                        entry["p"] = p.into();
                        entry["verdict"] = verdict.into();
                    }));
                    Ok(true)
                } else if flags.count {
                    Ok(true)
                } else {
                    write_record(&mut stdout, input.raw(index))
                }
            },
        )
        .await?;
    let total = input.records.len();
    let mut complete = completed && input.saved.is_ok();
    let mut exit = if !completed {
        Exit::Ok
    } else if total > 0 && unsure == total {
        Exit::Abstain
    } else if kept == 0 {
        Exit::No
    } else {
        Exit::Ok
    };
    if !machine && flags.count && !write_record(&mut stdout, format!("{kept}\n").as_bytes())? {
        exit = Exit::Ok;
        complete = false;
    }
    let saved_description = match &input.saved {
        Ok(path) => path.display().to_string(),
        Err(reason) => format!("not saved ({reason})"),
    };
    input.status(
        ctx,
        &format!("kept {kept} of {total}, {unsure} unsure, full output: {saved_description}"),
    );
    Ok(Outcome {
        exit,
        data: serde_json::json!({
            "records": entries, "kept": kept, "total": total, "unsure": unsure,
            "complete": complete, "saved_input": input.saved.as_ref().ok(),
            "excerpts_withheld": input.unread.count
        }),
        human: Vec::new(),
        exec: None,
    })
}

const HOLDS: &str = "the record says the statement holds";
const FAILS: &str = "the record says the statement does not hold";
const SILENT: &str = "the record does not say";

/// Three answers, not two: a record that says nothing either way is neither a yes nor a no.
/// A Noul reads "not stated" as a confident no (measured 2026-09-22 on both backends: merge
/// subjects under "the change is a bug fix" scored 0.00–0.17), so the third option takes that
/// mass and the band can see it.
fn question(statement: &str) -> Question {
    Question::choice(
        format!("judge this one record on its own: {statement}"),
        [
            (
                HOLDS.into(),
                Some("the record shows that the statement is true of it".into()),
            ),
            (
                FAILS.into(),
                Some(
                    "the record shows that the statement is false of it: it says the opposite, \
                     or it is about something else"
                        .into(),
                ),
            ),
            (
                SILENT.into(),
                Some(
                    "the record has no content to judge by: a bare reference such as a number, \
                     a name or a merge line"
                        .into(),
                ),
            ),
        ]
        .into(),
    )
}

/// One-sided: yes when P(holds) is at or above `threshold + band`, no when P(fails) is at or
/// above the same mark, unsure otherwise: below the mark on both sides, or mostly unstated.
/// Nothing is compared to `threshold - band`; the default mark is 0.5 + 0.15 = 0.65.
fn verdict(holds: f64, fails: f64, threshold: f64, band: f64) -> &'static str {
    let mark = (threshold + band).min(1.0);
    if holds >= mark {
        "yes"
    } else if fails >= mark {
        "no"
    } else {
        "unsure"
    }
}

fn selected(verdict: &str, invert: bool, strict: bool) -> bool {
    if verdict == "unsure" {
        !strict
    } else {
        (verdict == "yes") != invert
    }
}

// `filter --label a,b,c`: tag each stdin record with one of the caller's labels.
//
// Output is `LABEL<TAB>RECORD` in input order, `?` for an unsure record. The labels are
// validated by the parser (`cli::Labels`); their count against the backend's window is
// checked here, after `Config::load`, where the backend is known.
//
// One Choice per record, the labels plus an internal `NONE`. The `?` decision is
// `tournament::decide` over a `Ranking` built from the record's answer with `any = 1.0`, so the
// threshold plays no part: `NONE` winning or tying the best label and the winner ratio do.
// Labelling saves nothing: every record comes out.

pub struct LabelFlags {
    pub split: Split,
    pub files: bool,
}

const UNSURE: &str = "?";
const LABEL_EXAMPLE: &str = "head -n 20000 input | jevify filter --label bug,feature";

pub async fn label(
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
    let mut input = records::read("filter", flags.split, machine, true, LABEL_EXAMPLE).await?;
    input.excerpts(flags.files).await?;
    let questions = Questions::from([("label".into(), label_question(&labels))]);
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
                let (label, p, gate) =
                    label_verdict(&labels, response.probs("label")?, ctx.threshold);
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
fn label_question(labels: &[String]) -> Question {
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
fn label_verdict(
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
