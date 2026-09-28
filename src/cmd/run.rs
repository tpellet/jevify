use crate::cmd::Outcome;
use crate::config::Config;
use crate::exit::{Exit, JevifyError};
use crate::inventory::{self, Tool};
use crate::jev::client::Client;
use crate::jev::{Question, Questions};
use crate::manpage;
use crate::tournament::{Finalists, Prompts, shortlist};

/// Two fits closer than this are a tie, and a tie is an abstention: the absolute fit of a
/// command is a Noul of its own request, so several commands that serve one task all score
/// high, and a gap this small is noise, not an order. The margin sits above the measured
/// uncached jitter of 0.06 between identical requests on this model (docs/guide/how-it-works.md),
/// so one jitter width cannot turn a tie into a decision. The count is large: on the frozen
/// validation set (benchmarks/results.md) 8 of 24 route decisions fall inside it, 4 per
/// backend, all with the runner-up 0.00 to 0.05 from the best, so the same 8 would have tied
/// at 0.05; a PATH of 1,883 commands holds several tools for most tasks.
pub const TIE_MARGIN: f64 = 0.10;

pub struct Route {
    pub tool: Option<Tool>,
    pub fit: f64,
    /// Commands above the threshold that fit within `TIE_MARGIN` of the best one. Any entry
    /// makes the answer a tie.
    pub ties: Vec<(String, f64)>,
    pub alternatives: Vec<(String, f64)>,
}

fn load_tools(cache_dir: Option<std::path::PathBuf>) -> Result<Vec<Tool>, JevifyError> {
    if let Ok(p) = std::env::var("JEVIFY_INVENTORY_FILE") {
        let b = std::fs::read(&p)
            .map_err(|e| JevifyError::Input(format!("JEVIFY_INVENTORY_FILE: {e}")))?;
        return serde_json::from_slice(&b)
            .map_err(|e| JevifyError::Input(format!("JEVIFY_INVENTORY_FILE: {e}")));
    }
    inventory::load(cache_dir.as_deref())
}

