//! The end-to-end suite: the built `jevify` binary against the live backend, on real
//! repositories, CI logs and records, every case with a gold answer made before this suite.
//!
//! **What is asserted.** The decision (the exit code) and the chosen item: the handle, the
//! line number, the label, the kept records, the hunk; for `fill`, whether the command ran,
//! read from `JEVIFY_STATUS_FILE`; for every `--json` call, the envelope's shape. Never a
//! probability: a score moves between model versions and between two runs of one input, the
//! item does not. Each PASS line prints the chosen item's `p` so the margin stays visible.
//!
//! **The data.**
//!
//! - `sharkdp/hyperfine` at `f12f3d9` (1,018 commits, 28 remote branches), cloned once into
//!   `CARGO_TARGET_TMPDIR/e2e-hyperfine-f12f3d9` and reused, with the golds of `scripts/ergonomics/tasks.jsonl` (the
//!   case ids are the task ids). A second clone carries task `ad1`'s two unrelated edits for
//!   `add`. The clone is full, not blobless: `commit` evidence runs `git show` per finalist,
//!   which a blobless clone answers over the network inside the lister's deadline.
//! - `evals/why/*.log`, failing CI jobs of public projects (the first two per ecosystem) with
//!   the hand-labelled root-cause range of each `.expect`, and two passing CI jobs of
//!   `evals/validation`, where `why` must abstain.
//! - the repository of `evals/commit-subjects/make_repo.sh`, whose commit subjects lie.
//! - the files `scripts/ergonomics/fixtures.py` writes, `docs/demo/`, `evals/live/` (issue
//!   titles, a recorded `gh run list`) and this repository's own history.
//!
//! ```sh
//! TYPESAFE_API_KEY_FILE=/path/to/key cargo test --test e2e -- --ignored --test-threads=1
//! JEVIFY_E2E_KEYLESS=1 cargo test --test e2e -- --ignored --test-threads=1   # classifier.dev
//! JEVIFY_E2E_ONLY=pc1,why-go-01 ...                                            # some cases
//! ```
//!
//! Without a key the TypeSafe run prints SKIPPED and passes. The keyless run spends
//! classifier.dev's free per-IP budget, so it runs only under `JEVIFY_E2E_KEYLESS=1`. A hand-off
//! lists a skipped run as NOT RUN, never as passed. The suite needs the network, `git`, `bash`
//! and `python3`. Every case prints one PASS or FAIL line; the test fails at the end naming
//! every failed case, and a backend that answers exit 4 or 5 fails the case, since nothing was
//! proved.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const HYPERFINE: &str = "https://github.com/sharkdp/hyperfine";
const PIN: &str = "f12f3d9f86f3643b3b7deace5e160b1f0f44d2b7";
const RIPGREP_PIN: &str = "3fce3b5bb0236da2df6d99672afb8a719642eca7";
const BAT_PIN: &str = "4987f76709aae3a1c4db723c53874c9ddcb0c4fd";

/// Where a case runs.
#[derive(Clone, Copy)]
enum At {
    /// This repository.
    Repo,
    /// The pinned hyperfine clone.
    Hyperfine,
    Ripgrep,
    Bat,
    /// A second hyperfine clone with ergonomics task `ad1`'s two unrelated, unstaged edits.
    Edited,
    /// The repository whose commit subjects lie.
    Liars,
    /// The files `scripts/ergonomics/fixtures.py` writes.
    Data,
}

