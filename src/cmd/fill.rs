use crate::{
    cmd::{Exec, Outcome},
    config::Config,
    exit::{AMBIGUOUS, Exit, INSUFFICIENT_EVIDENCE, JevifyError, NO_MATCH, UNSURE_FLAG},
    input, jev,
    jev::{Question, Questions, client::Client},
    marker::{self, Marker},
    output,
    records::{Record, Split},
    source::{self, Scope},
    tournament::{self, Decision, Finalists, Prompts, Ranking, Shortlist},
};
use futures::future::join_all;
use serde_json::json;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::{IsTerminal, Read},
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

const FLAG_BAND: f64 = 0.15;
const MAX_CONTEXT_CHARS: usize = 96_000;
const MAX_FINALISTS: usize = 24;
/// A rival within this ratio of the best blocks it: `tournament::decide`'s winner ratio.
const RIVAL_RATIO: f64 = 2.0;

pub struct FillFlags {
    pub dry_run: bool,
    pub quiet: bool,
    pub candidates: Option<PathBuf>,
    pub context: Option<PathBuf>,
    pub field: Option<usize>,
    pub key: Option<String>,
    pub split: Split,
}

/// The environment variable that names the exec-mode status file.
pub const STATUS_FILE_ENV: &str = "JEVIFY_STATUS_FILE";

