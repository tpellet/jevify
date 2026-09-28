mod common;
use common::FakeJev;

async fn sort_with_folder_count(classifier: bool, folders: usize) -> serde_json::Value {
    let fake = FakeJev {
        choose: |_, _, _| "NONE".into(),
        noul: |_, _| 0.1,
    };
    let server = if classifier {
        common::mock_classifier(fake).await
    } else {
        common::mock(fake).await
    };
    let d = tempfile::tempdir().unwrap();
    for i in 0..folders {
        std::fs::create_dir(d.path().join(format!("folder-{i:03}"))).unwrap();
    }
    std::fs::write(
        d.path().join("unknown.txt"),
        "nothing identifies a destination",
    )
    .unwrap();
    let mut command = if classifier {
        common::jevify_classifier(&server)
    } else {
        common::jevify(&server)
    };
    let out = command
        .args(["--json", "sort", d.path().to_str().unwrap()])
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_sorted_alone_among_the_folders_beside_it() {
    let server = common::mock(FakeJev {
        choose: |_, _, options| {
            options
                .iter()
                .find(|o| o.as_str() != "NONE")
                .unwrap()
                .clone()
        },
        noul: |_, _| 0.9,
    })
    .await;
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("invoices")).unwrap();
    std::fs::write(d.path().join("bill.txt"), "invoice 42").unwrap();
    std::fs::write(d.path().join("other.txt"), "a letter").unwrap();
    let file = d.path().join("bill.txt");
    let mut command = common::jevify(&server);
    let out = tokio::task::spawn_blocking(move || {
        command
            .args(["--json", "sort", file.to_str().unwrap()])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["exit_code"], 0, "{v}");
    let moves = v["data"]["moves"].as_array().unwrap();
    assert_eq!(moves.len(), 1, "{v}");
    assert!(
        moves[0]["from"].as_str().unwrap().ends_with("bill.txt"),
        "{v}"
    );
    // A dry run: nothing moved, and the other file was not considered.
    assert!(d.path().join("bill.txt").exists() && d.path().join("other.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn destination_capacity_is_backend_aware_without_dropping_folders() {
    for folders in [99, 100, 200] {
        let v = sort_with_folder_count(false, folders).await;
        assert_eq!(v["exit_code"], 3, "TypeSafe with {folders} folders: {v}");
        assert_eq!(v["data"]["moves"].as_array().unwrap().len(), 0, "{v}");
    }

    let v = sort_with_folder_count(true, 99).await;
    assert_eq!(v["exit_code"], 3, "classifier with 99 folders: {v}");
    assert_eq!(v["data"]["moves"].as_array().unwrap().len(), 0, "{v}");

    for folders in [100, 200] {
        let v = sort_with_folder_count(true, folders).await;
        assert_eq!(v["exit_code"], 6, "classifier with {folders} folders: {v}");
        assert_eq!(v["error"]["kind"], "input_too_large", "{v}");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("at most 99 destination folders"),
            "{v}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn dangling_targets_and_outside_symlinks_are_not_followed() {
    use std::os::unix::fs::symlink;
    let server = common::mock(FakeJev {
        choose: |_, s, o| {
            common::option_containing(&serde_json::json!({"items": s["folders"]}), o, "Finance")
        },
        noul: |_, _| 0.9,
    })
    .await;
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("Finance")).unwrap();
    std::fs::write(d.path().join("invoice.txt"), "invoice").unwrap();
    std::fs::write(outside.path().join("secret.txt"), "outside private text").unwrap();
    symlink(
        outside.path().join("secret.txt"),
        d.path().join("secret.txt"),
    )
    .unwrap();
    symlink(outside.path(), d.path().join("Outside")).unwrap();
    symlink("missing", d.path().join("Finance/invoice.txt")).unwrap();
    let out = common::jevify(&server)
        .env("JEVIFY_CACHE_DIR", cache.path())
        .env_remove("JEVIFY_NO_CACHE")
        .args(["--json", "sort", d.path().to_str().unwrap(), "--apply"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(d.path().join("invoice.txt").exists(), "{v}");
    assert!(
        std::fs::symlink_metadata(d.path().join("Finance/invoice.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let requests = server.received_requests().await.unwrap();
    for request in requests {
        let body = String::from_utf8_lossy(&request.body);
        assert!(!body.contains("outside private text"), "{body}");
        assert!(!body.contains("Outside"), "{body}");
    }
    assert!(
        v["data"]["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["reason"].as_str().unwrap().contains("symlink")),
        "{v}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn relative_apply_journal_undo_is_independent_of_working_directory() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| {
            common::option_containing(&serde_json::json!({"items": s["folders"]}), o, "Finance")
        },
        noul: |_, _| 0.9,
    })
    .await;
    let d = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("Finance")).unwrap();
    let filename = "invoice\twith\nlines.txt";
    std::fs::write(d.path().join(filename), "invoice").unwrap();
    let out = common::jevify(&server)
        .current_dir(d.path())
        .env("JEVIFY_CACHE_DIR", cache.path())
        .env_remove("JEVIFY_NO_CACHE")
        .args(["--json", "sort", ".", "--apply"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["exit_code"], 0, "{v}");
    let log = v["data"]["undo_log"].as_str().unwrap();
    assert!(std::path::Path::new(log).is_absolute());
    assert!(d.path().join("Finance").join(filename).exists());
    let out = common::bin()
        .current_dir(elsewhere.path())
        .args(["--json", "sort", ".", "--undo", log])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["exit_code"], 0, "{v}");
    assert_eq!(
        std::fs::read_to_string(d.path().join(filename)).unwrap(),
        "invoice"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_then_apply_then_undo() {
    let server = common::mock(FakeJev {
        choose: |i, s, o| {
            // file k -> "Finance" folder if its text mentions invoice
            let k: usize = i
                .split("files[")
                .nth(1)
                .and_then(|r| r.split(']').next())
                .and_then(|n| n.parse().ok())
                .unwrap();
            let f = s["files"][k].as_str().unwrap_or_default();
            if f.contains("invoice") {
                common::option_containing(&serde_json::json!({"items": s["folders"]}), o, "Finance")
            } else {
                "NONE".into()
            }
        },
        // The absolute Noul gates the move: a file the Choice would file under Finance but whose
        // Noul says nothing fits ("unsure") must stay where it is.
        noul: |i, s| {
            let k: usize = i
                .split("files[")
                .nth(1)
                .and_then(|r| r.split(']').next())
                .and_then(|n| n.parse().ok())
                .unwrap();
            if s["files"][k]
                .as_str()
                .unwrap_or_default()
                .contains("unsure")
            {
                0.2
            } else {
                0.9
            }
        },
    })
    .await;
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("Finance")).unwrap();
    std::fs::write(d.path().join("march.txt"), "invoice total 42 EUR").unwrap();
    std::fs::write(d.path().join("poem.txt"), "roses are red").unwrap();
    std::fs::write(d.path().join("draft.txt"), "invoice? unsure, maybe a quote").unwrap();
    let cache = tempfile::tempdir().unwrap();
    let run = |args: Vec<String>| {
        let mut c = common::jevify(&server);
        c.env("JEVIFY_CACHE_DIR", cache.path())
            .env_remove("JEVIFY_NO_CACHE");
        let a = args.clone();
        async move {
            tokio::task::spawn_blocking(move || c.args(a).output().unwrap())
                .await
                .unwrap()
        }
    };
    let dir = d.path().to_string_lossy().to_string();
    let out = run(vec!["--json".into(), "sort".into(), dir.clone()]).await;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["moves"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["data"]["skipped"].as_array().unwrap().len(), 2, "{v}");
    assert!(d.path().join("march.txt").exists(), "dry-run must not move");
    let out = run(vec![
        "--json".into(),
        "sort".into(),
        dir.clone(),
        "--apply".into(),
    ])
    .await;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(d.path().join("Finance/march.txt").exists());
    assert!(
        d.path().join("draft.txt").exists(),
        "a low `any` must not move the file"
    );
    let log = v["data"]["undo_log"].as_str().unwrap().to_string();
    run(vec!["sort".into(), dir, "--undo".into(), log]).await;
    assert!(d.path().join("march.txt").exists());
}

// A journal as a crashed `--apply` leaves it, with no completion line, read by a fresh
// process: an intent whose rename never happened is skipped, an intent whose completion
// line was lost is restored, and an empty journal restores nothing (exit 3).
#[test]
fn undo_reads_the_journal_a_crashed_apply_left_behind() {
    use std::os::unix::fs::MetadataExt;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("Finance")).unwrap();
    let pending = (root.join("pending.txt"), root.join("Finance/pending.txt"));
    let moved = (root.join("moved.txt"), root.join("Finance/moved.txt"));
    std::fs::write(&pending.0, "pending").unwrap();
    std::fs::write(&moved.1, "moved").unwrap();
    let intent = |id: usize, (from, to): &(std::path::PathBuf, std::path::PathBuf)| {
        let m = std::fs::metadata(if id == 0 { from } else { to }).unwrap();
        serde_json::json!({
            "event": "intent", "id": id,
            "file": {
                "from": from.to_str().unwrap().as_bytes(),
                "to": to.to_str().unwrap().as_bytes(),
                "dev": m.dev(), "ino": m.ino(),
            }
        })
    };
    let log = root.join("sort-undo-crashed.jsonl");
    std::fs::write(
        &log,
        format!("{}\n{}\n", intent(0, &pending), intent(1, &moved)),
    )
    .unwrap();
    let out = common::bin()
        .args(["--json", "sort", ".", "--undo"])
        .arg(&log)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["exit_code"], 0, "{v}");
    assert_eq!(v["data"]["moves"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(
        v["data"]["skipped"][0]["reason"], "original path occupied or intent not performed",
        "{v}"
    );
    assert_eq!(std::fs::read_to_string(&moved.0).unwrap(), "moved");
    assert_eq!(std::fs::read_to_string(&pending.0).unwrap(), "pending");
    assert!(!moved.1.exists() && !pending.1.exists());
    let empty = root.join("sort-undo-empty.jsonl");
    std::fs::write(&empty, "").unwrap();
    let out = common::bin()
        .args(["--json", "sort", ".", "--undo"])
        .arg(&empty)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["exit_code"], 3, "{v}");
}