/// What the case pipes into jevify.
enum In {
    Empty,
    /// A file, relative to where the case runs.
    File(&'static str),
    Text(&'static str),
    /// The output of this command, run where the case runs.
    Listing(&'static [&'static str]),
    /// A JSON array file, one compact element per line, as `jq -c '.[]'` prints it.
    JsonLines(&'static str),
}

enum Expect {
    /// Exit 0 and these items chosen, in this order, each starting with its gold: `pick`'s
    /// matches, `fill`'s handles, the records `filter` kept, the hunks `add` selects, `is` yes.
    Chose(&'static [&'static str]),
    /// Exit 0 and `why`'s first cause inside the range of `evals/why/<id>.expect`.
    Cause(&'static str),
    /// Exit 0 and `why`'s first cause between the first line holding the first text and the
    /// next line holding the second, both included.
    CauseBetween(&'static str, &'static str),
    /// Exit 0 and the record starting with each text carries that label; `!x` is any label but x.
    Labels(&'static [(&'static str, &'static str)]),
    /// Exit 1: `is` answers no, `filter` keeps nothing.
    No,
    /// Exit 3 and nothing chosen; without `--json` nothing on stdout; for `fill`, nothing ran.
    Nothing,
    /// Exit 3, or exit 0 choosing an item other than this known false positive.
    Not(&'static str),
    /// `fill` ran the command with these handles, and the exit code is the command's own.
    Ran {
        code: i32,
        handles: &'static [&'static str],
        stdout: &'static str,
    },
    /// `evals/live/titles.tsv` labelled: at most one of 30 off its gold, at most 6 unsure (the
    /// held-out measurement is 29 of 30; one title reads as a bug or a feature).
    Titles,
    /// `evals/live/titles.tsv` filtered: all 7 crashes and hangs kept, every yes one of them.
    Crashes,
}

struct Case {
    id: &'static str,
    at: At,
    stdin: In,
    argv: &'static [&'static str],
    expect: Expect,
}

fn c(id: &'static str, at: At, stdin: In, argv: &'static [&'static str], expect: Expect) -> Case {
    Case {
        id,
        at,
        stdin,
        argv,
        expect,
    }
}

#[rustfmt::skip]
fn cases() -> Vec<Case> {
    use At::*;
    use Expect::*;
    use In::*;
    const WHY: &[&str] = &["why", "--json"];
    const FILES: In = Listing(&["git", "ls-files"]);
    const SCRIPTS: In = Listing(&["git", "ls-files", "scripts/*.py"]);
    const TITLES: In = Listing(&["cut", "-f3", "evals/live/titles.tsv"]);
    vec![
        // why: a failing CI job, the root cause a person would point to.
        c("why-cargo-01", Repo, File("evals/why/cargo-01.log"), WHY, Cause("cargo-01")),
        // KNOWN MISS, 2 of 3 runs on 2026-09-28: points at line 135, the `process didn't exit
        // successfully` summary, as at 0.9.3; the third run pointed at 121, the gold.
        c("why-cargo-02", Repo, File("evals/why/cargo-02.log"), WHY, Cause("cargo-02")),
        c("why-npm-01", Repo, File("evals/why/npm-01.log"), WHY, Cause("npm-01")),
        c("why-npm-02", Repo, File("evals/why/npm-02.log"), WHY, Cause("npm-02")),
        // KNOWN MISS: points at line 107, the progress line naming the warning, as at 0.9.3.
        c("why-pytest-01", Repo, File("evals/why/pytest-01.log"), WHY, Cause("pytest-01")),
        c("why-pytest-02", Repo, File("evals/why/pytest-02.log"), WHY, Cause("pytest-02")),
        c("why-go-01", Repo, File("evals/why/go-01.log"), WHY, Cause("go-01")),
        c("why-go-02", Repo, File("evals/why/go-02.log"), WHY, Cause("go-02")),
        c("why-docker-01", Repo, File("evals/why/docker-01.log"), WHY, Cause("docker-01")),
        c("why-docker-02", Repo, File("evals/why/docker-02.log"), WHY, Cause("docker-02")),
        c("wy1", Data, File("build.log"), WHY, CauseBetween("error[E0382]", "value borrowed here after move")),
        c("wy4", Data, File("gotest.log"), WHY, CauseBetween("--- FAIL: TestGetOrder (0.00s)", "handler.go:42")),
        // why: a passing CI job holds no failure.
        c("why-clean-prometheus", Repo, File("evals/validation/inputs/why/ok-prometheus-prometheus-33830612264.log"), WHY, Nothing),
        c("why-clean-tokio", Repo, File("evals/validation/inputs/why/ok-tokio-rs-tokio-32519536561.log"), &["why"], Nothing),

        // fill: the handle a command needs, from a description; --dry-run resolves, runs nothing.
        c("fill-bat-pr", Bat, Empty, &["fill", "--dry-run", "--json", "--", "gh", "pr", "checkout",
            "@{pr:keeps the grid aligned when a tab follows a multibyte character}"], Chose(&["4018"])),
        c("fill-ripgrep-commit", Ripgrep, Empty, &["fill", "--dry-run", "--json", "--", "git", "revert",
            "@{commit:stops idle search workers spinning forever when a visitor unwinds}"], Chose(&["0d7054d"])),
        // The certbot file maps INI syntax; it does not renew certificates.
        c("pick-certbot-negative", Bat, Empty, &["pick", "--json", "--from", "file",
            "renews TLS certificates for an HTTPS server"], Not("src/syntax_mapping/builtins/unix-family/50-certbot.toml")),
        c("fl1", Hyperfine, Empty, &["fill", "--dry-run", "--json", "--", "git", "show", "--stat",
            "@{commit:the commit that repaired how per-command names were applied when benchmarking over a range of parameter values}"], Chose(&["835fc43"])),
        c("fl2", Hyperfine, Empty, &["fill", "--dry-run", "--json", "--", "head", "-n", "10",
            "@{file:the source file that writes the results table in the markup language used by Asciidoctor}"], Chose(&["src/export/asciidoc.rs"])),
        c("fl3", Hyperfine, Empty, &["fill", "--dry-run", "--json", "--", "ls",
            "@{dir:the directory that holds the platform-specific code measuring CPU and wall-clock time}"], Chose(&["src/timer"])),
        // A branch that exists only on the remote gets the spelling `git log` can read.
        c("fill-branch", Hyperfine, Empty, &["fill", "--dry-run", "--json", "--", "git", "log", "-1",
            "@{branch:the work on recording how much memory commands use}"], Chose(&["origin/track-memory-usage"])),
        // fill runs the command: its output and its exit code are its own.
        c("fill-exec", Hyperfine, Empty, &["fill", "--", "git", "show", "--stat",
            "@{commit:the commit that lets the user give their own label to the baseline command that the other commands are compared against}"],
            Ran { code: 0, handles: &["2166c9f"], stdout: "commit 2166c9f02e43c5bb74171abf441dce0aae4b0912" }),
        c("fill-exec-exit", Hyperfine, Empty, &["fill", "--", "git", "merge-base", "--is-ancestor", "HEAD",
            "@{commit:the commit that started collecting how much memory the benchmarked commands use}"],
            Ran { code: 1, handles: &["6556b2b"], stdout: "" }),
        c("fill-nothing", Hyperfine, Empty, &["fill", "--", "git", "show",
            "@{commit:the commit that ports the user interface to Android}"], Nothing),
        c("fill-own-commit", Repo, Empty, &["fill", "--dry-run", "--json", "--", "git", "show",
            "@{commit:renamed the project from hunch to grevi}"], Chose(&["441703a"])),
        c("fill-tool", Repo, Empty, &["fill", "--dry-run", "--json", "--", "env", "@{tool:the Rust package manager}", "--version"],
            Chose(&["cargo"])),
        c("fill-one-flag", Repo, Text("Segmentation fault after upgrading to macOS 26\n"), &["fill", "--dry-run", "--json", "--", "echo",
            "--label=@{one:bug|feature|docs:what kind of report is this}", "@{flag:--urgent:the report describes a crash or data loss}"],
            Chose(&["bug", "--urgent"])),
        c("fill-ci-run", Repo, JsonLines("evals/live/runs.json"), &["fill", "--dry-run", "--json", "--key", "databaseId", "--",
            "gh", "run", "view", "@{-:the failed ci run on the v0.7.0 tag}"], Chose(&["35736209634"])),

        // pick --from: the handle alone, from the kind's own listing.
        c("pc1", Hyperfine, Empty, &["pick", "--json", "--from", "commit",
            "the commit that lets the user give their own label to the baseline command that the other commands are compared against"], Chose(&["2166c9f"])),
        c("pc2", Hyperfine, Empty, &["pick", "--json", "--from", "commit",
            "the commit that makes the option for tolerating failing commands accept several specific exit statuses instead of all of them"], Chose(&["b5a6860"])),
        c("pc3", Hyperfine, Empty, &["pick", "--json", "--from", "commit",
            "the commit that stopped the tests relying on the Unix file-printing utility from running on Windows"], Chose(&["3bd38f2"])),
        c("pc4", Hyperfine, Empty, &["pick", "--json", "--from", "commit",
            "the commit that started collecting how much memory the benchmarked commands use"], Chose(&["6556b2b"])),
        c("pk1", Hyperfine, Empty, &["pick", "--json", "--from", "branch",
            "the branch with the work on recording how much memory commands use"], Chose(&["track-memory-usage"])),
        c("pick-from-file", Hyperfine, Empty, &["pick", "--json", "--from", "file",
            "the file that exports the results as a Markdown table"], Chose(&["src/export/markdown.rs"])),
        c("pick-from-dir", Hyperfine, Empty, &["pick", "--json", "--from", "dir",
            "the directory with the code that exports results to other file formats"], Chose(&["src/export"])),
        c("pick-from-nothing", Hyperfine, Empty, &["pick", "--from", "branch", "the branch that ports the user interface to Android"], Nothing),
        // The subject of the target says nothing; another commit's subject claims the change.
        // KNOWN MISS (both): one window of names decides alone, with no evidence round, so the
        // lying subject wins; `fill` on the same marker runs the evidence round and is right.
        c("liar-01", Liars, Empty, &["pick", "--json", "--from", "commit", "the commit that changed the gateway timeout value in the code"],
            Chose(&["99c813c"])),
        c("liar-07", Liars, Empty, &["pick", "--json", "--from", "commit", "the commit that made the log function print one JSON object per line"],
            Chose(&["f413041"])),

        // pick: one record of stdin, or nothing.
        c("pk2", Hyperfine, File("Cargo.toml"), &["pick", "--json", "the dependency that draws the progress bars in the terminal"],
            Chose(&["indicatif"])),
        c("pick-downloads", Repo, File("docs/demo/downloads.txt"), &["pick", "--json", "the electricity bill from July"],
            Chose(&["con_edison_electric_bill_july.pdf"])),
        c("pick-nothing", Repo, File("docs/demo/downloads.txt"), &["pick", "a scan of my passport"], Nothing),

        // pick --files: the path, judged on the file's first lines.
        c("pf1", Hyperfine, FILES, &["pick", "--json", "--files",
            "the file that flags measurements that sit far from the rest, using a score based on the median"], Chose(&["src/outlier_detection.rs"])),
        c("pf3", Hyperfine, FILES, &["pick", "--json", "--files",
            "the script that draws a box-and-whisker chart comparing several benchmark runs"], Chose(&["scripts/plot_whisker.py"])),
        c("pf4", Hyperfine, FILES, &["pick", "--json", "--files",
            "the file that splits a comma-separated list of parameter values while honouring backslash escapes"], Chose(&["src/parameter/tokenize.rs"])),
        c("pick-files-nothing", Hyperfine, FILES, &["pick", "--files", "the file that compiles GPU shaders"], Nothing),

        // filter: the records where the statement holds, in input order.
        c("ft3", Data, File("jobs.log"), &["filter", "--json", "--strict", "the job failed because something took too long"],
            Chose(&["[worker-4] job 1009 ", "[worker-1] job 1021 ", "[worker-3] job 1033 "])),
        c("ft4", Data, File("tickets.txt"), &["filter", "--json", "--strict", "asks for money to be returned"],
            Chose(&["#103 ", "#106 ", "#110 "])),
        c("ft2", Hyperfine, SCRIPTS, &["filter", "--json", "--files", "--strict", "produces a chart or plot"],
            Chose(&["scripts/plot_benchmark_comparison.py", "scripts/plot_histogram.py", "scripts/plot_parametrized.py",
                    "scripts/plot_progression.py", "scripts/plot_whisker.py"])),
        c("filter-none", Data, File("tickets.txt"), &["filter", "--json", "--strict", "reports a security vulnerability"], No),
        c("filter-crashes", Repo, TITLES, &["filter", "--json", "reports a crash or a hang"], Crashes),

        // label: one tag per record.
        c("lb1", Data, File("tickets.txt"), &["label", "--json", "bug,feature,question"], Labels(&[
            ("#101 ", "bug"), ("#105 ", "bug"), ("#109 ", "bug"), ("#102 ", "feature"), ("#107 ", "feature"),
            ("#111 ", "feature"), ("#104 ", "question"), ("#108 ", "question"), ("#112 ", "question")])),
        c("lb4", Data, File("events.log"), &["label", "--json", "outage,degraded,normal"], Labels(&[
            ("09:14:03", "outage"), ("09:20:44", "outage"), ("09:40:02", "outage"), ("09:00:01", "!outage"),
            ("09:05:12", "!outage"), ("09:11:40", "!outage"), ("09:15:30", "!outage"), ("09:22:10", "!outage"),
            ("09:30:00", "!outage"), ("09:31:17", "!outage"), ("09:45:55", "!outage"), ("09:50:21", "!outage")])),
        c("lb3", Hyperfine, SCRIPTS, &["label", "--json", "--files", "plotting,statistics"], Labels(&[
            ("scripts/advanced_statistics.py", "statistics"), ("scripts/welch_ttest.py", "statistics"),
            ("scripts/plot_benchmark_comparison.py", "plotting"), ("scripts/plot_histogram.py", "plotting"),
            ("scripts/plot_parametrized.py", "plotting"), ("scripts/plot_progression.py", "plotting"),
            ("scripts/plot_whisker.py", "plotting")])),
        c("label-titles", Repo, TITLES, &["label", "--json", "bug,feature,docs,question"], Titles),

        // is: a fact about one context, yes, no or unsure.
        c("is1", Data, Empty, &["is", "--json", "--context", "mail_cancel.txt", "the customer wants to end their subscription"],
            Chose(&["yes"])),
        c("is2", Data, Empty, &["is", "--json", "--context", "mail_stay.txt", "the customer wants to end their subscription"], No),
        c("is3", Hyperfine, Empty, &["is", "--json", "--context", "README.md", "results can be exported as Markdown"], Chose(&["yes"])),
        c("is4", Hyperfine, Empty, &["is", "--json", "--context", "LICENSE-MIT", "the license forbids commercial use of the software"], No),
        // Evidence past the budget: no inference and no verdict on a whole input nobody read.
        c("is-oversized", Hyperfine, Listing(&["git", "log", "--stat"]), &["is", "--json", "a commit in this history adds Windows support"],
            Nothing),

        // add --dry-run: the unstaged hunks about a topic, nothing staged.
        c("ad1", Edited, Empty, &["add", "--json", "--dry-run", "the installation instructions"], Chose(&["README.md"])),
        c("add-nothing", Edited, Empty, &["add", "--json", "--dry-run", "the database migration scripts"], Nothing),
    ]
}

/// The corpora, prepared once per run and reused across runs.
struct Ctx {
    tmp: PathBuf,
    repo: PathBuf,
    hyperfine: PathBuf,
    ripgrep: PathBuf,
    bat: PathBuf,
    edited: PathBuf,
    liars: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    config: PathBuf,
}

/// `git -C dir args`, its stdout, or its stderr as the error.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "git {args:?} in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

/// Runs a setup command and panics with its stderr when it fails.
fn setup(command: &mut Command) {
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn pinned_clone(tmp: &Path, url: &str, name: &str, pin: &str) -> PathBuf {
    let dir = tmp.join(name);
    if !dir.exists() {
        git(tmp, &["clone", "--quiet", "--revision", pin, url, name]).unwrap();
    }
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]).unwrap().trim(), pin);
    dir
}

impl Ctx {
    fn prepare() -> Self {
        let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let hyperfine = pinned_clone(&tmp, HYPERFINE, "e2e-hyperfine-f12f3d9", PIN);
        let ripgrep = pinned_clone(
            &tmp,
            "https://github.com/BurntSushi/ripgrep",
            "e2e-ripgrep-3fce3b5",
            RIPGREP_PIN,
        );
        let bat = pinned_clone(
            &tmp,
            "https://github.com/sharkdp/bat",
            "e2e-bat-4987f76",
            BAT_PIN,
        );
        let filter = ["config", "--get", "remote.origin.partialclonefilter"];
        assert!(
            git(&hyperfine, &filter).is_err(),
            "{} is a partial clone: commit evidence would fetch over the network",
            hyperfine.display()
        );
        // The branch golds name a remote branch, which a reused clone may lack.
        let branch = "refs/remotes/origin/track-memory-usage";
        if git(&hyperfine, &["rev-parse", "--verify", "--quiet", branch]).is_err() {
            let refspec = "+refs/heads/*:refs/remotes/origin/*";
            git(&hyperfine, &["fetch", "--quiet", "origin", refspec]).unwrap();
        }

        let edited = pinned_clone(
            &tmp,
            hyperfine.to_str().unwrap(),
            "e2e-hyperfine-f12f3d9-edited",
            PIN,
        );
        // Task ad1's edits, written over the pinned content on every run: the installation
        // instructions, and an unrelated constant.
        let pinned = |path: &str| git(&edited, &["show", &format!("{PIN}:{path}")]).unwrap();
        let readme = pinned("README.md");
        assert!(readme.contains("brew install hyperfine"));
        let readme = readme.replacen(
            "brew install hyperfine",
            "brew install hyperfine --formula",
            1,
        );
        std::fs::write(edited.join("README.md"), readme).unwrap();
        let units = pinned("src/util/units.rs")
            + "\n/// Shown when a shell takes too long to spawn.\n\
               pub const DEFAULT_SHELL_TIMEOUT_HINT: &str = \"the shell took too long to start\";\n";
        std::fs::write(edited.join("src/util/units.rs"), units).unwrap();

        let liars = tmp.join("e2e-liars");
        if !liars.join(".git").is_dir() {
            let script = repo.join("evals/commit-subjects/make_repo.sh");
            setup(Command::new("bash").arg(script).arg(&liars));
        }
        git(&liars, &["rev-parse", "--verify", "99c813c^{commit}"])
            .expect("make_repo.sh no longer reproduces the shas of cases-scratch.jsonl");

        let data = tmp.join("e2e-data");
        let fixtures = repo.join("scripts/ergonomics/fixtures.py");
        setup(Command::new("python3").arg(fixtures).arg(&data));

        let scratch = |prefix: &str| {
            tempfile::Builder::new()
                .prefix(prefix)
                .tempdir_in(&tmp)
                .unwrap()
                .keep()
        };
        Self {
            cache: scratch("e2e-cache-"),
            config: scratch("e2e-config-"),
            tmp,
            repo,
            hyperfine,
            ripgrep,
            bat,
            edited,
            liars,
            data,
        }
    }

