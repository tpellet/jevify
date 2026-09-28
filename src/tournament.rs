use crate::exit::JevifyError;
use crate::jev::client::Client;
use crate::jev::{Question, Questions};
use std::collections::BTreeMap;

/// The TypeSafe window, and the largest one jevify ever sends. classifier.dev takes 99 options
/// plus NONE; the size in force comes from the backend, not from here.
pub const WINDOW: usize = 200;
const WINNER_RATIO: f64 = 2.0;

#[derive(Debug, Clone, Copy)]
pub enum Finalists {
    ThreeOnly,
    Auto,
    /// Route asks its own second question, without a Choice finals capacity.
    Fixed(usize),
}

impl Finalists {
    pub fn per_window(self, count: usize, size: usize) -> Result<usize, JevifyError> {
        let windows = count.div_ceil(size);
        let limit = match self {
            Self::ThreeOnly => size * (size / 3),
            Self::Auto => size * size,
            Self::Fixed(n) => return Ok(n),
        };
        if count > limit {
            return Err(JevifyError::Kinded {
                kind: "too_many",
                exit: crate::exit::Exit::Input,
                message: format!("{count} candidates exceed the two-round capacity of {limit}"),
                hint: "narrow with grep, head or a path prefix",
                example: "head -n 1000 candidates | jevify pick 'description'",
            });
        }
        Ok(if windows * 3 <= size {
            3
        } else if windows * 2 <= size {
            2
        } else {
            1
        })
    }
}
/// Character budget for all items of one window: ~15k tokens at ~4 chars/token (typical text),
/// ~24k for dense logs (hashes, paths, JSON) at ~2.5 chars/token, still under the 32k-token
/// state + question limit. Non-Latin scripts tokenize denser still; the API then answers
/// 413/422, which surfaces as `api_rejected_request` (exit 6), never as corruption.
const WINDOW_CHARS: usize = 60_000;

/// Truncates on a char boundary; long log/JSON lines would otherwise overflow the token budget.
pub fn clip(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    match s.char_indices().nth(max) {
        Some(_) => format!("{}…", s.chars().take(max - 1).collect::<String>()),
        None => s.to_string(),
    }
}

pub struct Prompts {
    /// Choice instruction; must refer to `request` and `items`.
    pub choose: String,
    /// Description of the NONE option.
    pub none: String,
    /// Absolute yes/no instruction: does any item satisfy the request?
    pub any: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub index: usize,
    pub p: f64,
}

#[derive(Debug, Clone)]
pub struct Ranking {
    pub candidates: Vec<Candidate>,
    pub any: f64,
    /// P(NONE) in the deciding Choice; a candidate that does not beat it is not a match.
    pub none: f64,
    pub windows: usize,
    pub n: usize,
}

#[derive(Debug)]
pub struct Shortlist {
    pub windows: Vec<Ranking>,
    pub finalists: Vec<Candidate>,
    pub n: usize,
}

/// `JEVIFY_DECISION=round_one` asks for `meta.decision.round_one`; without it no tournament
/// is recorded and the envelope carries no such field.
pub fn round_one_wanted() -> bool {
    std::env::var("JEVIFY_DECISION")
        .is_ok_and(|v| v.split(',').any(|part| part.trim() == "round_one"))
}

impl Shortlist {
    /// Surfaces round one in `meta.decision.round_one` when it was asked for. `judged` is the
    /// finals as they were sent, as item positions: the shortlist's own picks, widened or joined
    /// by a verb, or empty when round one alone decided. `index` turns an item position into the
    /// verb's own number for it (a line, a record, a listing position).
    pub fn record(
        &self,
        stats: &crate::jev::client::Stats,
        judged: &[usize],
        index: impl Fn(usize) -> usize,
    ) {
        if !round_one_wanted() {
            return;
        }
        let candidate = |c: &Candidate| crate::output::RoundOneCandidate {
            index: index(c.index),
            p: c.p,
        };
        stats.round_one(crate::output::RoundOne {
            windows: self
                .windows
                .iter()
                .map(|w| crate::output::RoundOneWindow {
                    ranks: w.candidates.iter().map(candidate).collect(),
                    none: w.none,
                    any: w.any,
                })
                .collect(),
            finalists: judged.iter().map(|&i| index(i)).collect(),
            n: self.n,
        });
    }
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Found(Candidate),
    NoMatch,
    Ambiguous(Vec<Candidate>),
}

pub fn decide(ranking: &Ranking, threshold: f64) -> Decision {
    let Some(best) = ranking.candidates.first() else {
        return Decision::NoMatch;
    };
    if ranking.any < threshold || ranking.none >= best.p {
        return Decision::NoMatch;
    }
    let second = ranking.candidates.get(1).map_or(0.0, |c| c.p);
    if best.p > second && best.p >= WINNER_RATIO * second.max(ranking.none) {
        Decision::Found(*best)
    } else {
        Decision::Ambiguous(ranking.candidates.iter().take(2).copied().collect())
    }
}

fn id(i: usize) -> String {
    format!("L{i:03}")
}

