use crate::cmd::Outcome;
use crate::config::Config;
use crate::exit::{Exit, JevifyError};
use crate::gitdiff;
use crate::jev::client::Client;
use crate::jev::{Question, Questions};
use std::io::Write;
use std::process::{Command, Stdio};

const BATCH: usize = 20;
/// Reject oversized hunks rather than staging evidence the model did not see.
const HUNK_CHARS: usize = 3_000;

pub async fn run(
    ctx: &Config,
    topic: &str,
    yes: bool,
    dry_run: bool,
    machine: bool,
) -> Result<Outcome, JevifyError> {
    // From a subdirectory `git diff` lists the whole repo, but `git apply` silently skips paths
    // outside the cwd (exit 0): run both at the top level, or hunks are reported staged but are not.
    let top = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| JevifyError::Input(format!("git: {e}")))?;
    if !top.status.success() {
        return Err(JevifyError::Input(
            "not a git repository (or git failed)".into(),
        ));
    }
    let top = std::path::PathBuf::from(String::from_utf8_lossy(&top.stdout).trim());
    // Explicit prefixes: a user's `diff.noprefix=true` would otherwise break file_name and git apply.
    let out = Command::new("git")
        .current_dir(&top)
        .args([
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "-U3",
        ])
        .output()
        .map_err(|e| JevifyError::Input(format!("git: {e}")))?;
    if !out.status.success() {
        return Err(JevifyError::Input(
            "not a git repository (or git failed)".into(),
        ));
    }
    let files = gitdiff::parse(&String::from_utf8_lossy(&out.stdout));
    let flat: Vec<(usize, usize, String)> = files
        .iter()
        .enumerate()
        .flat_map(|(fi, f)| {
            f.hunks.iter().enumerate().map(move |(hi, h)| {
                (
                    fi,
                    hi,
                    crate::input::redact(&format!("{}{}", h.header, h.body)),
                )
            })
        })
        .collect();
    if flat.is_empty() {
        return Err(JevifyError::EmptyInput(
            "no unstaged changes to tracked files (untracked files are never staged by add)",
        ));
    }
    if flat.iter().any(|(_, _, h)| h.chars().count() > HUNK_CHARS) {
        return Err(JevifyError::InputTooLarge(format!(
            "hunk exceeds the {HUNK_CHARS}-character evidence budget; no hunks were classified or staged"
        )));
    }
    // Staging needs a yes: --yes, or a terminal to ask on. Without either, stop before any
    // request, so an unattended caller learns the fix without paying for a classification.
    let terminal = || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .is_ok()
    };
    if !yes && !dry_run && (machine || !terminal()) {
        return Err(JevifyError::Kinded {
            kind: "declined",
            exit: Exit::Interrupted,
            message: "add stages only with --yes, or after a yes on a terminal, and there is none; nothing was classified or staged".into(),
            hint: "pass --yes to stage, or --dry-run to score the hunks",
            example: "jevify add --dry-run \"the token expiry fix\"",
        });
    }
    let client = Client::new(ctx)?;
    let file_name = |fi: usize| {
        files[fi]
            .header
            .lines()
            .next()
            .unwrap_or_default()
            .rsplit(" b/")
            .next()
            .unwrap_or_default()
            .to_string()
    };
    let states: Vec<_> = flat.chunks(BATCH).map(|chunk| {
        serde_json::json!({ "topic": topic, "hunks": chunk.iter().map(|(fi, _, h)| format!("file {}\n{}", file_name(*fi), h)).collect::<Vec<_>>() })
    }).collect();
    if states
        .iter()
        .any(|state| state.to_string().chars().count() > client.backend().max_state_chars())
    {
        return Err(JevifyError::InputTooLarge("complete hunk batch exceeds the backend evidence budget; no hunks were classified or staged".into()));
    }
    let batches = flat.chunks(BATCH).zip(states).map(|(chunk, state)| {
        let mut qs = Questions::new();
        for i in 0..chunk.len() {
            qs.insert(
                format!("h{i:02}"),
                Question::noul_with(
                    format!("Is the change in `hunks[{i}]` part of the work described by `topic`?"),
                    "this hunk implements or directly supports the described work",
                    "this hunk is about something else",
                ),
            );
        }
        let client = &client;
        async move {
            let r = client.ask(&state, &qs).await?;
            (0..chunk.len())
                .map(|i| r.noul(&format!("h{i:02}")))
                .collect::<Result<Vec<f64>, JevifyError>>()
        }
    });
    let ps: Vec<f64> = futures::future::try_join_all(batches)
        .await?
        .into_iter()
        .flatten()
        .collect();
    for p in &ps {
        ctx.stats.gate(crate::output::Gate::noul(*p));
    }
    let chosen: Vec<bool> = ps.iter().map(|p| *p >= ctx.threshold).collect();
    let rows: Vec<_> = flat.iter().zip(&ps).zip(&chosen)
        .map(|(((fi, hi, _), p), c)| serde_json::json!({ "file": file_name(*fi), "header": files[*fi].hunks[*hi].header.trim(), "p": p, "staged": *c && !dry_run }))
        .collect();
    let n = chosen.iter().filter(|c| **c).count();
    if n == 0 {
        let mut ranked: Vec<(String, f64)> = flat
            .iter()
            .zip(&ps)
            .map(|((fi, hi, _), p)| {
                (
                    format!("{} {}", file_name(*fi), files[*fi].hunks[*hi].header.trim()),
                    *p,
                )
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.truncate(3);
        eprintln!(
            "{}",
            super::abstain_line(
                "add",
                "no hunk is about the topic",
                &ranked,
                "name what the change does in the code (a function, a behaviour), and check the hunks with --dry-run"
            )
        );
        return Ok(Outcome {
            exit: Exit::Abstain,
            data: serde_json::json!({ "hunks": rows }),
            human: Vec::new(),
            exec: None,
        });
    }
    let mut summary = String::new();
    for (((fi, hi, _), p), c) in flat.iter().zip(&ps).zip(&chosen) {
        summary.push_str(&format!(
            "{} {:.2} {} {}\n",
            if *c { "+" } else { " " },
            p,
            file_name(*fi),
            files[*fi].hunks[*hi].header.trim()
        ));
    }
    if dry_run {
        return Ok(Outcome {
            exit: Exit::Ok,
            data: serde_json::json!({ "hunks": rows }),
            human: summary.into_bytes(),
            exec: None,
        });
    }
    // Machine mode requires --yes; humans confirm on /dev/tty (no TTY → declined, nothing staged).
    if !yes
        && (machine
            || !matches!(
                crate::cmd::confirm_tty(&format!("{summary}Stage {n} hunk(s)? [y/N] "))?,
                Some(true)
            ))
    {
        return Err(JevifyError::Declined);
    }
    let keep = |fi: usize, hi: usize| {
        flat.iter()
            .zip(&chosen)
            .any(|((f, h, _), c)| *c && *f == fi && *h == hi)
    };
    let p = gitdiff::patch(&files, &keep);
    let mut child = Command::new("git")
        .current_dir(&top)
        .args(["apply", "--cached", "--recount", "-"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(p.as_bytes())
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    if !child
        .wait()
        .map_err(|e| JevifyError::Input(e.to_string()))?
        .success()
    {
        return Err(JevifyError::Input(
            "git apply --cached rejected the patch; nothing was staged".into(),
        ));
    }
    Ok(Outcome {
        exit: Exit::Ok,
        data: serde_json::json!({ "hunks": rows }),
        human: summary.into_bytes(),
        exec: None,
    })
}