    fn dir(&self, at: At) -> &Path {
        match at {
            At::Repo => &self.repo,
            At::Hyperfine => &self.hyperfine,
            At::Ripgrep => &self.ripgrep,
            At::Bat => &self.bat,
            At::Edited => &self.edited,
            At::Liars => &self.liars,
            At::Data => &self.data,
        }
    }

    fn run(&self, case: &Case, backend: &str) -> Run {
        let dir = self.dir(case.at);
        let input = match case.stdin {
            In::Empty => String::new(),
            In::Text(text) => text.to_owned(),
            In::File(path) => std::fs::read_to_string(dir.join(path)).unwrap(),
            In::Listing(argv) => {
                let out = Command::new(argv[0])
                    .args(&argv[1..])
                    .current_dir(dir)
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{argv:?}");
                String::from_utf8(out.stdout).unwrap()
            }
            In::JsonLines(path) => {
                let text = std::fs::read_to_string(dir.join(path)).unwrap();
                let items: Vec<Value> = serde_json::from_str(&text).unwrap();
                items.iter().map(|item| format!("{item}\n")).collect()
            }
        };
        // Emptied, never removed: a status file an earlier run left must not pass for this one.
        let status_file = self.tmp.join(format!("e2e-status-{}.json", case.id));
        std::fs::write(&status_file, "").unwrap();
        let mut cmd = assert_cmd::Command::cargo_bin("jevify").unwrap();
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("JEVIFY_") {
                cmd.env_remove(key);
            }
        }
        cmd.current_dir(dir)
            .env("JEVIFY_BACKEND", backend)
            .env("JEVIFY_NO_CACHE", "1")
            .env("JEVIFY_CACHE_DIR", &self.cache)
            .env("JEVIFY_CONFIG_DIR", &self.config)
            .env("JEVIFY_STATUS_FILE", &status_file)
            .timeout(Duration::from_secs(300));
        if backend == "classifier" {
            cmd.env_remove("TYPESAFE_API_KEY")
                .env_remove("TYPESAFE_API_KEY_FILE");
        }
        let out = cmd
            .args(case.argv)
            .write_stdin(input.clone())
            .output()
            .unwrap();
        let status = std::fs::read_to_string(&status_file).unwrap();
        Run {
            code: out.status.code(),
            envelope: serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            status: serde_json::from_str(&status).unwrap_or(Value::Null),
            input,
        }
    }
}

