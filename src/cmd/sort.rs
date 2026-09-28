use crate::cmd::Outcome;
use crate::config::Config;
use crate::exit::{Exit, JevifyError};
use crate::jev::client::Client;
use crate::jev::{Question, Questions};
use rustix::fs::{Mode, OFlags, RenameFlags, openat, renameat_with};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const BATCH: usize = 10;

fn folders(root: &Path, skipped: &mut Vec<serde_json::Value>) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(handle) = open_dir(&d) else {
            skipped.push(serde_json::json!({"file": d.to_string_lossy(), "reason": "destination unavailable or symlink parent"}));
            continue;
        };
        let Ok(rd) = rustix::fs::Dir::read_from(&handle) else {
            continue;
        };
        for e in rd.flatten() {
            let name = std::ffi::OsStr::from_bytes(e.file_name().to_bytes());
            let p = d.join(name);
            let hidden = name.as_bytes().starts_with(b".");
            if e.file_type() == rustix::fs::FileType::Symlink {
                skipped.push(serde_json::json!({"file": p.to_string_lossy(), "reason": "symlink destination ignored"}));
            } else if e.file_type() == rustix::fs::FileType::Directory && !hidden {
                out.push(p.clone());
                if depth < 1 {
                    stack.push((p, depth + 1));
                }
            }
        }
    }
    out.sort();
    out.truncate(200);
    out
}

/// `p` must be absolute and normalized (`open_regular`). A file that cannot be opened or read
/// is described by its name alone.
pub(crate) fn excerpt(p: &Path) -> String {
    read_excerpt(p).unwrap_or_else(|_| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

/// The excerpt of a regular file, or the error that kept its bytes from being read (a denied
/// directory on the way, a missing file, a directory in its place). Binary content is not an
/// error: the name alone describes it. Shared with `--files` on the record verbs.
pub(crate) fn read_excerpt(p: &Path) -> io::Result<String> {
    use std::io::Read;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Read at most 8 KiB: `read_to_string` would load a multi-GB video before failing UTF-8.
    let mut head = Vec::new();
    open_regular(p)?.take(8192).read_to_end(&mut head)?;
    Ok(excerpt_text(p, name, &head))
}

fn excerpt_text(p: &Path, name: String, head: &[u8]) -> String {
    let text = match std::str::from_utf8(head) {
        Ok(s) => Some(s),
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&head[..e.valid_up_to()]).ok(),
        Err(_) => None,
    };
    if let Some(s) = text.filter(|s| !s.contains('\0')) {
        return format!(
            "{name}: {}",
            crate::input::redact(&s.chars().take(2000).collect::<String>())
        );
    }
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        let pdf = pdf_text(p);
        if let Some(text) = pdf {
            return format!("{name}: {}", crate::input::redact(&text));
        }
    }
    name
}

/// One `pdftotext` render of the first two pages; a hung or slow converter yields no text and
/// the file is described by its name alone.
const PDF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Two pages of text are a few kilobytes; the excerpt keeps 2,000 characters of it.
const PDF_OUTPUT_CAP: usize = 64 * 1024;

fn pdf_text(path: &Path) -> Option<String> {
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    pdf_text_within(path, &path_env, std::time::Instant::now() + PDF_TIMEOUT)
}

/// The text of the PDF at `path` from the `pdftotext` on `path_env`, killed at `deadline` (no
/// text then), stdin at `/dev/null`, output bounded. `path` is absolute and normalized, and
/// only a regular file that opens without following a link is handed to the converter.
fn pdf_text_within(
    path: &Path,
    path_env: &std::ffi::OsStr,
    deadline: std::time::Instant,
) -> Option<String> {
    open_regular(path).ok()?;
    let mut command = std::process::Command::new("pdftotext");
    command
        .args(["-l", "2"])
        .arg(path)
        .arg("-")
        .env("PATH", path_env);
    let output = crate::inventory::output_within(command, deadline, PDF_OUTPUT_CAP)?;
    let text: String = String::from_utf8_lossy(&output)
        .chars()
        .take(2000)
        .collect();
    (!text.trim().is_empty()).then_some(text)
}

