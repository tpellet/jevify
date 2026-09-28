//! `bin()` isolates every test from the developer's own recipes: it points `JEVIFY_CONFIG_DIR`
//! at a fresh empty directory, so the platform configuration directory is never read.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

const WIDGET: &str = "{\"kind\":\"widget\",\"list\":[\"printf\",\"w1\\\\n\"]}\n";

/// A temporary home with a `kinds.jsonl` in the directory that `directories::ProjectDirs`
/// names under that home, and the variables that redirect the platform lookup to it.
fn planted_home() -> (PathBuf, Vec<(&'static str, PathBuf)>) {
    let home = tempfile::tempdir().unwrap().keep();
    let real = jevify::config::config_dir(None).expect("a platform configuration directory");
    let (dir, vars) = if cfg!(target_os = "macos") {
        let real_home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        let relative = real.strip_prefix(&real_home).expect("under HOME");
        (home.join(relative), vec![("HOME", home.clone())])
    } else {
        let xdg = home.join(".config");
        (
            xdg.join(real.file_name().expect("an application directory")),
            vec![("HOME", home.clone()), ("XDG_CONFIG_HOME", xdg)],
        )
    };
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("kinds.jsonl"), WIDGET).unwrap();
    (dir, vars)
}

#[test]
fn bin_never_reads_the_platform_configuration_directory() {
    let (planted, vars) = planted_home();
    let mut cmd = common::bin();
    for (name, value) in &vars {
        cmd.env(name, value);
    }
    let out = cmd
        .args(["fill", "--dry-run", "--", "printf", "@{widget:x}"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown kind"));

    // The planted file is valid and resolves the kind from its directory.
    let env = jevify::source::Env {
        path: "/usr/bin:/bin".into(),
        config_dir: Some(planted.clone()),
        deadline: Instant::now() + Duration::from_secs(20),
        cwd: planted,
        cache_dir: None,
    };
    let widget = jevify::source::lookup("widget", &env).unwrap().unwrap();
    assert_eq!(widget.name, "widget");
}
