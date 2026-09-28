use crate::cmd::Outcome;
use crate::config::Config;
use crate::exit::{Exit, JevifyError};
use crate::jev::client::Client;
use crate::records::{self, Split};
use crate::source::{self, Scope};
use crate::tournament::{Decision, decide};
use crate::tournament::{Finalists, Prompts, Ranking, shortlist, window};
use std::os::unix::ffi::OsStrExt;

/// 100k lines would be 500 requests: a 429 storm and ~25 s. pick ranks a list, it does not scan.
pub const MAX_LINES: usize = 20_000;
const MAX_FINALISTS: usize = 24;

pub async fn run(
    ctx: &Config,
    intent: &str,
    top: usize,
    index: bool,
    split: Split,
    files: bool,
    from: Option<&str>,
) -> Result<Outcome, JevifyError> {
    // `--from -` names the default source, stdin.
    if let Some(kind) = from.filter(|kind| *kind != "-") {
        return from_kind(ctx, intent, top, kind).await;
    }
    if top == 0 {
        return Err(JevifyError::Usage("-n must be at least 1".into()));
    }
    if files && index {
        return Err(JevifyError::Usage(
            "--index numbers stdin lines; with --files the match is a path".into(),
        ));
    }
    let client = Client::new(ctx)?;
    let read = tokio::task::spawn_blocking(crate::input::read_stdin_bytes)
        .await
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    // `--files` with nothing on stdin (a terminal, or an upstream command that printed nothing)
    // ranks the files of the working directory: what `git ls-files | jevify pick --files` does.
    let (bytes, split) = match read {
        Ok(bytes) if files && bytes.iter().all(u8::is_ascii_whitespace) => {
            (working_directory_files().await?, Split::Lines)
        }
        Err(JevifyError::EmptyInput(_)) if files => {
            (working_directory_files().await?, Split::Lines)
        }
        other => (other?, split),
    };
    let records = records::parse(&bytes, split)?;
    if records.is_empty() {
        return Err(JevifyError::EmptyInput("stdin was empty"));
    }
    // Blank lines never leave the machine: as empty items they would take window slots and tokens.
    // A repeated line is sent once (the first occurrence keeps its line number): two identical
    // items split the Choice mass between them.
    let (kept, _) = records::distinct(&bytes, &records);
    if kept.len() > MAX_LINES {
        return Err(JevifyError::InputTooLarge(if files {
            format!("more than {MAX_LINES} files; narrow the input list")
        } else {
            format!("more than {MAX_LINES} lines; filter first (rg, head) or split the list")
        }));
    }
    let items: Vec<String> = kept.iter().map(|&i| records[i].evidence.clone()).collect();
    eprintln!(
        "jevify pick: candidates {}, windows {}",
        kept.len(),
        kept.len().div_ceil(ctx.backend.window())
    );
    let prompts = if files {
        Prompts {
            choose: "Each entry in `items` is a file: its path, and for some entries the beginning of its content. Which file is the one described by `request`? Choose NONE if no file matches.".into(),
            none: "no file in the list matches the request".into(),
            any: "Is at least one file in `items` the one described by `request`?".into(),
        }
    } else {
        Prompts {
            choose: "Which entry in `items` is the one described by `request`? Choose NONE if no entry matches.".into(),
            none: "no entry in the list matches the request".into(),
            any: "Is at least one entry in `items` the thing described by `request`?".into(),
        }
    };
    let ranking = if files {
        let short = shortlist(&client, intent, &items, &prompts, Finalists::Auto).await?;
        let windows = short.windows.len();
        let pool = short.finalists.clone();
        if pool.iter().all(|candidate| candidate.p == 0.0) {
            short.record(&ctx.stats, &[], |i| kept[i] + 1);
            Ranking {
                candidates: vec![],
                any: 0.0,
                none: 1.0,
                windows,
                n: short.n,
            }
        } else {
            // Every finalist reaches the finals; the excerpts stop at MAX_FINALISTS.
            let judged: Vec<usize> = pool.iter().map(|c| c.index).collect();
            short.record(&ctx.stats, &judged, |i| kept[i] + 1);
            let mut finalists: Vec<_> = pool
                .iter()
                .map(|c| records[kept[c.index]].clone())
                .collect();
            let cwd = std::env::current_dir().map_err(|e| JevifyError::Input(e.to_string()))?;
            let evidence_count = finalists.len().min(MAX_FINALISTS);
            let unread = records::excerpts(&mut finalists[..evidence_count], &cwd).await?;
            eprintln!("jevify pick: excerpts withheld: {}", unread.count);
            unread.report("pick", &finalists);
            let finals: Vec<_> = pool
                .iter()
                .zip(finalists)
                .map(|(c, r)| (c.index, r.evidence))
                .collect();
            let mut ranking = window(&client, intent, &finals, &prompts).await?;
            ranking.windows = windows;
            ranking.n = short.n;
            ranking
        }
    } else {
        // Round one ranks each window on its own; the gold wins its window. The finals put the
        // gold next to its siblings (a documentation page, a test, a recording of the same
        // thing), and a plain listing shows only names, so the finals say which sibling a
        // description of behaviour means.
        let finals = Prompts {
            choose: "Which entry in `items` is the one described by `request`? Several entries may concern the same thing: the entry that is or does what `request` describes beats a page that documents it, a test of it or a recording of it, unless `request` asks for documentation, a test or a recording. Choose NONE if no entry matches.".into(),
            none: prompts.none.clone(),
            any: prompts.any.clone(),
        };
        let short = shortlist(&client, intent, &items, &prompts, Finalists::Auto).await?;
        let windows = short.windows.len();
        // One window decides alone; otherwise the shortlist's picks are the finals.
        let judged: Vec<usize> = if windows == 1 {
            vec![]
        } else {
            short.finalists.iter().map(|c| c.index).collect()
        };
        short.record(&ctx.stats, &judged, |i| kept[i] + 1);
        let single = if windows == 1 {
            short.windows.into_iter().next()
        } else {
            None
        };
        if let Some(only) = single {
            only
        } else if short.finalists.is_empty() {
            Ranking {
                candidates: vec![],
                any: 0.0,
                none: 1.0,
                windows,
                n: short.n,
            }
        } else {
            let pool: Vec<_> = short
                .finalists
                .iter()
                .map(|c| (c.index, items[c.index].clone()))
                .collect();
            let mut ranking = window(&client, intent, &pool, &finals).await?;
            ranking.windows = windows;
            ranking.n = short.n;
            ranking
        }
    };
    if ranking.n != 3 {
        eprintln!("jevify pick: finalists per window: {}", ranking.n);
    }
    ctx.stats.gate(super::gate_of(&ranking));
    // Found only if the absolute Noul agrees and the best line beats NONE in the Choice.
    let found = ranking.any >= ctx.threshold
        && ranking
            .candidates
            .first()
            .is_some_and(|c| c.p > ranking.none);
    let mut selected: Vec<_> = if found {
        ranking
            .candidates
            .iter()
            .filter(|c| c.p > ranking.none)
            .take(top)
            .collect()
    } else {
        vec![]
    };
    selected.sort_by_key(|candidate| candidate.index);
    let mut human = Vec::new();
    let mut matches = Vec::new();
    for candidate in selected {
        let i = kept[candidate.index];
        let record = &records[i];
        let ordinal = match split {
            Split::Para => i + 1,
            Split::Lines | Split::Nul => {
                let delimiter = if split == Split::Nul { 0 } else { b'\n' };
                bytes[..record.raw.start]
                    .iter()
                    .filter(|&&b| b == delimiter)
                    .count()
                    + 1
            }
        };
        let mut value = records::envelope(&bytes, record, ordinal);
        // Keep the established text field for UTF-8 handles; lossy records use the raw helper.
        if let Some(text) = record.handle.to_str() {
            value["text"] = text.into();
        }
        value["line"] = ordinal.into();
        value["p"] = candidate.p.into();
        matches.push(value);
        if index {
            human.extend_from_slice(format!("{ordinal}\n").as_bytes());
        } else {
            human.extend_from_slice(&bytes[record.raw.clone()]);
        }
    }
    let mut data = serde_json::json!({ "matches": matches, "any": ranking.any, "source": if files { "files" } else { "stdin" } });
    if matches.is_empty() {
        let closest = super::closest(&ranking, |i| {
            records[kept[i]].handle.to_string_lossy().into_owned()
        });
        let hint = super::abstain_hint(&ranking, ctx.threshold, super::DESCRIBE_THE_RECORD);
        eprintln!(
            "{}",
            super::abstain_line("pick", crate::exit::NO_MATCH, &closest, &hint)
        );
        data["closest"] = closest_json(&closest);
        data["hint"] = hint.into();
    }
    Ok(Outcome {
        exit: if matches.is_empty() {
            Exit::Abstain
        } else {
            Exit::Ok
        },
        data,
        human,
        exec: None,
    })
}