pub(crate) async fn window(
    client: &Client,
    request: &str,
    items: &[(usize, String)],
    prompts: &Prompts,
) -> Result<Ranking, JevifyError> {
    // The backend's own input limit caps the window budget: classifier.dev rejects an input
    // over 32,000 characters, so its windows carry shorter excerpts, never a rejected request.
    let budget = WINDOW_CHARS.min(client.backend().max_state_chars());
    let per_item = (budget / items.len().max(1)).clamp(200, 2_000);
    let state = serde_json::json!({
        "request": request,
        "items": items.iter().enumerate().map(|(i, (_, t))| format!("[{}] {}", id(i), clip(&crate::input::redact(t), per_item))).collect::<Vec<_>>(),
    });
    let mut crit: BTreeMap<String, Option<String>> =
        (0..items.len()).map(|i| (id(i), None)).collect();
    crit.insert("NONE".into(), Some(prompts.none.clone()));
    let mut qs = Questions::new();
    qs.insert(
        "pick".into(),
        Question::choice(prompts.choose.clone(), crit),
    );
    qs.insert("any".into(), Question::noul(prompts.any.clone()));
    let r = client.ask(&state, &qs).await?;
    let probs = r.probs("pick")?;
    let none = probs.get("NONE").copied().unwrap_or(0.0);
    let mut candidates: Vec<Candidate> = probs
        .iter()
        .filter(|(k, _)| k.as_str() != "NONE")
        .filter_map(|(k, p)| {
            k.strip_prefix('L')?
                .parse::<usize>()
                .ok()
                .filter(|i| id(*i) == *k)
                .and_then(|i| items.get(i))
                .map(|(g, _)| Candidate { index: *g, p: *p })
        })
        .collect();
    candidates.sort_by(|a, b| b.p.total_cmp(&a.p));
    Ok(Ranking {
        candidates,
        any: r.noul("any")?,
        none,
        windows: 1,
        n: 3,
    })
}

pub async fn rank(
    client: &Client,
    request: &str,
    items: &[String],
    prompts: &Prompts,
    finalist_text: Option<&(dyn Fn(usize) -> String + Sync)>,
    mode: Finalists,
) -> Result<Ranking, JevifyError> {
    let first = shortlist(client, request, items, prompts, mode).await?;
    let windows = first.windows.len();
    if windows == 1 && finalist_text.is_none() {
        first.record(client.stats(), &[], |i| i + 1);
        return Ok(first.windows.into_iter().next().unwrap());
    }
    let judged: Vec<usize> = first.finalists.iter().map(|c| c.index).collect();
    first.record(client.stats(), &judged, |i| i + 1);
    if first.finalists.is_empty() {
        return Ok(Ranking {
            candidates: vec![],
            any: 0.0,
            none: 1.0,
            windows,
            n: first.n,
        });
    }
    let finals: Vec<(usize, String)> = first
        .finalists
        .iter()
        .map(|c| {
            (
                c.index,
                finalist_text.map_or_else(|| items[c.index].clone(), |f| f(c.index)),
            )
        })
        .collect();
    let mut ranking = window(client, request, &finals, prompts).await?;
    ranking.windows = windows;
    ranking.n = first.n;
    Ok(ranking)
}

/// Round one, with finalists ordered by rank, then by window index.
pub async fn shortlist(
    client: &Client,
    request: &str,
    items: &[String],
    prompts: &Prompts,
    mode: Finalists,
) -> Result<Shortlist, JevifyError> {
    let n = mode.per_window(items.len(), client.backend().window())?;
    let all: Vec<(usize, String)> = items.iter().cloned().enumerate().collect();
    let rounds = futures::future::try_join_all(
        all.chunks(client.backend().window())
            .map(|w| window(client, request, w, prompts)),
    )
    .await?;
    let finalists = (0..n)
        .flat_map(|rank| {
            rounds
                .iter()
                .filter_map(move |r| r.candidates.get(rank).copied())
        })
        .collect();
    Ok(Shortlist {
        windows: rounds,
        finalists,
        n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decisions_use_absolute_fit_and_ratio() {
        for (best, second, none, any, expected) in [
            (0.6, 0.3, 0.1, 0.9, "found"),
            (0.5, 0.4, 0.1, 0.9, "ambiguous"),
            (0.45, 0.45, 0.1, 0.9, "ambiguous"),
            (0.5, 0.1, 0.4, 0.9, "ambiguous"),
            (0.8, 0.1, 0.1, 0.4, "no_match"),
            (0.2, 0.1, 0.7, 0.9, "no_match"),
        ] {
            let ranking = Ranking {
                candidates: vec![
                    Candidate { index: 0, p: best },
                    Candidate {
                        index: 1,
                        p: second,
                    },
                ],
                any,
                none,
                windows: 1,
                n: 3,
            };
            let actual = match decide(&ranking, 0.5) {
                Decision::Found(_) => "found",
                Decision::NoMatch => "no_match",
                Decision::Ambiguous(closest) => {
                    assert_eq!(closest.len(), 2);
                    "ambiguous"
                }
            };
            assert_eq!(actual, expected);
        }
        assert_eq!(
            decide(
                &Ranking {
                    candidates: vec![],
                    any: 1.0,
                    none: 0.0,
                    windows: 0,
                    n: 3
                },
                0.5
            ),
            Decision::NoMatch
        );
    }
}
