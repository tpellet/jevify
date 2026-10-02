//! `meta.decision`: verb, backend, requested and answering model, threshold and the gate
//! scores of every decision.
mod common;

use common::FakeJev;
use serde_json::Value;

/// Runs the binary with `--json` and returns the envelope.
async fn run(mut c: assert_cmd::Command, args: &[&str], stdin: &str) -> Value {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let stdin = stdin.to_string();
    let out = tokio::task::spawn_blocking(move || {
        c.arg("--json")
            .args(args)
            .write_stdin(stdin)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let parsed = serde_json::from_slice(&out.stdout);
    assert!(
        parsed.is_ok(),
        "no envelope: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    parsed.unwrap()
}

/// The fields every verb carries; returns the gates.
fn decision(v: &Value, verb: &str, backend: &str) -> Vec<Value> {
    let d = &v["meta"]["decision"];
    assert_eq!(d["verb"], verb, "{v}");
    assert_eq!(d["backend"], backend, "{v}");
    assert_eq!(d["backend"], v["meta"]["backend"], "{v}");
    assert_eq!(d["threshold"], 0.5, "{v}");
    assert_eq!(d["threshold"], v["meta"]["threshold"], "{v}");
    if backend == "typesafe" {
        assert_eq!(d["model"]["requested"], "jev-1.13.0", "{v}");
    } else {
        assert!(d["model"]["requested"].is_null(), "{v}");
    }
    assert_eq!(d["model"]["answering"], "jev-fake", "{v}");
    assert_eq!(d["model"]["answering"], v["meta"]["model"], "{v}");
    let gates = d["gates"].as_array().unwrap().clone();
    for gate in &gates {
        for score in ["best", "next", "none", "any", "fails"] {
            assert!(gate[score].is_null() || gate[score].is_number(), "{v}");
        }
    }
    gates
}

fn noul_gate(p: f64) -> Value {
    serde_json::json!({ "best": null, "next": null, "none": null, "any": p, "fails": null })
}

#[tokio::test(flavor = "multi_thread")]
async fn is_records_one_noul_gate_per_statement() {
    let server = common::mock(FakeJev {
        choose: |_, _, o| o[0].clone(),
        noul: |i, _| if i.contains("first") { 0.81 } else { 0.2 },
    })
    .await;
    let v = run(common::jevify(&server), &["is", "first", "second"], "text").await;
    assert_eq!(
        decision(&v, "is", "typesafe"),
        vec![noul_gate(0.81), noul_gate(0.2)]
    );
    // round_one is opt-in: absent from a default run.
    assert!(v["meta"]["decision"].get("round_one").is_none(), "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn filter_records_one_three_way_gate_per_judged_record() {
    let server = common::mock_classifier(FakeJev {
        choose: |_, state, options| {
            let side = match state.as_str() {
                Some("no") => "does not hold",
                Some("silent") => "does not say",
                _ => "statement holds",
            };
            options
                .iter()
                .find(|option| option.ends_with(side))
                .cloned()
                .expect("one of the three filter options ends with the side")
        },
        noul: |_, _| unreachable!(),
    })
    .await;
    let v = run(
        common::jevify_classifier(&server),
        &["filter", "x"],
        "yes\nno\nyes\nsilent\n",
    )
    .await;
    // A duplicate record is judged once: three gates for four records. `any` is P(holds),
    // `fails` is P(does not hold) and `none` is P(the record does not say); the fake gives 0.9
    // to its pick and 0.05 to the rest.
    let gates = decision(&v, "filter", "classifier");
    assert_eq!(gates.len(), 3, "{v}");
    let sides = [(0.9, 0.05, 0.05), (0.05, 0.9, 0.05), (0.05, 0.05, 0.9)];
    for (gate, (holds, fails, silent)) in gates.iter().zip(sides) {
        for (score, expected) in [("any", holds), ("fails", fails), ("none", silent)] {
            assert!(
                (gate[score].as_f64().unwrap() - expected).abs() < 1e-9,
                "{v}"
            );
        }
        assert!(gate["best"].is_null() && gate["next"].is_null(), "{v}");
    }
    let kept: Vec<&str> = v["data"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["text"].as_str().unwrap())
        .collect();
    assert_eq!(kept, ["yes\n", "yes\n", "silent\n"], "{v}");
    assert_eq!(
        (&v["data"]["kept"], &v["data"]["unsure"]),
        (&3.into(), &1.into())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn label_records_choice_gates_without_a_noul() {
    let server = common::mock_classifier(
        FakeJev {
            choose: |_, _, _| "NONE".into(),
            noul: |_, _| unreachable!(),
        }
        .with_probabilities(|_, _, labels| {
            labels
                .iter()
                .map(|l| match l.as_str() {
                    "bug" => 0.8,
                    "NONE" => 0.05,
                    _ => 0.15,
                })
                .collect()
        }),
    )
    .await;
    let v = run(
        common::jevify_classifier(&server),
        &["filter", "--label", "bug,feature"],
        "crash\n",
    )
    .await;
    let gates = decision(&v, "filter", "classifier");
    assert_eq!(
        gates,
        vec![
            serde_json::json!({"best": 0.8, "next": 0.15, "none": 0.05, "any": null, "fails": null})
        ],
        "{v}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn why_and_pick_record_the_ranking_gate_and_round_one_on_request() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| common::option_containing(s, o, "ROOT"),
        noul: |_, _| 0.95,
    })
    .await;
    let lines = "error step\nerror ROOT\nerror other\n";
    for (verb, args) in [
        ("why", &["why", "--no-save"][..]),
        ("pick", &["pick", "the root"][..]),
    ] {
        let mut c = common::jevify(&server);
        c.env("JEVIFY_DECISION", "round_one");
        let v = run(c, args, lines).await;
        assert_eq!(v["exit_code"], 0, "{v}");
        let gates = decision(&v, verb, "typesafe");
        assert_eq!(gates.len(), 1, "{v}");
        assert!(
            gates[0]["best"].as_f64().unwrap() > gates[0]["none"].as_f64().unwrap(),
            "{v}"
        );
        assert!(gates[0]["next"].is_number(), "{v}");
        assert_eq!(gates[0]["any"], 0.95, "{v}");
        // Round one of the tournament: every line ranked, ROOT first.
        let windows = v["meta"]["decision"]["round_one"][0]["windows"]
            .as_array()
            .unwrap();
        assert_eq!(windows[0]["ranks"][0]["index"], 2, "{v}");
        assert_eq!(windows[0]["any"], 0.95, "{v}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn fill_records_one_gate_per_marker_in_argv_order() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| common::option_containing(s, o, "beta"),
        noul: |i, _| if i.contains("draft") { 0.9 } else { 0.5 },
    })
    .await;
    let v = run(
        common::jevify(&server),
        &[
            "fill",
            "--dry-run",
            "--",
            "printf",
            "@{flag:--draft:draft}",
            "@{one:alpha|beta|gamma:which one}",
        ],
        "context",
    )
    .await;
    let gates = decision(&v, "fill", "typesafe");
    assert_eq!(gates.len(), 2, "{v}");
    // The flag marker is a Noul; the `one` marker is a Choice with no Noul.
    assert_eq!(gates[0], noul_gate(0.9), "{v}");
    assert!(
        gates[1]["best"].is_number() && gates[1]["none"].is_number() && gates[1]["any"].is_null(),
        "{v}"
    );
    assert_eq!(v["data"]["markers"].as_array().unwrap().len(), 2, "{v}");
}

#[tokio::test(flavor = "multi_thread")]
async fn answering_model_is_unknown_when_the_service_names_none() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let answers: serde_json::Map<String, Value> = body["questions"]
                .as_object()
                .unwrap()
                .keys()
                .map(|id| (id.clone(), serde_json::json!({"noul":0.9})))
                .collect();
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"answers":answers}))
        })
        .mount(&server)
        .await;
    let v = run(common::jevify(&server), &["is", "holds"], "evidence").await;
    let d = &v["meta"]["decision"];
    assert_eq!(d["model"]["requested"], "jev-1.13.0", "{v}");
    assert_eq!(d["model"]["answering"], "unknown", "{v}");
    assert_eq!(v["meta"]["model"], "unknown", "{v}");
    assert_eq!(d["gates"], serde_json::json!([noul_gate(0.9)]), "{v}");
}

#[tokio::test]
async fn a_usage_error_carries_the_verb_and_an_unknown_model() {
    let v = run(common::bin(), &["is"], "").await;
    assert_eq!(v["exit_code"], 2, "{v}");
    let d = &v["meta"]["decision"];
    assert_eq!(d["verb"], "is", "{v}");
    assert_eq!(d["model"]["answering"], "unknown", "{v}");
    assert!(d["model"]["requested"].is_null(), "{v}");
    assert_eq!(d["gates"], serde_json::json!([]), "{v}");
}