async fn from_kind(
    ctx: &Config,
    intent: &str,
    top: usize,
    name: &str,
) -> Result<Outcome, JevifyError> {
    let env = source::Env::from_process(source::LISTER_TIMEOUT);
    // Coded kinds and shipped recipes, then the user's kinds.jsonl for a name in neither.
    let Some(kind) = source::lookup(name, &env)? else {
        let user: Vec<String> = source::catalog(&env)
            .kinds
            .into_iter()
            .filter(|entry| entry.origin == "user")
            .map(|entry| entry.name)
            .collect();
        let kinds: Vec<&str> = source::KINDS
            .iter()
            .copied()
            .chain(user.iter().map(String::as_str))
            .collect();
        let nearest = kinds
            .iter()
            .min_by_key(|candidate| distance(name, candidate))
            .copied()
            .unwrap_or("branch");
        return Err(JevifyError::Usage(format!(
            "unknown kind {name:?}; did you mean {nearest:?}? kinds: {}",
            kinds.join(", ")
        )));
    };
    if top == 0 {
        return Err(JevifyError::Usage("-n must be at least 1".into()));
    }
    let size = ctx.backend.window();
    let limit = (size * size).min(MAX_LINES);
    let listing = source::enumerate(name, Scope::Prefix(None), limit, &env).await?;
    let count = listing.records.len();
    let n = Finalists::Auto.per_window(count, size)?;
    if count > limit {
        return Err(JevifyError::Kinded {
            kind: "too_many",
            exit: Exit::Input,
            message: format!("{count} candidates exceed the limit of {limit}"),
            hint: "narrow with a prefix, or pipe a narrower list into stdin pick",
            example: "head -n 1000 candidates | jevify pick 'description'",
        });
    }
    let windows = count.div_ceil(size);
    if listing.ordered && listing.total > count {
        eprintln!(
            "jevify pick: candidates {count} of {}, newest first; windows {windows}",
            listing.total
        );
    } else {
        eprintln!("jevify pick: candidates {count}, windows {windows}");
    }
    if n != 3 {
        eprintln!("jevify pick: finalists per window: {n}");
    }
    let prompts = Prompts {
        choose: "Which entry in `items` is the one described by `request`? Choose NONE if no entry matches.".into(),
        none: "no entry in the list matches the request".into(),
        any: "Is at least one entry in `items` the thing described by `request`?".into(),
    };
    let mut ranking = Ranking {
        candidates: vec![],
        any: 0.0,
        none: 1.0,
        windows,
        n,
    };
    if count != 0 {
        let client = Client::new(ctx)?;
        let items: Vec<_> = listing.records.iter().map(|r| r.evidence.clone()).collect();
        let short = shortlist(&client, intent, &items, &prompts, Finalists::Auto).await?;
        if windows == 1 {
            ranking = short.windows[0].clone();
        }
        let finals_run = windows > 1
            || (kind.has_tier_two
                && !matches!(decide(&ranking, ctx.threshold), Decision::Found(_)));
        let judged: Vec<usize> = if finals_run {
            short.finalists.iter().map(|c| c.index).collect()
        } else {
            vec![]
        };
        short.record(&ctx.stats, &judged, |i| i + 1);
        if finals_run {
            let mut finals: Vec<_> = short
                .finalists
                .iter()
                .map(|c| (c.index, items[c.index].clone()))
                .collect();
            if kind.has_tier_two {
                let handles: Vec<_> = short
                    .finalists
                    .iter()
                    .take(MAX_FINALISTS)
                    .map(|c| listing.records[c.index].handle.clone())
                    .collect();
                let (evidence, withheld) =
                    source::enrich_in(name, std::path::Path::new(""), &handles, &env).await;
                for ((_, text), extra) in finals.iter_mut().zip(evidence) {
                    if !extra.is_empty() {
                        text.push('\n');
                        text.push_str(&extra);
                    }
                }
                if withheld > 0 {
                    eprintln!("jevify pick: excerpts withheld: {withheld}");
                }
            }
            ranking = window(&client, intent, &finals, &prompts).await?;
            ranking.windows = windows;
            ranking.n = n;
        }
    }
    ctx.stats.gate(super::gate_of(&ranking));
    let reason = match decide(&ranking, ctx.threshold) {
        Decision::Found(_) => None,
        Decision::NoMatch => Some(crate::exit::NO_MATCH),
        Decision::Ambiguous(_) => Some(crate::exit::AMBIGUOUS),
    };
    let mut matches = Vec::new();
    let mut human = Vec::new();
    let mut closest = vec![];
    let mut hint = None;
    if let Some(reason) = reason {
        closest = super::closest(&ranking, |i| {
            listing.records[i].handle.to_string_lossy().into_owned()
        });
        let text = super::abstain_hint(&ranking, ctx.threshold, super::DESCRIBE_THE_RECORD);
        eprintln!("{}", super::abstain_line("pick", reason, &closest, &text));
        hint = Some(text);
    } else {
        for candidate in ranking
            .candidates
            .iter()
            .filter(|c| c.p > ranking.none)
            .take(top)
        {
            let handle = &listing.records[candidate.index].handle;
            human.extend_from_slice(handle.as_bytes());
            human.push(b'\n');
            matches.push(serde_json::json!({"text": handle.to_string_lossy(), "lossy": handle.to_str().is_none(), "ordinal": candidate.index + 1, "p": candidate.p}));
        }
    }
    let mut data = serde_json::json!({"matches": matches, "reason": reason, "any": ranking.any, "source": name, "candidates": count, "total": listing.total, "omitted": listing.omitted, "windows": windows, "finalists_per_window": n});
    if let Some(hint) = hint {
        data["closest"] = closest_json(&closest);
        data["hint"] = hint.into();
    }
    Ok(Outcome {
        exit: if reason.is_some() {
            Exit::Abstain
        } else {
            Exit::Ok
        },
        data,
        human,
        exec: None,
    })
}