/// The path the caller asked the status to be written to, if any.
fn status_path() -> Option<PathBuf> {
    std::env::var_os(STATUS_FILE_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// One JSON object saying whether the command started. `ran` is the whole point: after `exec`
/// jevify is gone and its exit code is the command's, so the answer has to be written down
/// before the hand-off. It is written for every outcome `fill` decides, and never afterwards,
/// so an absent file means nothing ran either.
fn write_status(
    path: &Path,
    exit: Exit,
    ran: bool,
    data: &serde_json::Value,
    error: Option<&JevifyError>,
) -> std::io::Result<()> {
    let field = |name: &str| data.get(name).cloned().unwrap_or(serde_json::Value::Null);
    let body = json!({
        "command": "fill",
        "version": env!("CARGO_PKG_VERSION"),
        "exit_code": exit.code(),
        "ran": ran,
        "argv": field("argv"),
        "reason": field("reason"),
        "markers": field("markers"),
        "error": error.map(|e| json!({
            "kind": e.kind(), "message": e.to_string(), "hint": e.hint(), "example": e.example(),
        })),
    });
    std::fs::write(path, format!("{body}\n"))
}

pub async fn run(
    ctx: &Config,
    flags: FillFlags,
    cmd: &[OsString],
    machine: bool,
) -> Result<Outcome, JevifyError> {
    let outcome = resolve(ctx, flags, cmd, machine).await;
    let Some(path) = status_path() else {
        return outcome;
    };
    match &outcome {
        Ok(out) => {
            // A command jevify cannot report having started is a command jevify does not start.
            if let Err(e) = write_status(&path, out.exit, out.exec.is_some(), &out.data, None) {
                return Err(JevifyError::status_file_unwritable(format!(
                    "{}: {e}",
                    path.display()
                )));
            }
        }
        Err(e) => {
            // The original error is the answer; a failed status write cannot replace it.
            let _ = write_status(&path, e.exit(), false, &serde_json::Value::Null, Some(e));
        }
    }
    outcome
}

async fn resolve(
    ctx: &Config,
    flags: FillFlags,
    cmd: &[OsString],
    machine: bool,
) -> Result<Outcome, JevifyError> {
    if machine && !flags.dry_run {
        return Err(JevifyError::Usage(
            "machine output requires --dry-run".into(),
        ));
    }
    let args = marker::parse(cmd).map_err(|e| JevifyError::Usage(e.to_string()))?;
    let markers: Vec<_> = args.iter().flat_map(|arg| &arg.markers).collect();
    let mut env = source::Env::from_process(source::LISTER_TIMEOUT);
    // `git -C DIR ...` lists where git itself will run, not where jevify was started.
    if let Some(dir) = git_directory(cmd) {
        env.cwd = env.cwd.join(dir);
    }
    // One `Kind` per marker (`None` for `one` and `flag`), resolved once, before any other I/O.
    let mut kinds = Vec::with_capacity(markers.len());
    for marker in &markers {
        kinds.push(validate_kind(
            marker,
            &args[marker.argv_index].literal,
            &env,
        )?);
        if marker.options.len() > ctx.backend.window() {
            return Err(JevifyError::Usage(format!(
                "one has {} options; the backend accepts at most {}",
                marker.options.len(),
                ctx.backend.window()
            )));
        }
    }
    marker::check_stdin_roles(&args, flags.candidates.is_some(), flags.context.is_some())
        .map_err(|e| JevifyError::Usage(e.to_string()))?;
    if flags.field == Some(0) || flags.field.is_some() && flags.key.is_some() {
        return Err(JevifyError::Usage(
            "use a positive --field or --key, not both".into(),
        ));
    }
    let program = cmd
        .first()
        .ok_or_else(|| JevifyError::Usage("missing command".into()))?
        .clone();
    tokio::task::spawn_blocking(move || check_program(&program))
        .await
        .map_err(|e| JevifyError::Input(e.to_string()))??;
    let needs_candidates = markers.iter().any(|m| m.kind == "-");
    let needs_context = markers
        .iter()
        .any(|m| matches!(m.kind.as_str(), "one" | "flag"));
    let stdin_null =
        needs_candidates && flags.candidates.is_none() || needs_context && flags.context.is_none();
    let candidates = if needs_candidates {
        read_input(flags.candidates.clone()).await?
    } else {
        vec![]
    };
    let context = if needs_context {
        read_input(flags.context.clone()).await?
    } else {
        vec![]
    };
    let context = input::redact(&input::split_lines(&String::from_utf8_lossy(&context)).join("\n"));
    let insufficient =
        context.chars().count() > MAX_CONTEXT_CHARS.min(ctx.backend.max_state_chars());
    let limit = ctx.backend.window() * (ctx.backend.window() / 3);
    // The literal before a path marker is its scope; every other kind lists from the cwd.
    let prefix_of = |i: usize| -> Option<PathBuf> {
        kinds[i].as_ref().filter(|k| k.path_kind).and_then(|_| {
            (!markers[i].prefix.is_empty()).then(|| PathBuf::from(&markers[i].prefix))
        })
    };
    let mut scopes = Vec::new();
    for (i, m) in markers.iter().enumerate() {
        if kinds[i].is_some() {
            let key = (m.kind.clone(), prefix_of(i));
            if !scopes.contains(&key) {
                scopes.push(key);
            }
        }
    }
    let listings = join_all(scopes.iter().map(|(kind, prefix)| {
        let scope = if kind == "-" {
            Scope::Input {
                bytes: candidates.clone(),
                split: flags.split,
                field: flags.field,
                key: flags.key.clone(),
            }
        } else {
            Scope::Prefix(prefix.clone())
        };
        source::enumerate(kind, scope, limit, &env)
    }))
    .await;
    let mut states = Vec::new();
    for (i, m) in markers.iter().enumerate() {
        let mut state = State::default();
        if let Some(kind) = &kinds[i] {
            let prefix = prefix_of(i);
            let index = scopes
                .iter()
                .position(|(kind, scope)| kind == &m.kind && scope == &prefix)
                .unwrap();
            let listing = match &listings[index] {
                Ok(listing) => listing,
                // Move the first error in marker order, without requiring errors to be Clone.
                Err(_) => return Err(listings.into_iter().nth(index).unwrap().unwrap_err()),
            };
            state.records = listing.records.clone();
            state.omitted = listing.omitted;
            state.total = listing.total;
            state.scope_hint = listing.scope_hint.clone();
            state.newest = listing.ordered && listing.total > listing.records.len();
            if m.opens_argument && !kind.path_kind {
                state.records.retain(|r| {
                    let keep = !r.handle.as_bytes().starts_with(b"-");
                    state.omitted += usize::from(!keep);
                    keep
                });
            }
            if state.records.len() > limit && !listing.ordered {
                return Err(JevifyError::Kinded {
                    kind: "too_many",
                    exit: Exit::Input,
                    message: format!(
                        "{} candidates exceed the fill capacity of {limit}",
                        state.records.len()
                    ),
                    hint: "narrow with a literal path prefix, or pipe a narrower list into '@{-:description}'",
                    example: "grep pattern candidates | jevify fill -- command '@{-:description}'",
                });
            }
            if state.records.is_empty() {
                // An empty listing is not a judgment: nothing was there to choose from.
                state.reason = Some(NO_MATCH);
                state.detail = format!(
                    "no {} to choose from",
                    if m.kind == "-" {
                        "record"
                    } else {
                        m.kind.as_str()
                    }
                );
            }
        } else if insufficient {
            state.reason = Some(INSUFFICIENT_EVIDENCE);
        }
        states.push(state);
    }
    // An oversized context cannot authorize any request on a partial input.
    if insufficient {
        for state in &mut states {
            state.reason.get_or_insert(INSUFFICIENT_EVIDENCE);
        }
        return finish(ctx, &flags, &args, &markers, states, stdin_null, machine);
    }
    let client = Client::new(ctx)?;
    let prompts = Prompts {
        choose: "Which item best satisfies request? Select NONE when no item fits.".into(),
        none: "No item satisfies the request".into(),
        any: "Does any item satisfy request?".into(),
    };
    let items: Vec<Vec<String>> = states
        .iter()
        .map(|s| s.records.iter().map(|r| r.evidence.clone()).collect())
        .collect();
    let mut questions = Questions::new();
    for (i, m) in markers.iter().enumerate() {
        if m.kind == "one" {
            let mut criteria: BTreeMap<_, _> = m
                .options
                .iter()
                .enumerate()
                .map(|(i, option)| (format!("L{i:03}"), Some(option.clone())))
                .collect();
            criteria.insert("NONE".into(), Some("None of these options fits".into()));
            questions.insert(format!("m{i}"), Question::choice(&m.description, criteria));
        } else if m.kind == "flag" {
            questions.insert(format!("m{i}"), Question::noul(&m.description));
        }
    }
    let context_state = json!(context);
    let listing_round = join_all(markers.iter().enumerate().map(|(i, m)| {
        let client = &client;
        let prompts = &prompts;
        let texts = &items[i];
        async move {
            if texts.is_empty() {
                Ok(None)
            } else {
                tournament::shortlist(client, &m.description, texts, prompts, Finalists::ThreeOnly)
                    .await
                    .map(Some)
            }
        }
    }));
    let questions: Vec<_> = questions.into_iter().collect();
    let batches: Vec<Questions> = questions
        .chunks(20)
        .map(|chunk| chunk.iter().cloned().collect())
        .collect();
    let context_round = join_all(
        batches
            .iter()
            .map(|batch| client.ask(&context_state, batch)),
    );
    let (shortlists, answers) = futures::join!(listing_round, context_round);
    let mut errors = Vec::new();
    let mut shortlists: Vec<Option<Shortlist>> = shortlists
        .into_iter()
        .enumerate()
        .map(|(i, result)| match result {
            Ok(value) => value,
            Err(error) => {
                errors.push((i, error));
                None
            }
        })
        .collect();
    let mut merged: Option<jev::Response> = None;
    for (batch, result) in batches.iter().zip(answers) {
        match result {
            Ok(answer) => {
                if let Some(merged) = &mut merged {
                    merged.answers.extend(answer.answers);
                } else {
                    merged = Some(answer);
                }
            }
            Err(error) => {
                let index = batch
                    .keys()
                    .filter_map(|key| key.strip_prefix('m')?.parse::<usize>().ok())
                    .min()
                    .unwrap();
                errors.push((index, error));
            }
        }
    }
    if let Some((_, error)) = errors.into_iter().min_by_key(|(index, _)| *index) {
        return Err(error);
    }
    let answers = merged;
    if answers.is_some() || shortlists.iter().any(Option::is_some) {
        guard_model(ctx)?;
    }
    for (i, m) in markers.iter().enumerate() {
        if let Some(answer) = &answers {
            if m.kind == "flag" {
                let p = answer.noul(&format!("m{i}"))?;
                ctx.stats.gate(crate::output::Gate::noul(p));
                let (exit, verdict) = super::is::band_verdict(p, ctx.threshold, FLAG_BAND);
                states[i].p = Some(p);
                states[i].detail = format!(
                    "{}: {verdict} {p:.2}{}",
                    m.flag.as_deref().unwrap(),
                    if exit == Exit::No { ", left out" } else { "" }
                );
                if exit == Exit::Abstain {
                    states[i].reason = Some(UNSURE_FLAG);
                    states[i].detail.push_str(&format!(
                        "; write {} or drop the marker",
                        m.flag.as_deref().unwrap()
                    ));
                } else {
                    states[i].handle = Some(if exit == Exit::Ok {
                        m.flag.as_deref().unwrap().into()
                    } else {
                        OsString::new()
                    });
                }
            } else if m.kind == "one" {
                let probabilities = answer.probs(&format!("m{i}"))?;
                states[i].records = m
                    .options
                    .iter()
                    .map(|o| Record {
                        handle: o.into(),
                        evidence: o.clone(),
                        raw: 0..0,
                    })
                    .collect();
                states[i].total = m.options.len();
                let mut candidates: Vec<_> = m
                    .options
                    .iter()
                    .enumerate()
                    .map(|(index, _)| tournament::Candidate {
                        index,
                        p: probabilities[&format!("L{index:03}")],
                    })
                    .collect();
                candidates.sort_by(|a, b| b.p.total_cmp(&a.p));
                let ranking = Ranking {
                    candidates,
                    any: 1.0,
                    none: probabilities["NONE"],
                    windows: 1,
                    n: 3,
                };
                // `one` judges no Noul: only the options and NONE compete.
                ctx.stats.gate(crate::output::Gate {
                    any: None,
                    ..super::gate_of(&ranking)
                });
                apply_ranking(&mut states[i], &ranking, ctx.threshold);
            }
        }
    }
    let finals = join_all(markers.iter().enumerate().map(|(i, m)| {
        let first = shortlists[i].take();
        let records = &states[i].records;
        let client = &client;
        let prompts = &prompts;
        let env = &env;
        let tier_two = kinds[i].as_ref().is_some_and(|k| k.has_tier_two);
        let names_may_decide = kinds[i].as_ref().is_some_and(|k| names_may_decide(&k.name));
        let prefix = prefix_of(i).unwrap_or_default();
        async move {
            let Some(first) = first else {
                return Ok(None);
            };
            let mut finalists = first.finalists.clone();
            if first.windows.len() == 1 {
                let ranking = &first.windows[0];
                // Names alone cannot refute a content phrase. A `file` or `dir` marker always
                // runs its finals: on the held-out set of `evals/fill/finals/` a decisive
                // names round with the runner-up out of play still chose a file named for
                // the concept and holding something else (src/wrapping.rs for code that
                // lives in src/printer.rs), once per twelve to fifteen fires on each backend,
                // and fill is the verb whose selection reaches a command. `branch` and
                // `commit` keep the shortcut, a decisive Found with the runner-up out of play,
                // because no held-out evidence exists for them yet.
                if !tier_two
                    || (names_may_decide
                        && matches!(
                            tournament::decide(ranking, ctx.threshold),
                            Decision::Found(_)
                        )
                        && !runner_up_matches(ranking))
                {
                    return Ok(Some((ranking.clone(), 0, first, vec![])));
                }
                // The names ranked a content phrase's file low, not out: every candidate the
                // names did not rule out reaches the finals with its excerpt, as many as the
                // finals hold. Three names alone left the finals choosing the best of three
                // wrong files, and a related excerpt then won at 0.94.
                //
                // A commit keeps its zeroes as well. Its names round reads a subject line,
                // which is a claim about a change rather than the change; on the lying-subject
                // cases of `evals/commit-subjects/` the subject that announces the change
                // takes the whole mass and the commit that holds it is scored 0.00, so a
                // filter on p alone leaves the finals one candidate and no way back.
                let keep_zeroes = m.kind == "commit";
                finalists = ranking
                    .candidates
                    .iter()
                    .filter(|c| keep_zeroes || c.p > 0.0)
                    .take(MAX_FINALISTS)
                    .copied()
                    .collect();
                if finalists.is_empty() {
                    return Ok(Some((ranking.clone(), 0, first, vec![])));
                }
            }
            // The finals as sent, for `round_one`: widened past the shortlist's picks here.
            let judged: Vec<usize> = finalists.iter().map(|c| c.index).collect();
            let handles: Vec<_> = finalists
                .iter()
                .take(MAX_FINALISTS)
                .map(|c| records[c.index].handle.clone())
                .collect();
            let (evidence, withheld) = if tier_two {
                source::enrich_in(&m.kind, &prefix, &handles, env).await
            } else {
                (vec![], 0)
            };
            let items: Vec<_> = finalists
                .iter()
                .enumerate()
                .map(|(rank, c)| {
                    let mut text = records[c.index].evidence.clone();
                    if let Some(extra) = evidence.get(rank) {
                        text.push('\n');
                        text.push_str(extra);
                    }
                    (c.index, text)
                })
                .collect();
            let mut ranking = tournament::window(client, &m.description, &items, prompts).await?;
            ranking.windows = first.windows.len();
            Ok::<_, JevifyError>(Some((ranking, withheld, first, judged)))
        }
    }))
    .await;
    // In marker order: the rounds ran concurrently, and the envelope keeps decision order.
    for (i, ranking) in finals.into_iter().enumerate() {
        if let Some((ranking, withheld, first, judged)) = ranking? {
            first.record(&ctx.stats, &judged, |i| i + 1);
            states[i].withheld = withheld;
            ctx.stats.gate(super::gate_of(&ranking));
            apply_ranking(&mut states[i], &ranking, ctx.threshold);
        }
    }
    if answers.is_some() || items.iter().any(|items| !items.is_empty()) {
        guard_model(ctx)?;
    }
    finish(ctx, &flags, &args, &markers, states, stdin_null, machine)
}

#[derive(Default)]
struct State {
    records: Vec<Record>,
    total: usize,
    omitted: usize,
    /// An ordered listing above the limit kept its newest part.
    newest: bool,
    scope_hint: Option<String>,
    /// Finalists whose excerpt the withholding policy kept out of round two.
    withheld: usize,
    handle: Option<OsString>,
    reason: Option<&'static str>,
    p: Option<f64>,
    detail: String,
    /// The nearest candidates of an abstention, best first: `data.shortlist`'s entries, in
    /// `pick`'s shape. Empty when nothing was scored.
    closest: Vec<(String, f64)>,
}

/// `data.shortlist` of an abstention: `[{text, p}]`, best first.
fn shortlist_json(closest: &[(String, f64)]) -> serde_json::Value {
    closest
        .iter()
        .map(|(text, p)| json!({ "text": text, "p": p }))
        .collect()
}

impl State {
    /// `candidates N`, `candidates N of M` when the listing was cut, `newest first` when the
    /// cut kept the head of an ordered listing, and the omitted count when there is one.
    fn count_line(&self) -> String {
        let mut line = format!("candidates {}", self.records.len());
        if self.total > self.records.len() {
            line.push_str(&format!(" of {}", self.total));
            if self.newest {
                line.push_str(", newest first");
            }
        }
        if self.omitted > 0 {
            line.push_str(&format!(", omitted {}", self.omitted));
        }
        line
    }
}

fn guard_model(ctx: &Config) -> Result<(), JevifyError> {
    let model = ctx.meta().model.unwrap_or_else(|| "unknown".into());
    if !jev::all_jev(&model) {
        return Err(JevifyError::Unavailable(format!(
            "answered by {model}, not Jev"
        )));
    }
    Ok(())
}

/// The runner-up name also matches the phrase when the winner of the names round does not
/// beat the field it leads, the runner-up and every other name together with NONE, by the
/// winner ratio: `RIVAL_RATIO * (none + sum of the other names) > best.p`, the same ratio
/// that lets one rival or NONE block the best in `tournament::decide`. A content phrase leaves
/// the winner about half the mass ("the code that splits input into records": input.rs 0.51
/// to 0.59, records.rs 0.14 to 0.20, NONE 0.15 to 0.18), a name phrase leaves it nearly all
/// ("the workflow that publishes to crates.io": publish-crates.yml 0.96 to 0.98, NONE and
/// release.sh 0.01 to 0.03). The runner-up is measured with the field, not against NONE
/// alone: the backend reports two decimals, and a runner-up at 0.01 against NONE at 0.01 is
/// noise that sent the withheld `.github/` finalist into a finals it lost on its name. Only
/// when the field is in play does a decisive Found on names go on to the finals, where the
/// excerpts decide; otherwise the one-request path stands and a finalist whose excerpt would
/// be withheld never competes on its name alone. Read from round one only: no request, no
/// threshold change, no new ratio. Applied to the kinds `names_may_decide` names.
fn runner_up_matches(ranking: &Ranking) -> bool {
    let Some(best) = ranking.candidates.first() else {
        return false;
    };
    let field: f64 = ranking.none + ranking.candidates[1..].iter().map(|c| c.p).sum::<f64>();
    RIVAL_RATIO * field > best.p
}

/// The tier-two kinds whose names round may decide alone, a decisive Found with the runner-up
/// out of play (`runner_up_matches`): `branch`, whose refs carry nothing but a name until the
/// finals. `file` and `dir` always run their finals: on the 33 held-out content phrases of
/// `evals/fill/finals/` the shortcut chose a name decoy once per twelve to fifteen fires on
/// each backend (benchmarks/results.md), and a wrong file reaches the caller's command.
/// `commit` always runs its finals too: a subject line is a claim about a change, and on the
/// 20 cases of `evals/commit-subjects/`, where the claim sits on a commit that does not hold
/// the change, the shortcut fired on every case and answered wrong at 0.82 to 1.00 on both
/// backends. The finals read the patch, so they can tell the claim from the change.
fn names_may_decide(kind: &str) -> bool {
    matches!(kind, "branch")
}

fn apply_ranking(state: &mut State, ranking: &Ranking, threshold: f64) {
    // Emitted only on an abstention, the duplicate-evidence one included.
    state.closest = super::closest(ranking, |i| {
        state.records[i].handle.to_string_lossy().into_owned()
    });
    match tournament::decide(ranking, threshold) {
        Decision::Found(best) => {
            let record = &state.records[best.index];
            let duplicates = state
                .records
                .iter()
                .filter(|r| r.evidence == record.evidence)
                .count();
            if duplicates > 1 {
                state.reason = Some(AMBIGUOUS);
                state.detail = format!("{duplicates} candidates share the same evidence");
                return;
            }
            state.handle = Some(record.handle.clone());
            state.p = Some(best.p);
            state.detail = format!(
                "{} {:.2} (next {:.2}, none {:.2}); {}, windows {}{}",
                record.handle.to_string_lossy(),
                best.p,
                ranking.candidates.get(1).map_or(0.0, |c| c.p),
                ranking.none,
                state.count_line(),
                ranking.windows,
                if state.withheld > 0 {
                    format!(", excerpts withheld: {}", state.withheld)
                } else {
                    String::new()
                }
            );
        }
        Decision::NoMatch => {
            state.reason = Some(NO_MATCH);
            let top: Vec<_> = ranking.candidates.iter().take(2).collect();
            state.detail = closest_line(state, &top, ranking.none);
        }
        Decision::Ambiguous(closest) => {
            state.reason = Some(AMBIGUOUS);
            // The best, then every rival close enough to have blocked it, NONE included.
            let best = closest[0].p;
            let rivals: Vec<_> = closest
                .iter()
                .enumerate()
                .filter(|(rank, c)| *rank == 0 || RIVAL_RATIO * c.p >= best)
                .map(|(_, c)| c)
                .collect();
            let none = (RIVAL_RATIO * ranking.none >= best).then_some(ranking.none);
            state.detail = closest_line(state, &rivals, none);
        }
    }
}

/// What a caller can change after an abstention, by its reason.
fn abstention_hint(reason: &str) -> &'static str {
    match reason {
        AMBIGUOUS => "add the detail that separates the nearest candidates",
        UNSURE_FLAG => "state the condition the flag depends on literally, or decide it yourself",
        INSUFFICIENT_EVIDENCE => {
            "the context is over the evidence budget: pass the part that matters with --context FILE"
        }
        _ => {
            "describe what the thing itself says (its words, not your goal), or narrow the listing with a literal prefix"
        }
    }
}

/// A `git` command line read up to its subcommand: the directory its global options name
/// (every `-C DIR`, each relative to the one before, as git reads them; `None` for no `-C` or
/// one whose value holds a marker) and the subcommand. `None` for any other program.
fn git_invocation(cmd: &[OsString]) -> Option<(Option<PathBuf>, Option<String>)> {
    let program = Path::new(cmd.first()?);
    if program.file_name()? != "git" {
        return None;
    }
    let mut dir: Option<PathBuf> = None;
    let mut marked = false;
    let mut args = cmd[1..].iter();
    let mut subcommand = None;
    while let Some(arg) = args.next() {
        let Some(text) = arg.to_str() else {
            break;
        };
        match text {
            "-C" => {
                let Some(value) = args.next() else {
                    break;
                };
                marked |= value.to_string_lossy().contains("@{");
                dir = Some(match dir {
                    Some(base) => base.join(value),
                    None => PathBuf::from(value),
                });
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--config-env" => {
                args.next();
            }
            _ if text.starts_with('-') => {}
            _ => {
                subcommand = Some(text.to_owned());
                break;
            }
        }
    }
    Some((dir.filter(|_| !marked), subcommand))
}

/// The directory a `git` command runs in when its global options name one.
fn git_directory(cmd: &[OsString]) -> Option<PathBuf> {
    git_invocation(cmd)?.0
}

/// Git subcommands that read a revision and change nothing: the ones for which a remote-only
/// branch must be spelled as its remote ref. `switch`, `checkout`, `push` and `branch` take the
/// short name (hunch-x4w), so they are not here.
const REVISION_READERS: [&str; 16] = [
    "log",
    "show",
    "diff",
    "rev-parse",
    "rev-list",
    "blame",
    "shortlog",
    "describe",
    "merge-base",
    "ls-tree",
    "cat-file",
    "name-rev",
    "range-diff",
    "format-patch",
    "whatchanged",
    "cherry",
];

/// Whether `name` resolves to a commit in `dir`, read under the lister deadline.
fn resolves(dir: &Path, name: &OsStr) -> bool {
    let mut revision = name.to_os_string();
    revision.push("^{commit}");
    let mut git = std::process::Command::new("git");
    git.current_dir(dir)
        .args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
        .arg(revision);
    let deadline = std::time::Instant::now() + source::LISTER_TIMEOUT;
    source::supervise(git, source::Mode::Strict, deadline, usize::MAX).is_ok()
}

/// A bare `@{branch:...}` substitutes the short name, which only `git switch`'s DWIM resolves
/// for a branch that exists only on a remote. For a git subcommand that reads a revision, the
/// short name is a revision git cannot find, so the remote ref the listing showed replaces it
/// when the short name does not resolve and the remote ref does.
fn remote_spelling(
    cmd: &[OsString],
    marker: &Marker,
    state: &State,
    cwd: &Path,
) -> Option<OsString> {
    if marker.kind != "branch" || !marker.prefix.is_empty() {
        return None;
    }
    let (dir, subcommand) = git_invocation(cmd)?;
    if !REVISION_READERS.contains(&subcommand?.as_str()) {
        return None;
    }
    let handle = state.handle.as_ref()?;
    let shown = state
        .records
        .iter()
        .find(|r| &r.handle == handle)?
        .evidence
        .split(" — ")
        .next()?;
    let shown = OsString::from(shown);
    let dir = cwd.join(dir.unwrap_or_default());
    (&shown != handle && !resolves(&dir, handle) && resolves(&dir, &shown)).then_some(shown)
}

/// `nearest (not chosen): a (0.42), none (0.30), b (0.08)`: the named candidates and NONE, by
/// probability, labelled so that no caller takes one for the answer.
fn closest_line(
    state: &State,
    candidates: &[&tournament::Candidate],
    none: impl Into<Option<f64>>,
) -> String {
    let mut entries: Vec<(String, f64)> = candidates
        .iter()
        .map(|c| {
            (
                state.records[c.index].handle.to_string_lossy().into_owned(),
                c.p,
            )
        })
        .collect();
    if let Some(none) = none.into() {
        entries.push(("none".into(), none));
    }
    entries.sort_by(|a, b| b.1.total_cmp(&a.1));
    format!(
        "nearest (not chosen): {}",
        entries
            .iter()
            .map(|(name, p)| format!("{name} ({p:.2})"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn finish(
    ctx: &Config,
    flags: &FillFlags,
    args: &[marker::Arg],
    markers: &[&Marker],
    mut states: Vec<State>,
    stdin_null: bool,
    machine: bool,
) -> Result<Outcome, JevifyError> {
    let reason = states.iter().find_map(|s| s.reason);
    if reason.is_none() {
        let cmd: Vec<OsString> = args.iter().map(|a| a.literal.clone()).collect();
        let cwd = std::env::current_dir().unwrap_or_default();
        for (m, state) in markers.iter().zip(states.iter_mut()) {
            let short = state.handle.clone().unwrap_or_default();
            if let Some(remote) = remote_spelling(&cmd, m, state, &cwd) {
                if !flags.quiet {
                    eprintln!(
                        "jevify fill: branch {} exists only as {}, the spelling a revision needs",
                        output::status_escape(&short.to_string_lossy()),
                        output::status_escape(&remote.to_string_lossy())
                    );
                }
                state.handle = Some(remote);
            }
        }
    }
    let model = ctx.meta().model.unwrap_or_else(|| "not requested".into());
    for (m, state) in markers.iter().zip(&states) {
        let scope = state.scope_hint.as_ref().map_or(String::new(), |hint| {
            format!("; {}", output::status_escape(hint))
        });
        if state.reason.is_none() && !flags.quiet {
            eprintln!(
                "jevify fill: {} {}; model {}{scope}",
                m.kind,
                output::status_escape(&state.detail),
                output::status_escape(&model)
            );
        }
    }
    for (m, state) in markers.iter().zip(&states) {
        if let Some(reason) = state.reason {
            let scope = state.scope_hint.as_ref().map_or(String::new(), |hint| {
                format!("; {}", output::status_escape(hint))
            });
            let detail = if state.detail.is_empty() {
                String::new()
            } else {
                format!("{}; ", output::status_escape(&state.detail))
            };
            let hint = if flags.quiet {
                String::new()
            } else {
                format!("; hint: {}", abstention_hint(reason))
            };
            eprintln!(
                "jevify fill: not run: arg {} {}: {reason}; {detail}candidates {} of {}, omitted {}; model {}{hint}{scope}",
                m.argv_index + 1,
                m.kind,
                state.records.len(),
                state.total,
                state.omitted,
                output::status_escape(&model)
            );
        }
    }
    let mut data = json!({"reason": reason, "markers": markers.iter().zip(&states).map(|(m,s)| json!({
        "arg": m.argv_index + 1, "kind": m.kind, "reason": s.reason,
        "handle": s.handle.as_ref().map(|h| h.to_string_lossy()), "p": s.p,
        "candidates": s.records.len(), "total": s.total, "omitted": s.omitted,
        "shortlist": s.reason.map(|_| shortlist_json(&s.closest)),
    })).collect::<Vec<_>>()});
    if reason.is_some() {
        // The nearest candidates of the marker `reason` names: a list on every abstention,
        // empty when that marker scored nothing (no listing, an unsure flag, an oversized
        // context). Candidates are not answers.
        let first = states.iter().find(|s| s.reason.is_some()).unwrap();
        data["shortlist"] = shortlist_json(&first.closest);
        return Ok(Outcome {
            exit: Exit::Abstain,
            data,
            human: vec![],
            exec: None,
        });
    }
    let handles: Vec<_> = states
        .into_iter()
        .map(|s| s.handle.expect("every marker resolved"))
        .collect();
    let argv = marker::substitute(args, &handles).map_err(|e| JevifyError::Usage(e.to_string()))?;
    if machine && argv.iter().any(|arg| arg.to_str().is_none()) {
        return Err(JevifyError::cannot_run(
            "machine output cannot represent non-UTF-8 argv".into(),
        ));
    }
    data["argv"] = json!(argv.iter().map(|s| s.to_string_lossy()).collect::<Vec<_>>());
    let mut quoted = output::shell_quote(&argv);
    if !flags.quiet {
        let verb = if flags.dry_run { "would run" } else { "exec" };
        eprintln!(
            "jevify fill: {verb} {}",
            output::status_escape(&String::from_utf8_lossy(&quoted))
        );
    }
    if flags.dry_run {
        quoted.push(b'\n');
        Ok(Outcome {
            exit: Exit::Ok,
            data,
            human: quoted,
            exec: None,
        })
    } else {
        Ok(Outcome {
            exit: Exit::Ok,
            data,
            human: vec![],
            exec: Some(Exec { argv, stdin_null }),
        })
    }
}

fn check_stdin_terminal(is_terminal: bool) -> Result<(), JevifyError> {
    if is_terminal {
        Err(JevifyError::stdin_is_tty(
            "pipe candidates or context into fill, or supply a file".into(),
        ))
    } else {
        Ok(())
    }
}

async fn read_input(path: Option<PathBuf>) -> Result<Vec<u8>, JevifyError> {
    tokio::task::spawn_blocking(move || {
        let Some(path) = path else {
            check_stdin_terminal(std::io::stdin().is_terminal())?;
            return match input::read_stdin_bytes() {
                Err(JevifyError::EmptyInput(_)) => Ok(vec![]),
                result => result,
            };
        };
        let file = std::fs::File::open(&path)
            .map_err(|e| JevifyError::Input(format!("{}: {e}", path.display())))?;
        let mut bytes = vec![];
        file.take(input::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| JevifyError::Input(e.to_string()))?;
        if bytes.len() > input::MAX_BYTES {
            return Err(JevifyError::InputTooLarge("input exceeds 64 MiB".into()));
        }
        Ok(bytes)
    })
    .await
    .map_err(|e| JevifyError::Input(e.to_string()))?
}

fn check_program(program: &OsStr) -> Result<(), JevifyError> {
    let executable = |path: &Path| {
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let found = if program.as_bytes().contains(&b'/') {
        executable(Path::new(program))
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .any(|dir| executable(&dir.join(program)))
    };
    if found && !program.as_bytes().contains(&0) {
        Ok(())
    } else {
        Err(JevifyError::cannot_run(format!(
            "{} is not an executable on PATH",
            program.to_string_lossy()
        )))
    }
}

/// The kind of a listing marker; `None` for `one` and `flag`. A name that is neither coded nor
/// shipped is looked up in the user's recipes; a name found nowhere is exit 2 with the nearest
/// kind, and a bad recipe file is `recipe_invalid` with its line number.
fn validate_kind(
    marker: &Marker,
    argument: &OsStr,
    env: &source::Env,
) -> Result<Option<source::Kind>, JevifyError> {
    if matches!(marker.kind.as_str(), "one" | "flag") {
        return Ok(None);
    }
    if let Some(kind) = source::lookup(&marker.kind, env)? {
        return Ok(Some(kind));
    }
    let (nearest, kinds) = super::pick::nearest_kind(&marker.kind, &["one", "flag"], env);
    let mut literal = argument.as_bytes().to_vec();
    literal.insert(marker.span.start, b'@');
    let literal = output::status_escape(&String::from_utf8_lossy(&literal)).replace('\'', "'\\''");
    Err(JevifyError::Usage(format!(
        "unknown kind '{}'; nearest kind: {nearest}; kinds: {kinds}; for a literal write '{literal}'",
        marker.kind
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_directory_follows_every_dash_c_before_the_subcommand() {
        let argv = |a: &[&str]| a.iter().map(OsString::from).collect::<Vec<_>>();
        let dir = |a: &[&str]| git_directory(&argv(a));
        assert_eq!(
            dir(&["git", "-C", "work/hyper", "show", "@{commit:x}"]),
            Some(PathBuf::from("work/hyper"))
        );
        assert_eq!(
            dir(&[
                "/usr/bin/git",
                "-C",
                "/abs",
                "-c",
                "a=b",
                "-C",
                "sub",
                "log"
            ]),
            Some(PathBuf::from("/abs/sub"))
        );
        assert_eq!(
            dir(&["git", "--no-pager", "-C", "r", "log"]),
            Some(PathBuf::from("r"))
        );
        // After the subcommand, -C belongs to it (git log has none, but grep -C is context).
        assert_eq!(dir(&["git", "grep", "-C", "3", "x"]), None);
        assert_eq!(dir(&["git", "show", "@{commit:x}"]), None);
        assert_eq!(dir(&["gitx", "-C", "r", "show"]), None);
        assert_eq!(dir(&["git", "-C", "@{dir:the repo}", "show"]), None);
        assert_eq!(dir(&["git", "-C"]), None);
        assert_eq!(dir(&[]), None);
    }

    #[test]
    fn terminal_requires_explicit_input() {
        assert!(check_stdin_terminal(false).is_ok());
        let error = check_stdin_terminal(true).unwrap_err();
        assert_eq!(error.exit(), Exit::Input);
        assert_eq!(error.kind(), "stdin_is_tty");
    }

    #[test]
    fn ranking_requires_fit_ratio_and_distinct_evidence() {
        for (best, second, none, any, expected) in [
            (0.9, 0.05, 0.05, 0.9, None),
            (0.45, 0.45, 0.1, 0.9, Some(AMBIGUOUS)),
            (0.5, 0.1, 0.4, 0.9, Some(AMBIGUOUS)),
            (0.9, 0.05, 0.05, 0.1, Some(NO_MATCH)),
            (0.1, 0.1, 0.8, 0.9, Some(NO_MATCH)),
        ] {
            let mut state = State {
                records: vec![
                    Record {
                        handle: "a".into(),
                        evidence: "first".into(),
                        raw: 0..0,
                    },
                    Record {
                        handle: "b".into(),
                        evidence: "second".into(),
                        raw: 0..0,
                    },
                ],
                total: 2,
                ..State::default()
            };
            let ranking = Ranking {
                candidates: vec![
                    tournament::Candidate { index: 0, p: best },
                    tournament::Candidate {
                        index: 1,
                        p: second,
                    },
                ],
                any,
                none,
                windows: 1,
                n: 3,
            };
            apply_ranking(&mut state, &ranking, 0.5);
            assert_eq!(state.reason, expected);
            assert_eq!(state.handle.is_some(), expected.is_none());
            if expected.is_none() {
                state.handle = None;
                state.records[1].evidence = "first".into();
                apply_ranking(&mut state, &ranking, 0.5);
                assert_eq!(state.reason, Some(AMBIGUOUS));
                assert!(state.handle.is_none());
                assert_eq!(state.detail, "2 candidates share the same evidence");
            }
        }
        let mut empty = State::default();
        apply_ranking(
            &mut empty,
            &Ranking {
                candidates: vec![],
                any: 1.0,
                none: 0.0,
                windows: 0,
                n: 3,
            },
            0.5,
        );
        assert_eq!(empty.reason, Some(NO_MATCH));
    }

    #[test]
    fn runner_up_matches_when_the_winner_does_not_beat_the_field_by_the_ratio() {
        let ranking = |best: f64, second: f64, rest: f64, none: f64| Ranking {
            candidates: vec![
                tournament::Candidate { index: 0, p: best },
                tournament::Candidate {
                    index: 1,
                    p: second,
                },
                tournament::Candidate { index: 2, p: rest },
            ],
            any: 0.9,
            none,
            windows: 1,
            n: 3,
        };
        // Measured names rounds: records.rs behind input.rs stays in play; release.sh at 0.01
        // behind publish-crates.yml, tied with NONE, does not.
        assert!(runner_up_matches(&ranking(0.51, 0.20, 0.12, 0.17)));
        assert!(!runner_up_matches(&ranking(0.98, 0.01, 0.0, 0.01)));
        // The boundary is the winner ratio: twice the field must exceed the winner.
        assert!(runner_up_matches(&ranking(0.66, 0.20, 0.10, 0.04)));
        assert!(!runner_up_matches(&ranking(0.68, 0.20, 0.10, 0.02)));
        let mut single = ranking(0.5, 0.0, 0.0, 0.5);
        single.candidates.truncate(1);
        assert!(runner_up_matches(&single));
        single.candidates.clear();
        assert!(!runner_up_matches(&single));
    }

    #[test]
    fn program_validation_rejects_missing_nonexecutables_and_nul() {
        assert!(check_program(OsStr::new("/bin/sh")).is_ok());
        for program in ["", "/", "/not/a/jevify/program", "/bin/sh\0"] {
            let error = check_program(OsStr::new(program)).unwrap_err();
            assert_eq!(error.exit(), Exit::Input);
            assert_eq!(error.kind(), "cannot_run");
        }
    }
}
