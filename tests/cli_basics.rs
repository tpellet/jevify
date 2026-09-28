use assert_cmd::cargo::CommandCargoExt;
mod common;

/// Every usage error is exit 2 with nothing on stdout; under `--json` it is one envelope with
/// `error.kind` usage, a hint, a copyable example and no request. Where jevify corrects the
/// caller's command, the example is that correction.
#[test]
fn usage_errors_are_exit_2_and_one_envelope_with_a_corrected_example() {
    for (args, example) in [
        (vec![], None),
        (vec!["--nope", "why"], None),
        (vec!["pick", "--nope", "x"], None),
        (vec!["-t", "2", "is", "x"], None),
        (vec!["is", "--band", "0.9", "x"], None),
        (vec!["why", "-0"], None),
        (vec!["why", "--", "true"], None),
        (vec!["run", "x"], None),
        (vec!["route", "x"], None),
        (vec!["sort", "."], None),
        (vec!["robot-docs"], None),
        (vec!["init", "zsh"], None),
        (vec!["init", "bash"], None),
        (vec!["--format", "human", "capabilities"], None),
        (vec!["--format", "json", "capabilities"], None),
        (vec!["--format=jsonl", "capabilities"], None),
        (vec!["--format", "toon", "capabilities"], None),
        (vec!["fill", "--nope", "--", "x"], None),
        (vec!["pick", "--from", "branch", "-0", "x"], None),
        (vec!["label", "bug"], None),
        (vec!["label", "bug,bug"], None),
        (vec!["label", "bug,,feature"], None),
        (vec!["label", "bug,?"], None),
        (vec!["label", "-0", "--para", "bug,feature"], None),
        (vec!["pik", "the", "fix"], Some("jevify pick 'the fix'")),
        (
            vec!["label", "bug", "feature"],
            Some("jevify label bug,feature"),
        ),
        (
            vec!["is", "a", "refund", "-v"],
            Some("jevify is 'a refund'"),
        ),
        (
            vec!["pick"],
            Some("jevify pick '<what the line you want says>'"),
        ),
        (
            vec!["fill", "git", "switch", "@{branch:x}"],
            Some("jevify fill -- git switch '@{branch:x}'"),
        ),
        (vec!["pick", "-n", "0", "x"], Some("jevify pick -n 1 x")),
    ] {
        let out = common::bin()
            .args(&args)
            .write_stdin("a\n")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        if let Some(example) = example {
            let stderr = String::from_utf8(out.stderr).unwrap();
            assert!(stderr.contains(example), "{args:?}: {stderr}");
        }
        let out = common::bin()
            .arg("--json")
            .args(&args)
            .write_stdin("a\n")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["ok"], false, "{args:?}");
        assert_eq!(value["exit_code"], 2, "{args:?}");
        assert_eq!(value["error"]["kind"], "usage", "{args:?}");
        assert!(
            !value["error"]["hint"].as_str().unwrap().is_empty(),
            "{args:?}"
        );
        assert!(
            value["error"]["example"]
                .as_str()
                .unwrap()
                .starts_with("jevify"),
            "{args:?}"
        );
        assert!(value["data"].is_null(), "{args:?}");
        assert_eq!(value["meta"]["requests"], 0, "{args:?}");
        if let Some(example) = example {
            assert_eq!(
                value["error"]["example"],
                example.replacen("jevify", "jevify --json", 1),
                "{args:?}"
            );
        }
    }
}

/// A verb's options are read wherever an agent puts them: before the verb they still belong
/// to it, so `why` with no input is an input error (6), not a usage error about `--no-save`.
#[test]
fn verb_options_before_the_verb_are_read_as_the_verbs() {
    for args in [
        vec!["--no-save", "why"],
        vec!["-C", "5", "why"],
        vec!["--json", "--no-save", "why"],
    ] {
        let out = common::bin().args(&args).write_stdin("").output().unwrap();
        assert_eq!(out.status.code(), Some(6), "{args:?} {out:?}");
    }
}

/// Clap failures keep non-UTF-8 arguments and stop scanning for `--json` at `--`: exit 2 and
/// no panic, with the envelope only when `--json` came before the verb.
#[test]
fn clap_errors_preserve_non_utf8_args_and_stop_format_scanning_at_double_dash() {
    use std::os::unix::ffi::OsStringExt;
    for json in [false, true] {
        let mut cmd = common::bin();
        if json {
            cmd.arg("--json");
        }
        let out = cmd
            .args(["pick", "q", "--", "--json"])
            .arg(std::ffi::OsString::from_vec(vec![0xff]))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
        if json {
            let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(value["error"]["kind"], "usage");
        } else {
            assert!(out.stdout.is_empty());
        }
    }
}

#[test]
fn closed_stdout_is_normal_for_output_and_json_errors() {
    for args in [vec!["capabilities"], vec!["--json", "pick", "--nope"]] {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let out = std::process::Command::cargo_bin("jevify")
            .unwrap()
            .args(args)
            .stdout(writer)
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
    }
}

#[test]
fn json_and_robot_emit_one_line_without_pricing_fields() {
    for flag in ["--json", "--robot"] {
        let out = common::bin()
            .env("JEVIFY_PRICE_PER_MTOK", "not-a-price")
            .args([flag, "capabilities"])
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 1);
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(v["meta"].get("cost_usd").is_none());
        assert!(v["meta"]["telemetry"].get("cost_estimate").is_none());
        assert!(
            v["data"]["global_flags"]
                .as_array()
                .unwrap()
                .iter()
                .all(|flag| !flag.as_str().unwrap().starts_with("--format"))
        );
    }
}
