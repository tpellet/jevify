use crate::cmd::Outcome;
use crate::config::Config;
use crate::exit::{Exit, JevifyError};
use crate::jev::client::Client;
use crate::tournament::{Finalists, Prompts, Ranking, shortlist, window};
use regex::Regex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

pub const SMALL: usize = 1500;
pub const TAIL: usize = 1000;
pub const NEIGHBOURS: usize = 5;
pub const MAX_KEEP: usize = 4000;

static SIGNAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(error|err!|fail(ed|ure|s)?|fatal|panic(ked)?|exception|traceback|denied|not found|no such|cannot|can't|couldn't|undefined|unresolved|refused|timed? ?out|segmentation|abort(ed)?|killed|exit (code|status) [1-9]|assert)").unwrap()
});

/// Lines that state a test or step outcome: the finals show each candidate next to the nearest
/// one, so a loud line printed by a passing step reads as such.
static FAILURE: LazyLock<Regex> = LazyLock::new(|| {
    // Case matters: `0 failed` in a passing summary is a count, `FAILED` is a verdict.
    Regex::new(r"(--- FAIL\b|\bFAILED\b|\bFAILURES?\b|^FAIL\b|npm ERR!|Traceback|panicked at|race detected|error: (test failed|could not compile|process didn't exit|failed to)|exit (code|status) [1-9]|Process completed with exit code [1-9])").unwrap()
});