/// `data.closest` of an abstention: `[{text, p}]`, best first.
fn closest_json(closest: &[(String, f64)]) -> serde_json::Value {
    closest
        .iter()
        .map(|(text, p)| serde_json::json!({ "text": text, "p": p }))
        .collect()
}

/// The paths of the `file` kind under the current directory, one per line: tracked and
/// untracked files of a work tree with ignored ones excluded, or a no-follow walk outside one.
/// A path holding a newline is left out by the listing. Stated on stderr in one line.
async fn working_directory_files() -> Result<Vec<u8>, JevifyError> {
    let env = source::Env::from_process(source::LISTER_TIMEOUT);
    let listing = source::enumerate("file", Scope::Prefix(None), MAX_LINES + 1, &env).await?;
    if listing.records.is_empty() {
        return Err(JevifyError::EmptyInput(
            "no paths on stdin and no files under the current directory",
        ));
    }
    eprintln!(
        "jevify pick: no paths on stdin; ranking the {} files under the current directory (git ls-files, or a walk outside a work tree)",
        listing.records.len()
    );
    let mut bytes = Vec::new();
    for record in &listing.records {
        bytes.extend_from_slice(record.handle.as_bytes());
        bytes.push(b'\n');
    }
    Ok(bytes)
}

pub(crate) fn distance(a: &str, b: &str) -> usize {
    let mut row: Vec<_> = (0..=b.chars().count()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right) in b.chars().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(left != right));
            diagonal = above;
        }
    }
    *row.last().unwrap()
}

#[cfg(test)]
mod tests {
    #[test]
    fn kind_distance_handles_empty_unicode_and_edits() {
        for (a, b, expected) in [
            ("", "", 0),
            ("", "branch", 6),
            ("branc", "branch", 1),
            ("branch", "branch", 0),
            ("é", "a", 1),
        ] {
            assert_eq!(super::distance(a, b), expected);
        }
    }
}
