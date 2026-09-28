use crate::{
    cmd::Outcome,
    config::Config,
    exit::{Exit, JevifyError},
    jev::{Question, Questions},
    records::{self, Split, write_record},
};

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