struct Run {
    code: Option<i32>,
    /// The `--json` envelope, or null.
    envelope: Value,
    stdout: String,
    stderr: String,
    /// What `fill` wrote to `JEVIFY_STATUS_FILE`, or null.
    status: Value,
    input: String,
}

/// The items a run chose, in output order, with their scores.
fn chosen(verb: &str, run: &Run) -> Result<Vec<(String, Option<f64>)>, String> {
    // `fill` without --dry-run prints no envelope: its markers come from the status file.
    let data = if run.envelope.is_null() {
        &run.status
    } else {
        &run.envelope["data"]
    };
    let list = |key: &str, field: &str| -> Vec<(String, Option<f64>)> {
        data[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let item = match &r[field] {
                    Value::String(s) => s.trim_end().to_owned(),
                    Value::Number(n) => n.to_string(),
                    _ => return None,
                };
                Some((item, r["p"].as_f64()))
            })
            .collect()
    };
    Ok(match verb {
        "why" => list("causes", "line"),
        "pick" => list("matches", "text"),
        "fill" => list("markers", "handle"),
        "filter" => list("records", "text"),
        "label" => list("records", "label"),
        // --dry-run stages nothing; the hunks at or above the threshold are the ones --yes stages.
        "add" => {
            let threshold = run.envelope["meta"]["threshold"].as_f64().unwrap_or(1.0);
            let mut hunks = list("hunks", "file");
            hunks.retain(|(_, p)| p.is_some_and(|p| p >= threshold));
            hunks
        }
        "is" => match data["verdict"].as_str() {
            Some(verdict @ ("yes" | "no")) => vec![(verdict.to_owned(), data["p"].as_f64())],
            _ => Vec::new(),
        },
        other => return Err(format!("no rule for what `{other}` chooses")),
    })
}

