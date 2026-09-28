//! The tournament: a planted answer is found across windows on both backends, a list past the
//! capacity is refused before any request, and long items are clipped to the window budget.
mod common;
use common::{FakeJev, option_containing};
use jevify::config::Backend;
use jevify::jev::client::Client;
use jevify::tournament::{Finalists, Prompts, rank};

fn prompts() -> Prompts {
    Prompts {
        choose: "Which item?".into(),
        none: "none".into(),
        any: "Any?".into(),
    }
}

fn needle_fake() -> FakeJev {
    FakeJev {
        choose: |_, s, o| option_containing(s, o, "NEEDLE"),
        noul: |_, s| {
            if s.to_string().contains("NEEDLE") {
                0.95
            } else {
                0.05
            }
        },
    }
}

#[tokio::test]
async fn both_backends_find_the_needle_across_windows_and_refuse_a_list_past_capacity() {
    for backend in [Backend::Typesafe, Backend::Classifier] {
        // FakeClassifier rejects any request exceeding the backend's wire limits.
        let server = match backend {
            Backend::Typesafe => common::mock(needle_fake()).await,
            Backend::Classifier => common::mock_classifier(needle_fake()).await,
        };
        let mut cfg = common::config(&server);
        cfg.backend = backend;
        let client = Client::new(&cfg).unwrap();
        let w = backend.window();
        for (count, index) in [(1, 0), (w, w - 1), (w + 1, w), (1000, 777)] {
            let mut items: Vec<String> = (0..count).map(|i| format!("line {i}")).collect();
            items[index] = "the NEEDLE is here".into();
            let r = rank(
                &client,
                "find the needle",
                &items,
                &prompts(),
                None,
                Finalists::Auto,
            )
            .await
            .unwrap();
            assert_eq!(r.candidates[0].index, index, "{count} items");
            assert!(r.any > 0.9, "{count} items: {}", r.any);
        }
        let before = server.received_requests().await.unwrap().len();
        let error = rank(
            &client,
            "needle",
            &vec!["item".into(); w * w + 1],
            &prompts(),
            None,
            Finalists::Auto,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            jevify::exit::JevifyError::Kinded {
                kind: "too_many",
                ..
            }
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), before);
    }
}

#[tokio::test]
async fn long_lines_are_clipped_so_a_window_stays_under_budget() {
    let server = common::mock_classifier(FakeJev {
        choose: |_, s, o| {
            // FakeClassifier also checks the backend's serialized UTF-16 input limit.
            assert!(
                s["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|i| i.as_str().unwrap().ends_with('…')),
                "item not clipped"
            );
            option_containing(s, o, "NEEDLE")
        },
        noul: |_, _| 0.9,
    })
    .await;
    let mut cfg = common::config(&server);
    cfg.backend = Backend::Classifier;
    let client = Client::new(&cfg).unwrap();
    let mut items: Vec<String> = (0..cfg.backend.window())
        .map(|i| format!("{i} {}", "x".repeat(5_000)))
        .collect();
    items[3] = format!("NEEDLE {}", "y".repeat(5_000));
    let r = rank(
        &client,
        "find the needle",
        &items,
        &prompts(),
        None,
        Finalists::Auto,
    )
    .await
    .unwrap();
    assert_eq!(r.candidates[0].index, 3);
}

#[tokio::test]
async fn ranked_finals_give_leaders_more_evidence_within_backend_limits() {
    for backend in [Backend::Typesafe, Backend::Classifier] {
        let fake = FakeJev {
            choose: |_, state, options| {
                let items = state["items"].as_array().unwrap();
                if items[0].as_str().unwrap().contains("ENRICHED") {
                    assert!(items[0].as_str().unwrap().contains("NEEDLE"));
                    let first = items[0].as_str().unwrap().chars().count();
                    let last = items.last().unwrap().as_str().unwrap().chars().count();
                    assert!(first > last, "leading candidate lost its extra evidence");
                    assert!(
                        items
                            .iter()
                            .all(|item| item.as_str().unwrap().ends_with('…'))
                    );
                }
                option_containing(state, options, "NEEDLE")
            },
            noul: |_, _| 0.95,
        };
        let server = match backend {
            Backend::Typesafe => common::mock(fake).await,
            Backend::Classifier => common::mock_classifier(fake).await,
        };
        let mut cfg = common::config(&server);
        cfg.backend = backend;
        let client = Client::new(&cfg).unwrap();
        let count = backend.window() * 3 / 4;
        let mut items = vec!["other".to_owned(); count];
        items[count - 1] = "NEEDLE".to_owned();
        let evidence = |i| format!("{} ENRICHED {}", items[i], "x".repeat(5_000));
        let ranking = rank(
            &client,
            "needle",
            &items,
            &prompts(),
            Some(&evidence),
            Finalists::Fixed(count),
        )
        .await
        .unwrap();
        assert_eq!(ranking.candidates[0].index, count - 1);
    }
}

/// The one decision rule: a clear winner above the threshold is found; below the threshold or
/// with NONE ahead there is no match; two close rivals are ambiguous, both named.
#[test]
fn decide_applies_the_threshold_none_and_the_near_tie() {
    use jevify::tournament::{Candidate, Decision, Ranking, decide};
    let ranking = |candidates: &[(usize, f64)], any: f64, none: f64| Ranking {
        candidates: candidates
            .iter()
            .map(|&(index, p)| Candidate { index, p })
            .collect(),
        any,
        none,
        windows: 1,
        n: 1,
    };
    assert!(matches!(
        decide(&ranking(&[(2, 0.8), (0, 0.1)], 0.9, 0.1), 0.5),
        Decision::Found(c) if c.index == 2
    ));
    assert!(matches!(
        decide(&ranking(&[(2, 0.8), (0, 0.1)], 0.4, 0.1), 0.5),
        Decision::NoMatch
    ));
    assert!(matches!(
        decide(&ranking(&[(2, 0.3), (0, 0.1)], 0.9, 0.5), 0.5),
        Decision::NoMatch
    ));
    assert!(matches!(
        decide(&ranking(&[(2, 0.5), (7, 0.4)], 0.9, 0.1), 0.5),
        Decision::Ambiguous(ref pair) if pair[0].index == 2 && pair[1].index == 7
    ));
    assert!(matches!(
        decide(&ranking(&[], 0.9, 0.1), 0.5),
        Decision::NoMatch
    ));
}