pub async fn route(
    client: &Client,
    ctx: &Config,
    request: &str,
    tools: &[Tool],
) -> Result<Route, JevifyError> {
    let items: Vec<String> = tools
        .iter()
        .map(|t| format!("{}: {}", t.name, t.summary))
        .collect();
    let prompts = Prompts {
        choose: "Which command in `items` is the right tool to accomplish `request`? Choose NONE if no listed command does it.".into(),
        none: "none of the listed commands does what the request asks".into(),
        any: "Is there a command in `items` whose purpose is to accomplish `request`?".into(),
    };
    // Round 1: windows only. The absolute fit Nouls below are round 2, so no Choice finals round.
    let short = shortlist(client, request, &items, &prompts, Finalists::Fixed(3)).await?;
    let mut pool: Vec<_> = short
        .windows
        .iter()
        .flat_map(|r| r.candidates.iter().take(3).copied())
        .collect();
    pool.sort_by(|a, b| b.p.total_cmp(&a.p));
    let finalists: Vec<usize> = pool.iter().take(12).map(|c| c.index).collect();
    // `round_one.finalists` is the twelve that round two judged, not the shortlist's pool.
    short.record(&ctx.stats, &finalists, |i| i + 1);
    if finalists.is_empty() {
        return Ok(Route {
            tool: None,
            fit: 0.0,
            ties: vec![],
            alternatives: vec![],
        });
    }
    // Round 2: absolute fit per finalist, with a richer man-page excerpt.
    // `man` costs ~90 ms per page; render the finalists' pages in parallel, not in series.
    let described: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = finalists
            .iter()
            .map(|&i| {
                let t = &tools[i];
                s.spawn(move || match manpage::description(&t.name, 500) {
                    Some(d) => format!("{}: {}. {}", t.name, t.summary, d),
                    None => format!("{}: {}", t.name, t.summary),
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let state = serde_json::json!({ "request": request, "commands": described });
    let mut qs = Questions::new();
    for (k, _) in finalists.iter().enumerate() {
        qs.insert(
            format!("fit{k:02}"),
            Question::noul_with(
                // Judge the command's purpose, not the counterfactual effect of running it.
                format!("Is the command described in `commands[{k}]` a correct, direct way to accomplish `request`?"),
                "this command is a correct, direct way to accomplish the request",
                "this command does something else, or is only tangentially related",
            ),
        );
    }
    let r = client.ask(&state, &qs).await?;
    // A missing answer is a protocol error, not "low confidence".
    let mut fits: Vec<(usize, f64)> = Vec::with_capacity(finalists.len());
    for (k, &i) in finalists.iter().enumerate() {
        fits.push((i, r.noul(&format!("fit{k:02}"))?));
    }
    fits.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, fit) = fits[0];
    let alternatives = fits
        .iter()
        .skip(1)
        .take(4)
        .map(|(i, p)| (tools[*i].name.clone(), *p))
        .collect();
    ctx.stats.gate(crate::output::Gate {
        best: Some(fit),
        next: fits.get(1).map(|(_, p)| *p),
        none: None,
        any: None,
        fails: None,
    });
    let tool = (fit >= ctx.threshold).then(|| tools[best].clone());
    let ties = if tool.is_some() {
        fits.iter()
            .skip(1)
            .take_while(|(_, p)| *p >= ctx.threshold && fit - p <= TIE_MARGIN)
            .map(|(i, p)| (tools[*i].name.clone(), *p))
            .collect()
    } else {
        Vec::new()
    };
    Ok(Route {
        tool,
        fit,
        ties,
        alternatives,
    })
}

pub async fn run(ctx: &Config, intent: &str, machine: bool) -> Result<Outcome, JevifyError> {
    if intent.trim().is_empty() {
        return Err(JevifyError::Usage("route needs a non-empty task".into()));
    }
    let client = Client::new(ctx)?;
    // Overlap connection setup with the local inventory read.
    client.prewarm();
    let cache_dir = ctx.cache_dir.clone();
    let tools = tokio::task::spawn_blocking(move || load_tools(cache_dir))
        .await
        .map_err(|e| JevifyError::Input(e.to_string()))??;
    let r = route(&client, ctx, intent, &tools).await?;
    let alts: Vec<_> = r
        .alternatives
        .iter()
        .map(|(n, p)| serde_json::json!({ "tool": n, "fit": p }))
        .collect();
    let Some(tool) = r.tool else {
        if !machine {
            eprintln!(
                "jevify: nothing installed does this (best fit {:.2})",
                r.fit
            );
            for (n, p) in r.alternatives.iter().take(3) {
                eprintln!("  nearest (not chosen): {n} ({p:.2})");
            }
        }
        return Ok(Outcome {
            exit: Exit::Abstain,
            data: serde_json::json!({ "tool": null, "summary": null, "synopsis": null, "fit": r.fit, "ties": [], "alternatives": alts }),
            human: Vec::new(),
            exec: None,
        });
    };
    if !r.ties.is_empty() {
        // Two commands too close to tell apart: say so and name nothing, as VISION promises.
        let tied: Vec<(String, f64)> = std::iter::once((tool.name.clone(), r.fit))
            .chain(r.ties.iter().cloned())
            .collect();
        if !machine {
            let named: Vec<String> = tied.iter().map(|(n, p)| format!("{n} ({p:.2})")).collect();
            eprintln!("jevify: too close to tell apart: {}", named.join(", "));
        }
        let ties: Vec<_> = tied
            .iter()
            .map(|(n, p)| serde_json::json!({ "tool": n, "fit": p }))
            .collect();
        return Ok(Outcome {
            exit: Exit::Abstain,
            data: serde_json::json!({ "tool": null, "summary": null, "synopsis": null, "fit": r.fit, "ties": ties, "alternatives": alts }),
            human: Vec::new(),
            exec: None,
        });
    }
    let name = tool.name.clone();
    let synopsis = tokio::task::spawn_blocking(move || manpage::synopsis(&name))
        .await
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    if !machine {
        eprintln!("jevify: {} ({:.2}) — {}", tool.name, r.fit, tool.summary);
        if let Some(text) = &synopsis {
            eprintln!("  {text}");
        }
    }
    Ok(Outcome {
        exit: Exit::Ok,
        data: serde_json::json!({ "tool": tool.name, "summary": tool.summary, "synopsis": synopsis, "fit": r.fit, "ties": [], "alternatives": alts }),
        human: format!("{}\n", tool.name).into_bytes(),
        exec: None,
    })
}