/// A Rust panic header: it names the thread and the source position, and the message follows on
/// the next lines. The header states that a test failed; the message says why.
static PANIC_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"panicked at \S+:\d+:\d+:\s*$").unwrap());
/// Where a panic's message ends: the backtrace, the `RUST_BACKTRACE` note, or the next block.
static PANIC_END: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(stack backtrace:|note: run with `RUST_BACKTRACE|^\s*(----|failures:)|\bpanicked at\b|test result:)")
        .unwrap()
});
/// The most lines a panic message contributes to the finals.
pub const PANIC_MESSAGE: usize = 6;

/// The text of a log line without a `gh run view --log` prefix (`job\tstep\ttimestamp `): a
/// prefix alone is a blank line, not a message.
fn payload(line: &str) -> &str {
    let tail = line.rsplit('\t').next().unwrap_or(line);
    match tail.split_once(' ') {
        Some((stamp, rest))
            if stamp.ends_with('Z') && stamp.starts_with(|c: char| c.is_ascii_digit()) =>
        {
            rest
        }
        _ => tail,
    }
}

/// The lines that carry the message of the panic whose header is line `header`: the non-blank
/// lines that follow, up to `PANIC_MESSAGE` of them, until the backtrace or the next block.
/// Empty when the line is not a panic header.
pub fn panic_message(lines: &[String], header: usize) -> Vec<usize> {
    if !PANIC_HEADER.is_match(&lines[header]) {
        return vec![];
    }
    lines
        .iter()
        .enumerate()
        .skip(header + 1)
        .take_while(|(_, l)| !PANIC_END.is_match(l))
        .filter(|(_, l)| !payload(l).trim().is_empty())
        .map(|(i, _)| i)
        .take(PANIC_MESSAGE)
        .collect()
}

/// The panic block that line `i` belongs to, header first, without `i` itself: empty when the
/// line is neither a panic header nor one of the lines of a panic message.
pub fn panic_block(lines: &[String], i: usize) -> Vec<usize> {
    let message = panic_message(lines, i);
    if !message.is_empty() {
        return message;
    }
    // A message line: its header is a few lines up, and its own message names this line.
    (i.saturating_sub(2 * PANIC_MESSAGE)..i)
        .rev()
        .find(|&h| panic_message(lines, h).contains(&i))
        .map(|h| {
            std::iter::once(h)
                .chain(panic_message(lines, h).into_iter().filter(|&m| m != i))
                .collect()
        })
        .unwrap_or_default()
}

pub fn failure_lines(lines: &[String]) -> Vec<usize> {
    (0..lines.len())
        .filter(|&i| FAILURE.is_match(&lines[i]))
        .collect()
}

/// The nearest failure statement to line `i`, as context for the finals; none when the output
/// has no other, so the model never reads the lack of a marker as "no failure".
pub fn nearest_failure(lines: &[String], failures: &[usize], i: usize) -> Option<String> {
    let clip = |s: &str| s.chars().take(200).collect::<String>();
    match failures
        .iter()
        .filter(|&&f| f != i)
        .min_by_key(|&&f| f.abs_diff(i))
    {
        Some(&f) if f < i => Some(format!(
            "\n    (context only, not a candidate; nearest failure statement, {} lines before: line {}: {})",
            i - f,
            f + 1,
            clip(&lines[f])
        )),
        Some(&f) => Some(format!(
            "\n    (context only, not a candidate; nearest failure statement, {} lines after: line {}: {})",
            f - i,
            f + 1,
            clip(&lines[f])
        )),
        None => None,
    }
}

pub fn prefilter(lines: &[String]) -> Vec<usize> {
    // Blank lines and repeats never leave the machine: a line already seen (a repeated warning,
    // a separator) would be a second identical item splitting the Choice mass. The first
    // occurrence keeps its line number; context is read from `lines`, so nothing is lost.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut base: Vec<usize> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if l.trim().is_empty() || !seen.insert(l.as_str()) {
            continue;
        }
        base.push(i);
    }
    if base.len() <= SMALL {
        return base;
    }
    // The root cause is usually the first error: keep signal neighbourhoods from the top
    // first, then fill the remaining budget with the tail.
    let mut keep: BTreeSet<usize> = BTreeSet::new();
    for p in (0..base.len()).filter(|&p| SIGNAL.is_match(&lines[base[p]])) {
        if keep.len() >= MAX_KEEP - TAIL {
            break;
        }
        let hi = (p + NEIGHBOURS).min(base.len() - 1);
        keep.extend(&base[p.saturating_sub(NEIGHBOURS)..=hi]);
    }
    for &i in base.iter().rev() {
        if keep.len() >= MAX_KEEP {
            break;
        }
        keep.insert(i);
    }
    keep.into_iter().collect()
}

pub async fn run(
    ctx: &Config,
    context: usize,
    top: usize,
    no_save: bool,
) -> Result<Outcome, JevifyError> {
    if top == 0 {
        return Err(JevifyError::Usage("-n must be at least 1".into()));
    }
    let client = Client::new(ctx)?;
    let directory = crate::config::saved_input_dir(no_save);
    let (lines, saved) = tokio::task::spawn_blocking(move || {
        let bytes = crate::input::read_stdin_bytes()?;
        let saved = crate::save::save(&bytes, directory.as_deref());
        let lines = crate::input::split_lines(&String::from_utf8_lossy(&bytes));
        Ok::<_, JevifyError>((lines, saved))
    })
    .await
    .map_err(|e| JevifyError::Input(e.to_string()))??;
    let complete = saved.is_ok();
    let saved_input = match saved {
        Ok(path) => {
            eprintln!(
                "jevify why: full output: {}",
                path.to_string_lossy().replace(['\r', '\n'], " ")
            );
            Some(path)
        }
        Err(reason) => {
            eprintln!(
                "jevify why: full output: not saved ({})",
                reason.replace(['\r', '\n'], " ")
            );
            None
        }
    };
    if lines.iter().all(|line| line.trim().is_empty()) {
        return Err(JevifyError::EmptyInput("stdin was empty"));
    }
    let kept = prefilter(&lines);
    eprintln!(
        "jevify why: {} lines, candidates {}, windows {}",
        lines.len(),
        kept.len(),
        kept.len().div_ceil(ctx.backend.window())
    );
    let no_signal = !kept.iter().any(|&i| SIGNAL.is_match(&lines[i]));
    let items: Vec<String> = kept
        .iter()
        .map(|&i| format!("line {}: {}", i + 1, lines[i]))
        .collect();
    let prompts = Prompts {
        choose: "`items` are lines of output from a failed command, build or test run. Which line states the root cause of the failure: the first line from the step or test that failed which explains why it failed? Not a later consequence, a generic summary such as `build failed`, or a stack frame; and not a line that a step or test which passed printed, however loud it looks (a warning, a logged error, an expected stderr message): the failing step's own finding can be quiet, such as a lint or a diff. Choose NONE if the output shows no failure.".into(),
        none: "the output shows no failure".into(),
        any: "Does `items` contain a line that states why the command, build or test failed?".into(),
    };
    // The finals see a panic header next to its message: the header names the test and the
    // source position, the message says why, and the answer is the message.
    let finals_prompts = Prompts {
        choose: prompts.choose.replace(
            " Choose NONE if the output shows no failure.",
            " When a test panicked or threw, the line that carries the message, not the header that names the test and the source position. Choose NONE if the output shows no failure.",
        ),
        none: prompts.none.clone(),
        any: prompts.any.clone(),
    };
    // Neighbours are labelled as context so a literal reader does not pick them as the answer;
    // the nearest failure statement tells a loud line of a passing step from the failure itself.
    let failures = failure_lines(&lines);
    let with_context = |k: usize| {
        let i = kept[k];
        let prev = if i > 0 { lines[i - 1].as_str() } else { "" };
        let next = lines.get(i + 1).map(String::as_str).unwrap_or("");
        let outcome = nearest_failure(&lines, &failures, i).unwrap_or_default();
        format!(
            "line {}: {}\n    (context only, not a candidate; before: {prev})\n    (context only, not a candidate; after: {next}){outcome}",
            i + 1,
            lines[i]
        )
    };
    // Round one reads the whole output; the finals re-rank a handful of lines. Whether the
    // output shows a failure is best answered where the failure is in view, so `any` is the
    // largest answer over both rounds, never only the finals' reading of three lines.
    let request = "find the root cause of the failure";
    let first = shortlist(&client, request, &items, &prompts, Finalists::Auto).await?;
    let windows = first.windows.len();
    let round_one_any = first.windows.iter().map(|w| w.any).fold(0.0, f64::max);
    let mut ranking = if first.finalists.is_empty() {
        first.record(&ctx.stats, &[], |i| kept[i] + 1);
        Ranking {
            candidates: vec![],
            any: 0.0,
            none: 1.0,
            windows,
            n: first.n,
        }
    } else {
        // A panic header outranks its own message in round one, where the header names the test
        // and the message stands alone. The finals judge them side by side: a finalist's whole
        // panic block, header and message lines, joins the finals right after it.
        //
        // The finals are one request, so they hold at most one window. Round one already keeps
        // its finalists within it (`Finalists::Auto`: three per window up to a third of the
        // window, then two, then one; 33 × 3 = 99 keyless, 66 × 3 = 198 with a key; a
        // 1,000-line log is 11 keyless windows and 33 finalists, or 5 windows and 15). Every
        // finalist keeps its slot, by rank; the panic lines share only the room left, so a
        // late finalist is never pushed past the window by an early finalist's message.
        let size = ctx.backend.window();
        let mut chosen: Vec<usize> = Vec::new();
        for c in &first.finalists {
            if chosen.len() < size && !chosen.contains(&c.index) {
                chosen.push(c.index);
            }
        }
        if chosen.len() < first.finalists.len() {
            eprintln!(
                "jevify why: finals hold {} of {} finalists",
                chosen.len(),
                first.finalists.len()
            );
        }
        let mut room = size - chosen.len();
        let finalists = chosen.clone();
        for &f in &finalists {
            for i in panic_block(&lines, kept[f]) {
                if room == 0 {
                    break;
                }
                if let Ok(k) = kept.binary_search(&i) {
                    if !chosen.contains(&k) {
                        chosen.push(k);
                        room -= 1;
                    }
                }
            }
        }
        // `round_one.finalists` is the finals as sent, panic lines included.
        first.record(&ctx.stats, &chosen, |i| kept[i] + 1);
        let finals: Vec<(usize, String)> = chosen.iter().map(|&k| (k, with_context(k))).collect();
        window(&client, request, &finals, &finals_prompts).await?
    };
    ranking.windows = windows;
    ranking.n = first.n;
    ranking.any = ranking.any.max(round_one_any);
    if ranking.n != 3 {
        eprintln!("jevify why: finalists per window: {}", ranking.n);
    }
    ctx.stats.gate(super::gate_of(&ranking));
    let found = ranking.any >= ctx.threshold
        && ranking
            .candidates
            .first()
            .is_some_and(|c| c.p > ranking.none);
    let causes: Vec<serde_json::Value> = if found {
        ranking
            .candidates
            .iter()
            .filter(|c| c.p > ranking.none)
            .take(top)
            .map(|c| {
                let i = kept[c.index];
                let lo = i.saturating_sub(context);
                let hi = (i + context).min(lines.len() - 1);
                let ctx_lines: Vec<_> = (lo..=hi).map(|j| serde_json::json!({ "line": j + 1, "text": lines[j] })).collect();
                serde_json::json!({ "line": i + 1, "text": lines[i], "p": c.p, "context": ctx_lines })
            })
            .collect()
    } else {
        vec![]
    };
    let mut human = String::new();
    for c in &causes {
        let line = c["line"].as_u64().unwrap_or(0);
        for l in c["context"].as_array().into_iter().flatten() {
            let n = l["line"].as_u64().unwrap_or(0);
            let mark = if n == line { ">" } else { " " };
            human.push_str(&format!(
                "{mark}{n:>6} │ {}\n",
                l["text"].as_str().unwrap_or_default()
            ));
        }
        human.push('\n');
    }
    // The first thing most people get wrong: compilers write errors to stderr.
    let hint = (causes.is_empty() && no_signal).then(|| {
        "no error-like lines on stdin; most tools write errors to stderr: `cmd 2>&1 | jevify why`".to_string()
    });
    if let Some(h) = &hint {
        eprintln!("jevify why: {h}");
    } else if causes.is_empty() {
        let nearest = super::closest(&ranking, |i| {
            format!("line {}: {}", kept[i] + 1, lines[kept[i]])
        });
        eprintln!(
            "{}",
            super::abstain_line(
                "why",
                "no line looks like the cause",
                &nearest,
                &super::abstain_hint(
                    &ranking,
                    ctx.threshold,
                    "pipe the failing step's own output, with 2>&1, not a summary of it"
                )
            )
        );
    }
    Ok(Outcome {
        exit: if causes.is_empty() {
            Exit::Abstain
        } else {
            Exit::Ok
        },
        data: serde_json::json!({ "causes": causes, "any": ranking.any, "considered": kept.len(), "total": lines.len(), "hint": hint, "saved_input": saved_input, "complete": complete }),
        human: human.into_bytes(),
        exec: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefilter_dedupes_small_logs_and_keeps_the_first_error_and_the_tail_of_large_ones() {
        // Blank lines, a run of repeats and a later repeat of `a` are all dropped; the first
        // occurrence keeps its index.
        let lines: Vec<String> = ["a", "", "b", "b", "b", "c", "a"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(prefilter(&lines), vec![0, 2, 5]);
        // The earliest error and the tail survive thousands of later signals.
        let mut lines: Vec<String> = (0..50_000)
            .map(|i| {
                if i > 1_000 && i % 7 == 0 {
                    format!("npm ERR! error {i}")
                } else {
                    format!("step {i}")
                }
            })
            .collect();
        lines[10] = "error: linker `cc` not found".into();
        let kept = prefilter(&lines);
        assert!(kept.contains(&10) && kept.contains(&49_999));
    }

    #[test]
    fn failure_statements_and_the_nearest_one_with_its_distance_and_direction() {
        let lines: Vec<String> = [
            "  CACHE_ON_FAILURE: false",
            "jevify: output error: permission denied",
            "test tests::output_errors ... ok",
            "test result: ok. 132 passed; 0 failed",
            "test documented_examples ... FAILED",
            "thread 'documented_examples' panicked at tests/agent.rs:64:17:",
            "jevify fill: not run: arg 3 branch: no_match",
            "--- FAIL: TestPull (0.01s)",
            "    testing.go:1712: race detected during execution of test",
            "npm ERR! code ELIFECYCLE",
            "Traceback (most recent call last):",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // `CACHE_ON_FAILURE` is one word; `0 failed` is a count, not a statement.
        let f = failure_lines(&lines);
        assert_eq!(f, vec![4, 5, 7, 8, 9, 10]);
        let nearest = |i| nearest_failure(&lines, &f, i).unwrap();
        assert!(nearest(1).contains("3 lines after: line 5: test documented"));
        assert!(nearest(6).contains("1 lines before: line 6: thread"));
        // A failure statement itself is shown next to the nearest other one.
        assert!(nearest(5).contains("1 lines before: line 5:"));
        // No marker at all: nothing is said, so the lack of one never reads as "no failure".
        assert_eq!(nearest_failure(&lines, &[], 1), None);
    }

    #[test]
    fn panic_message_follows_its_header_until_the_backtrace_or_the_next_block() {
        let lines: Vec<String> = [
            "test documented_examples ... FAILED",
            "thread 'documented_examples' (3856) panicked at tests/agent.rs:64:17:",
            "jevify fill --dry-run -- git switch '@{branch:the auth refactor}'",
            "",
            "jevify fill: not run: arg 3 branch: no_match; ; candidates 0 of 0, omitted 0",
            "",
            "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
            "thread 'other' panicked at src/lib.rs:1:1:",
            "Some files were not up-to-date",
            "stack backtrace:",
            "   0: __rustc::rust_begin_unwind",
            "thread 'last' panicked at src/a.rs:2:2:",
            "one",
            "not a header: panicked at src/a.rs:2:2: with the message on the same line",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // Blank lines are skipped; the message stops at the `RUST_BACKTRACE` note.
        assert_eq!(panic_message(&lines, 1), vec![2, 4]);
        // ...and at the backtrace.
        assert_eq!(panic_message(&lines, 7), vec![8]);
        // A following panic statement ends the preceding message.
        assert_eq!(panic_message(&lines, 11), vec![12]);
        // Not a header: a FAILED verdict, a message line, a panic with its message on one line.
        for i in [0, 2, 4, 8, 13] {
            assert_eq!(panic_message(&lines, i), Vec::<usize>::new(), "line {i}");
        }
        // A `gh run view --log` prefix alone is a blank line.
        let prefixed: Vec<String> = [
            "job\tstep\t2026-09-22T13:51:47.2052615Z thread 'x' (1) panicked at tests/agent.rs:64:17:",
            "job\tstep\t2026-09-22T13:51:47.2053262Z jevify fill --dry-run -- git switch",
            "job\tstep\t2026-09-22T13:51:47.2053542Z ",
            "job\tstep\t2026-09-22T13:51:47.2053813Z jevify fill: not run: arg 3 branch: no_match",
            "job\tstep\t2026-09-22T13:51:47.2054196Z ",
            "job\tstep\t2026-09-22T13:51:47.2054409Z note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(panic_message(&prefixed, 0), vec![1, 3]);
        // A message line brings its header and the message's other lines; a header its message.
        assert_eq!(panic_block(&lines, 4), vec![1, 2]);
        assert_eq!(panic_block(&lines, 2), vec![1, 4]);
        assert_eq!(panic_block(&lines, 1), vec![2, 4]);
        assert_eq!(panic_block(&prefixed, 3), vec![0, 1]);
        for i in [0, 6, 10, 13] {
            assert_eq!(panic_block(&lines, i), Vec::<usize>::new(), "line {i}");
        }
        assert_eq!(payload("2026-09-22T13:51:47Z "), "");
        assert_eq!(payload("Z is not a stamp"), "Z is not a stamp");
        assert_eq!(payload("plain line"), "plain line");
    }
}