// Walk every component without following links. Once opened, directory handles anchor
// reads/renames even if an ancestor path is replaced. Roots are canonicalized once.
fn open_dir(path: &Path) -> io::Result<File> {
    let mut dir = File::open("/")?;
    for component in path.components() {
        match component {
            std::path::Component::RootDir => (),
            std::path::Component::Normal(name) => {
                dir = openat(
                    &dir,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )?
                .into();
            }
            _ => return Err(io::Error::other("expected absolute normalized path")),
        }
    }
    Ok(dir)
}

fn open_regular(path: &Path) -> io::Result<File> {
    let parent = open_dir(
        path.parent()
            .ok_or_else(|| io::Error::other("missing parent"))?,
    )?;
    let file: File = openat(
        &parent,
        path.file_name()
            .ok_or_else(|| io::Error::other("missing name"))?,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into();
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    Ok(file)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Intent {
    from: Vec<u8>,
    to: Vec<u8>,
    dev: u64,
    ino: u64,
}

impl Intent {
    fn new(from: &Path, to: &Path) -> io::Result<Self> {
        let m = open_regular(from)?.metadata()?;
        Ok(Self {
            from: from.as_os_str().as_bytes().to_vec(),
            to: to.as_os_str().as_bytes().to_vec(),
            dev: m.dev(),
            ino: m.ino(),
        })
    }
    fn paths(&self) -> (PathBuf, PathBuf) {
        (
            std::ffi::OsString::from_vec(self.from.clone()).into(),
            std::ffi::OsString::from_vec(self.to.clone()).into(),
        )
    }
    fn matches(&self, path: &Path) -> bool {
        open_regular(path)
            .and_then(|f| f.metadata())
            .is_ok_and(|m| m.dev() == self.dev && m.ino() == self.ino)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Record {
    Intent { id: usize, file: Intent },
    Complete { id: usize },
}

fn record(log: &mut File, event: &Record) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(event)?;
    bytes.push(b'\n');
    log.write_all(&bytes)?;
    log.sync_all()
}

fn journal(dir: &Path) -> io::Result<(PathBuf, File)> {
    std::fs::create_dir_all(dir)?;
    let dir = dir.canonicalize()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for n in 0..1000 {
        let path = dir.join(format!(
            "sort-undo-{stamp}-{}-{n}.jsonl",
            std::process::id()
        ));
        match File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => {
                file.sync_all()?;
                open_dir(&dir)?.sync_all()?;
                return Ok((path, file));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("cannot create unique undo journal"))
}

fn move_file(from: &Path, to: &Path, identity: &Intent) -> io::Result<()> {
    let source_dir = open_dir(
        from.parent()
            .ok_or_else(|| io::Error::other("missing source parent"))?,
    )?;
    let target_dir = open_dir(
        to.parent()
            .ok_or_else(|| io::Error::other("missing target parent"))?,
    )?;
    let source_name = from
        .file_name()
        .ok_or_else(|| io::Error::other("missing source name"))?;
    let target_name = to
        .file_name()
        .ok_or_else(|| io::Error::other("missing target name"))?;
    let source: File = openat(
        &source_dir,
        source_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )?
    .into();
    let metadata = source.metadata()?;
    if !metadata.is_file() || metadata.dev() != identity.dev || metadata.ino() != identity.ino {
        return Err(io::Error::other("source identity changed"));
    }
    if target_dir.metadata()?.dev() != metadata.dev() {
        return Err(io::Error::other(
            "--into must be on the same volume as the files",
        ));
    }
    // Atomic target exclusion; never fall back to replacing rename. The source name
    // can still be swapped after this check: concurrent source writers are unsupported.
    renameat_with(
        &source_dir,
        source_name,
        &target_dir,
        target_name,
        RenameFlags::NOREPLACE,
    )?;
    source_dir.sync_all()?;
    target_dir.sync_all()?;
    Ok(())
}

fn partial_error(log: &Path, completed: usize, error: impl std::fmt::Display) -> JevifyError {
    JevifyError::Input(format!(
        "{error}; confirmed completed {completed} move(s); recovery journal: {}; undo reconciles any pending intent by file identity",
        log.display()
    ))
}

fn apply_moves(moves: &[Intent], log: &Path, writer: &mut File) -> Result<(), JevifyError> {
    for (id, intent) in moves.iter().enumerate() {
        let (from, to) = intent.paths();
        record(
            writer,
            &Record::Intent {
                id,
                file: intent.clone(),
            },
        )
        .map_err(|e| partial_error(log, id, e))?;
        move_file(&from, &to, intent).map_err(|e| partial_error(log, id, e))?;
        record(writer, &Record::Complete { id }).map_err(|e| partial_error(log, id + 1, e))?;
    }
    Ok(())
}

fn undo(log: &Path) -> Result<Outcome, JevifyError> {
    let text =
        std::fs::read_to_string(log).map_err(|e| JevifyError::Input(format!("undo log: {e}")))?;
    let (mut restored, mut skipped) = (vec![], vec![]);
    // Parse the complete journal before mutating. A torn trailing completion record
    // is recoverable from its durable intent and filesystem identity.
    let mut intents = Vec::new();
    for line in text.split_inclusive('\n') {
        let event: Record = match serde_json::from_str(line) {
            Ok(event) => event,
            Err(_) if !line.ends_with('\n') && !intents.is_empty() => break,
            Err(e) => return Err(partial_error(log, 0, format!("invalid journal: {e}"))),
        };
        if let Record::Intent { id, file } = event {
            if id != intents.len() {
                return Err(partial_error(log, 0, "invalid intent order"));
            }
            let (from, to) = file.paths();
            if !from.is_absolute() || !to.is_absolute() {
                return Err(partial_error(log, 0, "relative journal path"));
            }
            intents.push(file);
        }
    }
    for intent in intents.iter().rev() {
        let (from, to) = intent.paths();
        if std::fs::symlink_metadata(&from).is_ok() {
            skipped.push(serde_json::json!({ "file": to.to_string_lossy(), "reason": "original path occupied or intent not performed" }));
        } else if !intent.matches(&to) {
            skipped.push(serde_json::json!({ "file": to.to_string_lossy(), "reason": "destination missing or identity changed; not restored" }));
        } else {
            move_file(&to, &from, intent).map_err(|e| {
                partial_error(log, restored.len(), format!("undo move failed: {e}"))
            })?;
            restored.push(
                serde_json::json!({ "from": to.to_string_lossy(), "to": from.to_string_lossy() }),
            );
        }
    }
    Ok(Outcome {
        exit: if restored.is_empty() {
            Exit::Abstain
        } else {
            Exit::Ok
        },
        human: format!("restored {} file(s)\n", restored.len()).into_bytes(),
        exec: None,
        data: serde_json::json!({ "moves": restored, "skipped": skipped, "undo_log": null, "applied": true }),
    })
}

pub async fn run(
    ctx: &Config,
    dir: &Path,
    into: Option<&Path>,
    apply: bool,
    undo_log: Option<&Path>,
) -> Result<Outcome, JevifyError> {
    if let Some(l) = undo_log {
        return undo(l);
    }
    let dir = dir
        .canonicalize()
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    // A file is sorted alone, among the folders beside it (or under --into).
    let (dir, only) = match dir.parent() {
        Some(parent) if dir.is_file() => (parent.to_path_buf(), Some(dir.clone())),
        _ => (dir, None),
    };
    let root = into
        .unwrap_or(&dir)
        .canonicalize()
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    let mut skipped = vec![];
    let dests = folders(&root, &mut skipped);
    if dests.is_empty() {
        return Err(JevifyError::Input(format!(
            "no folders under {} to sort into",
            root.display()
        )));
    }
    if dests.len() > ctx.backend.window() {
        return Err(JevifyError::InputTooLarge(format!(
            "{} supports at most {} destination folders per sort; found {}",
            ctx.backend.as_str(),
            ctx.backend.window(),
            dests.len()
        )));
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| JevifyError::Input(e.to_string()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
                skipped.push(serde_json::json!({"file": p.to_string_lossy(), "reason": "symlink source ignored"}));
                return false;
            }
            open_regular(p).is_ok()
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    if let Some(file) = &only {
        files.retain(|p| p == file);
    }
    // Canonical order: `read_dir` is filesystem order, and option order moves an uncertain
    // probability (up to 0.23 measured); sorted, the batches and the cache key are stable.
    files.sort();
    if files.is_empty() {
        return Err(JevifyError::EmptyInput("no files to sort"));
    }
    let identities: BTreeMap<PathBuf, Intent> = files
        .iter()
        .map(|f| Intent::new(f, f).map(|identity| (f.clone(), identity)))
        .collect::<io::Result<_>>()
        .map_err(|e| JevifyError::Input(e.to_string()))?;
    let client = Client::new(ctx)?;
    let folder_items: Vec<String> = dests
        .iter()
        .enumerate()
        .map(|(i, d)| format!("[D{i:03}] {}", d.strip_prefix(&root).unwrap_or(d).display()))
        .collect();
    let mut crit: BTreeMap<String, Option<String>> = (0..dests.len())
        .map(|i| (format!("D{i:03}"), None))
        .collect();
    crit.insert(
        "NONE".into(),
        Some("no listed folder is a good home for this file".into()),
    );
    let jobs = files.chunks(BATCH).map(|chunk| {
        let state = serde_json::json!({ "files": chunk.iter().enumerate().map(|(k, f)| format!("[{k}] {}", excerpt(f))).collect::<Vec<_>>(), "folders": folder_items });
        let mut qs = Questions::new();
        for k in 0..chunk.len() {
            // As in `pick`: the Choice says which folder, the Noul says whether any folder fits at
            // all. Only the Noul is compared to the threshold (a Choice probability is relative to
            // its option set and does not share the Noul's calibration); the Choice must beat NONE.
            qs.insert(format!("f{k:02}"), Question::choice(format!("Which folder in `folders` is the right home for the file in `files[{k}]`? Choose NONE if none fits."), crit.clone()));
            qs.insert(format!("a{k:02}"), Question::noul_with(
                format!("Is one of the folders in `folders` the right home for the file in `files[{k}]`?"),
                "a listed folder is where this file belongs",
                "no listed folder is a good home for this file",
            ));
        }
        let client = &client;
        async move {
            let r = client.ask(&state, &qs).await?;
            (0..chunk.len()).map(|k| {
                let a = r.answers.get(&format!("f{k:02}")).cloned().unwrap_or_default();
                let c = a.choice.unwrap_or_else(|| "NONE".into());
                let probs = a.probabilities.unwrap_or_default();
                let p = probs.get(&c).copied().unwrap_or(0.0);
                let none = probs.get("NONE").copied().unwrap_or(0.0);
                Ok((c, p, none, r.noul(&format!("a{k:02}"))?))
            }).collect::<Result<Vec<_>, JevifyError>>()
        }
    });
    let picks: Vec<(String, f64, f64, f64)> = futures::future::try_join_all(jobs)
        .await?
        .into_iter()
        .flatten()
        .collect();
    let mut moves = vec![];
    for (f, (c, p, none, any)) in files.iter().zip(&picks) {
        ctx.stats.gate(crate::output::Gate {
            best: Some(*p),
            next: None,
            none: Some(*none),
            any: Some(*any),
            fails: None,
        });
        let Some(d) = c
            .strip_prefix('D')
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| dests.get(n))
        else {
            skipped.push(
                serde_json::json!({ "file": f.display().to_string(), "reason": "no folder fits" }),
            );
            continue;
        };
        let target = d.join(f.file_name().unwrap());
        if *any < ctx.threshold || *p <= *none {
            skipped.push(serde_json::json!({ "file": f.display().to_string(), "reason": format!("low confidence (any {any:.2}, folder {p:.2})") }));
        } else if std::fs::symlink_metadata(&target).is_ok() {
            skipped.push(
                serde_json::json!({ "file": f.display().to_string(), "reason": "target exists" }),
            );
        } else {
            moves.push((f.clone(), target, *p));
        }
    }
    let mut undo_path = None;
    if apply && !moves.is_empty() {
        let cache = ctx.cache_dir.clone().unwrap_or_else(std::env::temp_dir);
        let (log, mut writer) = journal(&cache).map_err(|e| {
            JevifyError::Input(format!("create recovery journal before moving: {e}"))
        })?;
        let intents: Vec<_> = moves
            .iter()
            .map(|(from, to, _)| {
                let mut intent = identities[from].clone();
                intent.to = to.as_os_str().as_bytes().to_vec();
                intent
            })
            .collect();
        apply_moves(&intents, &log, &mut writer)?;
        undo_path = Some(log.display().to_string());
    }
    let human: String = moves
        .iter()
        .map(|(f, t, p)| format!("{:.2}  {} → {}\n", p, f.display(), t.display()))
        .collect();
    Ok(Outcome {
        exit: if moves.is_empty() {
            Exit::Abstain
        } else {
            Exit::Ok
        },
        data: serde_json::json!({ "moves": moves.iter().map(|(f, t, p)| serde_json::json!({ "from": f.display().to_string(), "to": t.display().to_string(), "p": p })).collect::<Vec<_>>(), "skipped": skipped, "undo_log": undo_path, "applied": apply }),
        human: human.into_bytes(),
        exec: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undo_moves_back_only_when_the_original_path_is_free() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("Finance")).unwrap();
        let (a_from, a_to) = (root.join("a.txt"), root.join("Finance/a.txt"));
        let (b_from, b_to) = (root.join("b.txt"), root.join("Finance/b.txt"));
        std::fs::write(&a_to, "a").unwrap();
        std::fs::write(&b_to, "b").unwrap();
        std::fs::write(&b_from, "taken").unwrap();
        let (log, mut writer) = journal(&root).unwrap();
        for (id, (from, to)) in [(&a_from, &a_to), (&b_from, &b_to)].into_iter().enumerate() {
            let mut file = Intent::new(to, to).unwrap();
            file.from = from.as_os_str().as_bytes().to_vec();
            record(&mut writer, &Record::Intent { id, file }).unwrap();
            record(&mut writer, &Record::Complete { id }).unwrap();
        }
        let out = undo(&log).unwrap();
        assert_eq!(out.exit, Exit::Ok);
        assert!(a_from.exists() && !a_to.exists());
        assert!(b_to.exists(), "a taken original path is never overwritten");
        assert_eq!(std::fs::read_to_string(&b_from).unwrap(), "taken");
        assert_eq!(out.data["skipped"].as_array().unwrap().len(), 1);
        // Nothing left to restore: exit 3, as documented.
        assert_eq!(undo(&log).unwrap().exit, Exit::Abstain);
        assert_eq!(
            undo(Path::new("/nonexistent/undo.tsv"))
                .err()
                .unwrap()
                .exit(),
            Exit::Input
        );
    }

    #[test]
    fn atomic_move_rejects_regular_dangling_and_competing_targets() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        for (n, dangling) in [(0, false), (1, true), (2, false)] {
            let from = root.join(format!("from{n}"));
            let to = root.join(format!("to{n}"));
            std::fs::write(&from, "source").unwrap();
            let intent = Intent::new(&from, &to).unwrap();
            assert!(!to.exists());
            // Deterministic competing creation after preparation, before the syscall.
            if dangling {
                std::os::unix::fs::symlink("missing", &to).unwrap();
            } else {
                std::fs::write(&to, "occupied").unwrap();
            }
            assert!(move_file(&from, &to, &intent).is_err());
            assert_eq!(std::fs::read_to_string(&from).unwrap(), "source");
            if dangling {
                assert!(
                    std::fs::symlink_metadata(&to)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
            } else {
                assert_eq!(std::fs::read_to_string(&to).unwrap(), "occupied");
            }
        }
    }

    #[test]
    fn journals_are_unique_and_preserve_arbitrary_filename_bytes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let arbitrary = Intent {
            from: b"/tab\tline\ninvalid\xff".to_vec(),
            to: b"/to\xfe".to_vec(),
            dev: 1,
            ino: 2,
        };
        let decoded: Intent =
            serde_json::from_slice(&serde_json::to_vec(&arbitrary).unwrap()).unwrap();
        assert_eq!(decoded.paths().0.as_os_str().as_bytes(), arbitrary.from);
        assert_eq!(decoded.paths().1.as_os_str().as_bytes(), arbitrary.to);
        // APFS rejects invalid UTF-8 filenames; Linux exercises those bytes on disk.
        #[cfg(target_os = "linux")]
        let name = std::ffi::OsString::from_vec(b"tab\tline\ninvalid\xff".to_vec());
        #[cfg(not(target_os = "linux"))]
        let name = std::ffi::OsString::from("tab\tline\n");
        let from = root.join(&name);
        let target_dir = root.join("target");
        std::fs::create_dir(&target_dir).unwrap();
        let to = target_dir.join(name);
        std::fs::write(&from, "contents").unwrap();
        let (log, mut writer) = journal(&root).unwrap();
        let (other, _) = journal(&root).unwrap();
        assert_ne!(log, other);
        let intent = Intent::new(&from, &to).unwrap();
        apply_moves(&[intent], &log, &mut writer).unwrap();
        assert_eq!(undo(&log).unwrap().exit, Exit::Ok);
        assert_eq!(std::fs::read_to_string(from).unwrap(), "contents");
    }

    #[test]
    fn durable_intent_recovers_missing_or_torn_completion_but_not_failed_move() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let from = root.join("from");
        let to = root.join("to");
        std::fs::write(&from, "source").unwrap();
        let intent = Intent::new(&from, &to).unwrap();
        let (log, mut writer) = journal(&root).unwrap();
        record(
            &mut writer,
            &Record::Intent {
                id: 0,
                file: intent.clone(),
            },
        )
        .unwrap();
        std::fs::write(&to, "unrelated").unwrap();
        assert!(move_file(&from, &to, &intent).is_err());
        assert_eq!(undo(&log).unwrap().exit, Exit::Abstain);
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "unrelated");
        let to2 = root.join("to2");
        let intent2 = Intent::new(&from, &to2).unwrap();
        let (log2, mut writer2) = journal(&root).unwrap();
        record(
            &mut writer2,
            &Record::Intent {
                id: 0,
                file: intent2.clone(),
            },
        )
        .unwrap();
        move_file(&from, &to2, &intent2).unwrap();
        writer2.write_all(b"{\"event\":\"complete\"").unwrap();
        writer2.sync_all().unwrap();
        assert_eq!(undo(&log2).unwrap().exit, Exit::Ok);
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "source");
    }

    #[test]
    fn failed_intent_write_moves_nothing_and_partial_failure_names_journal() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let from = root.join("from");
        let second = root.join("second");
        std::fs::write(&from, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        let first = Intent::new(&from, &root.join("to")).unwrap();
        let next = Intent::new(&second, &root.join("missing/second")).unwrap();
        let (log, mut writer) = journal(&root).unwrap();
        let mut readonly = File::open(&log).unwrap();
        let error = apply_moves(std::slice::from_ref(&first), &log, &mut readonly)
            .unwrap_err()
            .to_string();
        assert!(from.exists());
        assert!(error.contains("completed 0"));
        let error = apply_moves(&[first, next], &log, &mut writer)
            .unwrap_err()
            .to_string();
        assert!(error.contains("completed 1"));
        assert!(error.contains(log.to_str().unwrap()));
        assert!(!from.exists());
        assert!(second.exists());
        assert_eq!(undo(&log).unwrap().exit, Exit::Ok);
        assert!(from.exists());
    }

    #[test]
    fn failed_undo_is_reported_and_symlink_parents_are_not_traversed() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let parent = root.join("source");
        std::fs::create_dir(&parent).unwrap();
        let from = parent.join("file");
        let to = root.join("to");
        std::fs::write(&from, "source").unwrap();
        let intent = Intent::new(&from, &to).unwrap();
        let (log, mut writer) = journal(&root).unwrap();
        apply_moves(&[intent], &log, &mut writer).unwrap();
        std::fs::rename(&parent, root.join("renamed")).unwrap();
        let error = undo(&log).err().unwrap().to_string();
        assert!(error.contains("undo move failed"));
        assert!(error.contains(log.to_str().unwrap()));
        std::os::unix::fs::symlink(root.join("renamed"), &parent).unwrap();
        assert!(open_regular(&from).is_err());
        assert!(undo(&log).is_err());
        assert!(to.exists());
    }

    #[test]
    fn source_identity_change_and_malformed_journal_fail_closed() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let from = root.join("from");
        let to = root.join("to");
        std::fs::write(&from, "original").unwrap();
        let intent = Intent::new(&from, &to).unwrap();
        std::fs::rename(&from, root.join("saved")).unwrap();
        std::fs::write(&from, "replacement").unwrap();
        assert!(move_file(&from, &to, &intent).is_err());
        assert!(!to.exists());
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "replacement");
        let (log, mut writer) = journal(&root).unwrap();
        writer
            .write_all(b"invalid legacy or damaged journal")
            .unwrap();
        assert!(undo(&log).is_err());
    }

    // The four places a process can die between journal steps, each as the journal it leaves
    // behind. `undo` reads that journal with no other memory of the run.
    #[test]
    fn a_crash_between_journal_steps_leaves_undo_one_consistent_choice() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let (from, to) = (root.join("from"), root.join("to"));
        std::fs::write(&from, "source").unwrap();
        let intent = Intent::new(&from, &to).unwrap();
        // Before the intent line was durable: an empty journal. Nothing to restore, exit 3.
        let (empty, _keep) = journal(&root).unwrap();
        let out = undo(&empty).unwrap();
        assert_eq!(out.exit, Exit::Abstain);
        assert!(out.data["skipped"].as_array().unwrap().is_empty());
        // Mid-write of the first intent line: a torn journal is refused, and the file, which
        // is only moved after the intent line is synced, is untouched.
        let (torn, mut writer) = journal(&root).unwrap();
        writer.write_all(b"{\"event\":\"intent\",\"id\":0").unwrap();
        assert_eq!(undo(&torn).err().unwrap().exit(), Exit::Input);
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "source");
        // After the intent, before the rename: the source is still at its original path, so
        // undo leaves it alone and says why. A second run says the same.
        let (pending, mut writer) = journal(&root).unwrap();
        record(
            &mut writer,
            &Record::Intent {
                id: 0,
                file: intent.clone(),
            },
        )
        .unwrap();
        drop(writer);
        let out = undo(&pending).unwrap();
        assert_eq!(out.exit, Exit::Abstain);
        assert_eq!(
            out.data["skipped"][0]["reason"],
            "original path occupied or intent not performed"
        );
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "source");
        assert!(!to.exists());
        // After the rename, before the completion line: the durable intent plus the file's
        // identity at the destination are enough to move it back.
        move_file(&from, &to, &intent).unwrap();
        assert_eq!(undo(&pending).unwrap().exit, Exit::Ok);
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "source");
        assert!(!to.exists());
        assert_eq!(undo(&pending).unwrap().exit, Exit::Abstain);
    }

    // `/dev/full` answers every write with ENOSPC: the exact error a full volume gives the
    // intent line. Nothing moves, and the error counts zero completed moves.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_full_disk_at_the_intent_write_moves_nothing() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let (from, to) = (root.join("from"), root.join("to"));
        std::fs::write(&from, "source").unwrap();
        let intent = Intent::new(&from, &to).unwrap();
        let mut full = File::options().write(true).open("/dev/full").unwrap();
        let error = apply_moves(&[intent], Path::new("/dev/full"), &mut full)
            .unwrap_err()
            .to_string();
        assert!(error.contains("completed 0"), "{error}");
        assert!(
            error.contains("No space left on device"),
            "ENOSPC is reported as such: {error}"
        );
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "source");
        assert!(!to.exists());
    }

    // A real full volume, opt in: `JEVIFY_SMALL_VOLUME_DIR` names a directory on a small
    // dedicated volume (a 2 MiB disk image on macOS, a size-bounded tmpfs on Linux). The test
    // fills that volume itself, tries a move and a journal on it, and checks the invariants
    // that hold whatever the filesystem answers: the file is at exactly one of its two paths,
    // undo reconciles it, and a journal that could not be written moves nothing.
    #[test]
    fn a_full_volume_keeps_the_file_at_exactly_one_path() {
        let Some(dir) = std::env::var_os("JEVIFY_SMALL_VOLUME_DIR") else {
            println!("SKIPPED: set JEVIFY_SMALL_VOLUME_DIR=<dir on a small dedicated volume>");
            return;
        };
        let root = Path::new(&dir).canonicalize().unwrap();
        let stamp = std::process::id();
        let sub = root.join(format!("full-{stamp}"));
        std::fs::create_dir(&sub).unwrap();
        let target_dir = sub.join("target");
        std::fs::create_dir(&target_dir).unwrap();
        let (from, to) = (sub.join("from"), target_dir.join("from"));
        std::fs::write(&from, "source").unwrap();
        let intent = Intent::new(&from, &to).unwrap();
        let (log, mut writer) = journal(&sub).unwrap();
        // Fill the volume; a 64 KiB block that fails to write in full is the end.
        let mut filler = File::create(sub.join("filler")).unwrap();
        let block = vec![0u8; 64 * 1024];
        let mut written = 0u64;
        loop {
            match filler.write_all(&block) {
                Ok(()) => written += block.len() as u64,
                Err(e) => {
                    assert_eq!(e.kind(), io::ErrorKind::StorageFull, "{e}");
                    break;
                }
            }
            assert!(written < 1 << 30, "the volume is not small: 1 GiB written");
        }
        filler.sync_all().ok();
        let moved = apply_moves(std::slice::from_ref(&intent), &log, &mut writer);
        eprintln!(
            "full volume at {}: apply_moves -> {moved:?}",
            root.display()
        );
        let at_from = std::fs::symlink_metadata(&from).is_ok();
        let at_to = intent.matches(&to);
        assert!(at_from != at_to, "exactly one path holds the file");
        if let Err(e) = moved {
            // Either the intent line or the completion line got ENOSPC; the rename, atomic
            // in both directions, is reflected in exactly one of the two paths above.
            let text = e.to_string();
            assert!(text.contains("No space left on device"), "{text}");
        }
        // A journal that cannot be created is ENOSPC, reported before any move.
        match journal(&sub) {
            Ok(_) => eprintln!("the full volume still creates an empty journal"),
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::StorageFull, "{e}"),
        }
        // Free the space (truncate, never delete) and reconcile.
        filler.set_len(0).unwrap();
        filler.sync_all().unwrap();
        let out = undo(&log).unwrap();
        eprintln!("undo after freeing space -> {}", out.data);
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "source");
        assert!(!to.exists());
    }

    fn fake_pdftotext(body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .prefix("jevify-pdftotext-")
            .tempdir()
            .unwrap()
            .keep();
        let file = dir.join("pdftotext");
        std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn a_sleeping_pdftotext_is_killed_at_the_deadline_and_the_excerpt_goes_on() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let pdf = root.join("scan.pdf");
        std::fs::write(&pdf, "%PDF-1.4 binary\0").unwrap();
        let slow = fake_pdftotext("exec /bin/sleep 5");
        let start = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(300);
        assert_eq!(
            pdf_text_within(&pdf, slow.as_os_str(), start + budget),
            None
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        // The converter receives the path, not stdin, and its text is read.
        let quick = fake_pdftotext("test -f \"$3\" && printf 'invoice from acme'");
        let far = std::time::Instant::now() + std::time::Duration::from_secs(5);
        assert_eq!(
            pdf_text_within(&pdf, quick.as_os_str(), far).as_deref(),
            Some("invoice from acme")
        );
        // An expired deadline runs none, and a symlink is never handed over.
        assert_eq!(
            pdf_text_within(&pdf, quick.as_os_str(), std::time::Instant::now()),
            None
        );
        let link = root.join("link.pdf");
        std::os::unix::fs::symlink(&pdf, &link).unwrap();
        assert_eq!(pdf_text_within(&link, quick.as_os_str(), far), None);
    }
}