/// The 1-based, inclusive gold range of a `why` case: its `.expect` file, or the lines holding
/// the two texts.
fn cause_range(expect: &Expect, input: &str, repo: &Path) -> Result<(usize, usize), String> {
    match expect {
        Expect::Cause(id) => {
            let path = repo.join(format!("evals/why/{id}.expect"));
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{path:?}: {e}"))?;
            let gold: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let line = |i: usize| gold["lines"][i].as_u64().map(|n| n as usize);
            line(0)
                .zip(line(1))
                .ok_or_else(|| format!("{path:?} holds no `lines`"))
        }
        Expect::CauseBetween(from, to) => {
            let lo = line_of(input, 0, from)?;
            Ok((lo, line_of(input, lo - 1, to)?))
        }
        _ => Err("not a `why` expectation".into()),
    }
}

/// (gold label, crash or hang, title) per line of `evals/live/titles.tsv`.
fn titles(repo: &Path) -> Vec<(String, bool, String)> {
    std::fs::read_to_string(repo.join("evals/live/titles.tsv"))
        .unwrap()
        .lines()
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            let label = fields.next().unwrap().to_owned();
            let crash = fields.next().unwrap() == "1";
            (label, crash, fields.next().unwrap().to_owned())
        })
        .collect()
}

/// The 1-based line number of the first line from `from` (0-based) holding `text`.
fn line_of(input: &str, from: usize, text: &str) -> Result<usize, String> {
    input
        .lines()
        .enumerate()
        .skip(from)
        .find(|(_, line)| line.contains(text))
        .map(|(i, _)| i + 1)
        .ok_or_else(|| format!("no line holds `{text}`"))
}

