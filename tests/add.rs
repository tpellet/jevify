mod common;
use common::FakeJev;
use std::process::Command as P;

fn git(dir: &std::path::Path, args: &[&str]) {
    assert!(
        P::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success()
    );
}

/// An initialised repository that can commit.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-q"]);
    for (key, value) in [
        ("user.email", "t@t"),
        ("user.name", "t"),
        ("commit.gpgsign", "false"),
    ] {
        git(d.path(), &["config", key, value]);
    }
    d
}

fn commit_all(dir: &std::path::Path) {
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "init"]);
}

fn staged(dir: &std::path::Path, args: &[&str]) -> String {
    let out = P::new("git")
        .args(["diff", "--cached"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

fn yes_to_auth() -> FakeJev {
    FakeJev {
        choose: |_, _, o| o[0].clone(),
        noul: |i, s| {
            let idx: usize = i
                .split("hunks[")
                .nth(1)
                .and_then(|r| r.split(']').next())
                .and_then(|n| n.parse().ok())
                .unwrap();
            let h = s["hunks"][idx].as_str().unwrap_or_default().to_string();
            if h.contains("AUTH") { 0.95 } else { 0.05 }
        },
    }
}

/// A diff past the request cap is an input error before any request or staging: the index is
/// left as it was.
#[tokio::test(flavor = "multi_thread")]
async fn oversized_diff_is_rejected_without_a_request_or_staging() {
    let server = common::mock_classifier(yes_to_auth()).await;
    let d = repo();
    for i in 0..20 {
        std::fs::write(d.path().join(format!("f{i}.txt")), "original\n").unwrap();
    }
    git(d.path(), &["add", "."]);
    for i in 0..20 {
        std::fs::write(
            d.path().join(format!("f{i}.txt")),
            "ordinary words ".repeat(150),
        )
        .unwrap();
    }
    let before = staged(d.path(), &[]);
    let mut c = common::jevify_classifier(&server);
    c.current_dir(d.path());
    let out = tokio::task::spawn_blocking(move || {
        c.args(["--json", "add", "--yes", "ordinary words"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(6), "{v}");
    assert_eq!(v["error"]["kind"], "input_too_large");
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(staged(d.path(), &[]), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn stages_only_matching_hunks() {
    let server = common::mock(yes_to_auth()).await;
    let d = repo();
    let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
    std::fs::write(d.path().join("f.txt"), &body).unwrap();
    commit_all(d.path());
    let changed = body
        .replace("line 2\n", "line 2 AUTH fix\n")
        .replace("line 35\n", "line 35 typo\n");
    std::fs::write(d.path().join("f.txt"), changed).unwrap();
    let mut c = common::jevify(&server);
    c.current_dir(d.path());
    let out = tokio::task::spawn_blocking(move || {
        c.args(["add", "--yes", "the auth fix"]).output().unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let staged = staged(d.path(), &[]);
    assert!(staged.contains("AUTH") && !staged.contains("typo"));
}

/// Missing confirmation (exit 2 before any request), dry-run and abstention stage nothing.
#[tokio::test(flavor = "multi_thread")]
async fn usage_dry_run_and_abstention_stage_nothing() {
    let server = common::mock(yes_to_auth()).await;
    let d = repo();
    std::fs::write(d.path().join("f.txt"), "a\n").unwrap();
    commit_all(d.path());
    for (change, args, code) in [
        ("a\n", vec!["--json", "add", "anything"], 2),
        ("b AUTH\n", vec!["--json", "add", "anything"], 2),
        ("b AUTH\n", vec!["add", "--dry-run", "anything"], 0),
        ("b typo\n", vec!["--json", "add", "--yes", "anything"], 3),
    ] {
        std::fs::write(d.path().join("f.txt"), change).unwrap();
        let mut c = common::jevify(&server);
        c.current_dir(d.path()).args(&args);
        let out = tokio::task::spawn_blocking(move || c.output().unwrap())
            .await
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(code),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(staged(d.path(), &[]), "", "{args:?}");
        if code == 2 {
            assert!(server.received_requests().await.unwrap().is_empty());
            let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(v["error"]["kind"], "usage");
            let example = v["error"]["example"].as_str().unwrap();
            assert!(example.contains("--yes") || example.contains("--dry-run"));
        }
    }
}

/// A real terminal refusal remains exit 130; an index lock makes git reject staging atomically.
#[tokio::test(flavor = "multi_thread")]
async fn terminal_decline_and_apply_rejection_stage_nothing() {
    let server = common::mock(yes_to_auth()).await;
    let d = repo();
    std::fs::write(d.path().join("f.txt"), "a\n").unwrap();
    commit_all(d.path());
    std::fs::write(d.path().join("f.txt"), "b AUTH\n").unwrap();
    let mut terminal = assert_cmd::Command::new("script");
    terminal
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TYPESAFE_API_KEY", "test-key")
        .env("JEVIFY_BASE_URL", server.uri())
        .env("JEVIFY_NO_CACHE", "1")
        .env("JEVIFY_CONFIG_DIR", d.path())
        .current_dir(d.path())
        .timeout(std::time::Duration::from_secs(15));
    let binary = assert_cmd::cargo::cargo_bin("jevify");
    #[cfg(target_os = "macos")]
    terminal
        .args(["-q", "/dev/null"])
        .arg(&binary)
        .args(["add", "anything"]);
    #[cfg(target_os = "linux")]
    terminal
        .args(["-q", "-e", "-c"])
        .arg(format!(
            "{} add anything",
            jevify::argv::quote(binary.to_str().unwrap())
        ))
        .arg("/dev/null");
    let out = tokio::task::spawn_blocking(move || terminal.write_stdin("n\n").output().unwrap())
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(130), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Stage 1 hunk(s)?"));
    assert_eq!(staged(d.path(), &[]), "");

    std::fs::write(d.path().join(".git/index.lock"), "locked by test\n").unwrap();
    let mut command = common::jevify(&server);
    command
        .current_dir(d.path())
        .args(["add", "--yes", "--json", "anything"]);
    let out = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(6), "{value}");
    assert_eq!(value["error"]["kind"], "input");
    assert_eq!(staged(d.path(), &[]), "");
}

/// From a subdirectory, hunks in files outside it are still staged: git runs at the top level
/// (`git apply` from `sub/` would skip `top.txt` and still exit 0).
#[tokio::test(flavor = "multi_thread")]
async fn stages_from_a_subdirectory() {
    let server = common::mock(yes_to_auth()).await;
    let d = repo();
    std::fs::create_dir(d.path().join("sub")).unwrap();
    std::fs::write(d.path().join("top.txt"), "a\n").unwrap();
    commit_all(d.path());
    std::fs::write(d.path().join("top.txt"), "b AUTH\n").unwrap();
    let mut c = common::jevify(&server);
    c.current_dir(d.path().join("sub"));
    let out =
        tokio::task::spawn_blocking(move || c.args(["add", "--yes", "anything"]).output().unwrap())
            .await
            .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(staged(d.path(), &["--name-only"]).trim(), "top.txt");
}

/// A clean tree has nothing to stage: exit 6 (input) with a hint written for `add`, which
/// reads `git diff`, not stdin.
#[tokio::test(flavor = "multi_thread")]
async fn clean_tree_is_an_input_error_with_a_hint() {
    let server = common::mock(yes_to_auth()).await;
    let d = repo();
    std::fs::write(d.path().join("f.txt"), "a\n").unwrap();
    commit_all(d.path());
    let mut c = common::jevify(&server);
    c.current_dir(d.path());
    let out = tokio::task::spawn_blocking(move || {
        c.args(["--json", "add", "--yes", "anything"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(out.status.code(), Some(6));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["kind"], "empty_input");
    let hint = v["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("git diff"), "{hint}");
}