fn check(case: &Case, run: &Run, backend: &str, ctx: &Ctx) -> Result<String, String> {
    let verb = case.argv[0];
    let json = case.argv.contains(&"--json");
    if matches!(run.code, Some(4 | 5)) {
        return Err(format!(
            "backend unavailable, nothing proved: {}",
            run.stderr.trim()
        ));
    }
    let v = &run.envelope;
    if json {
        for key in [
            "ok",
            "command",
            "version",
            "exit_code",
            "data",
            "meta",
            "error",
        ] {
            if v.get(key).is_none() {
                return Err(format!("no `{key}` in the envelope; stderr {}", run.stderr));
            }
        }
        let shape = v["command"] == verb
            && v["exit_code"].as_i64() == run.code.map(i64::from)
            && v["ok"] == v["error"].is_null()
            && v["version"].is_string()
            && v["meta"]["backend"] == backend;
        if !shape {
            return Err(format!("envelope out of contract: {v}"));
        }
    }
    let exec = matches!(case.expect, Expect::Ran { .. });
    if verb == "fill" {
        // Written before anything starts, for every outcome fill decides.
        let s = &run.status;
        let decided = if exec { Some(0) } else { run.code };
        if s["ran"] != exec || s["exit_code"].as_i64() != decided.map(i64::from) {
            return Err(format!("status file says {s}, want ran={exec}"));
        }
    }
    let want = match case.expect {
        Expect::No => 1,
        Expect::Nothing => 3,
        Expect::Not(_) if run.code == Some(3) => 3,
        Expect::Ran { code, .. } => code,
        _ => 0,
    };
    let data = &v["data"];
    let tail = || {
        format!(
            "{}{}",
            run.stderr.trim(),
            if json {
                format!("\n{data}")
            } else {
                String::new()
            }
        )
    };
    if run.code != Some(want) {
        return Err(format!("exit {:?}, want {want}: {}", run.code, tail()));
    }
    let got = chosen(verb, run)?;
    let names: Vec<&str> = got.iter().map(|(item, _)| item.as_str()).collect();
    let same = |want: &[&str]| {
        names.len() == want.len() && names.iter().zip(want).all(|(g, w)| g.starts_with(w))
    };
    match &case.expect {
        Expect::Not(forbidden) => {
            if (want == 3 && !names.is_empty())
                || (want == 0 && (names.len() != 1 || names.contains(forbidden)))
            {
                return Err(format!(
                    "chose {names:?}, must abstain or avoid {forbidden}: {}",
                    tail()
                ));
            }
        }
        Expect::Chose(want) if !same(want) => {
            return Err(format!("chose {names:?}, want {want:?}: {}", tail()));
        }
        Expect::Cause(_) | Expect::CauseBetween(..) => {
            let (lo, hi) = cause_range(&case.expect, &run.input, &ctx.repo)?;
            let first = names.first().and_then(|n| n.parse::<usize>().ok());
            if !first.is_some_and(|n| (lo..=hi).contains(&n)) {
                return Err(format!(
                    "first cause at line {first:?}, gold {lo}-{hi}: {}",
                    tail()
                ));
            }
        }
        Expect::Labels(want) => {
            let records = data["records"].as_array().cloned().unwrap_or_default();
            for (prefix, label) in *want {
                let record = records
                    .iter()
                    .find(|r| r["text"].as_str().is_some_and(|t| t.starts_with(prefix)))
                    .ok_or_else(|| format!("no record starts with `{prefix}`: {data}"))?;
                let got = record["label"].as_str().unwrap_or_default();
                let right = match label.strip_prefix('!') {
                    Some(not) => got != not,
                    None => got == *label,
                };
                if !right {
                    return Err(format!(
                        "`{prefix}` labelled `{got}`, want `{label}`: {data}"
                    ));
                }
            }
        }
        Expect::No if names.iter().any(|n| *n != "no") => {
            return Err(format!("chose {names:?} on exit 1: {}", tail()));
        }
        Expect::Nothing if !names.is_empty() || (!json && !run.stdout.is_empty()) => {
            return Err(format!(
                "nothing fits, yet chose {names:?}, stdout {:?}: {}",
                run.stdout,
                tail()
            ));
        }
        Expect::Ran {
            handles, stdout, ..
        } if !same(handles) || !run.stdout.contains(stdout) => {
            return Err(format!(
                "ran {names:?}, want {handles:?} and stdout holding {stdout:?}: {}; stdout {}",
                run.status, run.stdout
            ));
        }
        Expect::Titles => {
            let rows = titles(&ctx.repo);
            let records = data["records"].as_array().cloned().unwrap_or_default();
            if records.len() != rows.len() {
                return Err(format!(
                    "{} records, want {}: {data}",
                    records.len(),
                    rows.len()
                ));
            }
            let (mut unsure, mut wrong) = (0, Vec::new());
            for (record, (gold, _, title)) in records.iter().zip(&rows) {
                match record["label"].as_str().unwrap_or_default() {
                    "?" => unsure += 1,
                    label if label == gold.as_str() => {}
                    label => wrong.push(format!("`{title}` {label}, gold {gold}")),
                }
            }
            if wrong.len() > 1 || unsure > 6 {
                return Err(format!("{unsure} unsure, wrong {wrong:?}"));
            }
            return Ok(format!(
                "{} of 30 right, {unsure} unsure, wrong {wrong:?}",
                30 - unsure - wrong.len()
            ));
        }
        Expect::Crashes => {
            let rows = titles(&ctx.repo);
            let crashes: Vec<&str> = rows.iter().filter(|r| r.1).map(|r| r.2.as_str()).collect();
            let records = data["records"].as_array().cloned().unwrap_or_default();
            let kept: Vec<(&str, &str)> = records
                .iter()
                .filter_map(|r| Some((r["text"].as_str()?.trim_end(), r["verdict"].as_str()?)))
                .collect();
            if let Some((title, _)) = kept
                .iter()
                .find(|(t, v)| *v == "yes" && !crashes.contains(t))
            {
                return Err(format!("kept `{title}` as a crash or a hang: {data}"));
            }
            if let Some(lost) = crashes.iter().find(|c| !kept.iter().any(|(t, _)| t == *c)) {
                return Err(format!("lost `{lost}`: {data}"));
            }
            return Ok(format!("kept {} of 30, all 7 crashes", kept.len()));
        }
        _ => {}
    }
    let shown: Vec<String> = got
        .iter()
        .take(5)
        .map(|(item, p)| match p {
            Some(p) => format!("{item:.40} ({p:.2})"),
            None => format!("{item:.40}"),
        })
        .collect();
    Ok(format!("exit {want} {shown:?}"))
}

fn run_suite(backend: &str) {
    let started = Instant::now();
    let ctx = Ctx::prepare();
    let only = std::env::var("JEVIFY_E2E_ONLY").ok();
    let (mut passed, mut skipped, mut requests) = (0, 0, 0);
    let mut failed = Vec::new();
    for case in cases() {
        if only
            .as_deref()
            .is_some_and(|o| !o.split(',').any(|id| id == case.id))
        {
            continue;
        }
        let t = Instant::now();
        if case.id == "fill-bat-pr"
            && !Command::new("gh")
                .args(["auth", "status", "--hostname", "github.com"])
                .output()
                .is_ok_and(|out| out.status.success())
        {
            eprintln!("SKIPPED {}: gh is unauthenticated", case.id);
            skipped += 1;
            continue;
        }
        let run = ctx.run(&case, backend);
        let asked = run.envelope["meta"]["requests"].as_u64().unwrap_or(0);
        requests += asked;
        // classifier.dev chooses its model: an answer from another model says nothing about
        // jevify. TypeSafe must answer with Jev. A call that asked nothing has no model.
        let model = match run.envelope["meta"]["model"].as_str() {
            Some(model) if asked > 0 => model,
            _ => "jev",
        };
        if backend == "classifier" && !jevify::jev::all_jev(model) {
            eprintln!(
                "SKIPPED {}: classifier.dev answered with `{model}`",
                case.id
            );
            skipped += 1;
            continue;
        }
        let result = if jevify::jev::all_jev(model) {
            check(&case, &run, backend, &ctx)
        } else {
            Err(format!("answered by `{model}`, not Jev"))
        };
        let secs = t.elapsed().as_secs_f64();
        match result {
            Ok(note) => {
                passed += 1;
                eprintln!("PASS {:<22} {secs:>5.1}s  {note}", case.id);
            }
            Err(why) => {
                eprintln!("FAIL {:<22} {secs:>5.1}s  {why}", case.id);
                failed.push(case.id);
            }
        }
    }
    eprintln!(
        "e2e on {backend}: {passed} passed, {} failed {failed:?}, {skipped} skipped; \
         {requests} requests in the envelopes; {:.0} s",
        failed.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(failed.is_empty(), "failed on {backend}: {failed:?}");
}

#[test]
#[ignore]
fn end_to_end_on_typesafe() {
    if std::env::var_os("TYPESAFE_API_KEY").is_none()
        && std::env::var_os("TYPESAFE_API_KEY_FILE").is_none()
    {
        eprintln!(
            "SKIPPED: set TYPESAFE_API_KEY_FILE=/path/to/key (or TYPESAFE_API_KEY) to run the end-to-end suite"
        );
        return;
    }
    run_suite("typesafe");
}

#[test]
#[ignore]
fn end_to_end_on_classifier() {
    if std::env::var_os("JEVIFY_E2E_KEYLESS").is_none() {
        eprintln!(
            "SKIPPED: set JEVIFY_E2E_KEYLESS=1 to spend classifier.dev's free per-IP budget on the end-to-end suite"
        );
        return;
    }
    run_suite("classifier");
}
