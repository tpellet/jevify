use crate::exit::JevifyError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub summary: String,
}

/// The tools on the PATH and how many undocumented names the cap dropped, so a caller can
/// report the real count instead of presenting a capped list as complete.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Inventory {
    pub tools: Vec<Tool>,
    pub omitted: usize,
}

const INDEX_TIMEOUT: Duration = Duration::from_secs(20);
/// The bound on one child's stdout: `manpath` is a line, a man index is a few megabytes.
const INDEX_OUTPUT_CAP: usize = 64 * 1024 * 1024;

static ENTRY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([^\s,()]+)\s*\(([0-9][A-Za-z0-9]*)\)").unwrap());

pub fn parse_whatis(text: &str, executables: &HashSet<String>) -> Vec<Tool> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let Some((names, desc)) = line.split_once(" - ") else {
            continue;
        };
        let desc = desc.trim();
        for cap in ENTRY.captures_iter(names) {
            let (name, section) = (&cap[1], &cap[2]);
            if matches!(section.chars().next(), Some('1' | '6' | '8')) && executables.contains(name)
            {
                out.entry(name.to_string())
                    .or_insert_with(|| desc.to_string());
            }
        }
    }
    out.into_iter()
        .map(|(name, summary)| Tool { name, summary })
        .collect()
}

fn executables(dirs: &[PathBuf]) -> HashSet<String> {
    let mut set = HashSet::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            // DirEntry::metadata does not follow symlinks; Homebrew and alternatives tools are symlinks.
            if let Ok(md) = std::fs::metadata(e.path()) {
                if md.is_file() && md.permissions().mode() & 0o111 != 0 {
                    set.insert(e.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }
    set
}

/// The best-effort stdout of a command (`source::Mode::BestEffort`): nothing from a command
/// still running at the deadline, so a slow `man` never hangs the caller. Also runs `man` and
/// `pdftotext` for their callers, which is why it is crate-visible.
pub(crate) fn output_within(command: Command, deadline: Instant, cap: usize) -> Option<Vec<u8>> {
    crate::source::supervise(command, crate::source::Mode::BestEffort, deadline, cap).ok()
}

fn whatis_text(path: &OsStr, deadline: Instant) -> String {
    let mut manpath = Command::new("manpath");
    manpath.env("PATH", path);
    let manpath = output_within(manpath, deadline, INDEX_OUTPUT_CAP)
        .map(|o| String::from_utf8_lossy(&o).trim().to_string())
        .unwrap_or_default();
    let mut text = String::new();
    for dir in manpath.split(':').filter(|d| !d.is_empty()) {
        if let Ok(t) = std::fs::read_to_string(Path::new(dir).join("whatis")) {
            text.push_str(&t);
            text.push('\n');
        }
    }
    if text.trim().is_empty() {
        // No plain whatis files (man-db on Linux, and recent macOS): ask man for its index.
        // Cold, this can regenerate the database (~2 s); the inventory cache makes it a one-off.
        let mut man = Command::new("man");
        man.args(["-k", "."])
            .env("PATH", path)
            .env("MANPAGER", "cat");
        if let Some(o) = output_within(man, deadline, INDEX_OUTPUT_CAP) {
            text = String::from_utf8_lossy(&o).into_owned();
        }
    }
    text
}

fn fingerprint(dirs: &[PathBuf]) -> String {
    let mut h = blake3::Hasher::new();
    for d in dirs {
        h.update(d.as_os_str().as_encoded_bytes());
        if let Ok(m) = std::fs::metadata(d).and_then(|m| m.modified()) {
            h.update(format!("{m:?}").as_bytes());
        }
    }
    h.finalize().to_hex()[..16].to_string()
}

/// The tools on the process PATH, for `route`.
pub fn load(cache_dir: Option<&Path>) -> Result<Vec<Tool>, JevifyError> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    load_with(&path, cache_dir, Instant::now() + INDEX_TIMEOUT).map(|inventory| inventory.tools)
}

/// The tools on an injected PATH. The man index is read under `deadline`; past it the index is
/// absent (names only, never cached). A cache directory of `None` never writes.
pub fn load_with(
    path: &OsStr,
    cache_dir: Option<&Path>,
    deadline: Instant,
) -> Result<Inventory, JevifyError> {
    let dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    let cache_file = cache_dir.map(|c| c.join(format!("inventory-{}.json", fingerprint(&dirs))));
    if let Some(inventory) = cache_file
        .as_ref()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice::<Inventory>(&b).ok())
    {
        return Ok(inventory);
    }
    let exes = executables(&dirs);
    let mut tools = parse_whatis(&whatis_text(path, deadline), &exes);
    // Tools people actually reach for (rg, fd, uv, ...) often ship no man page, and minimal
    // Linux images ship no whatis database at all: list those by name rather than hide them.
    let documented: HashSet<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    let system = |d: &Path| {
        ["/bin", "/sbin", "/usr/bin", "/usr/sbin", "/usr/libexec"]
            .iter()
            .any(|s| d == Path::new(s))
    };
    // No man index at all (containers, `man -k` refused inside a sandbox, or past the deadline):
    // names only, and that degraded list is never cached, so the next run with a working `man`
    // rebuilds it.
    let names_only = tools.is_empty();
    // 1,000 names cost ~5k tokens and a few windows; 400 dropped uv and yq on a Mac with ~600
    // undocumented tools on PATH.
    let (extra_dirs, cap): (Vec<PathBuf>, usize) = if names_only {
        (dirs.clone(), 1_500)
    } else {
        (dirs.iter().filter(|d| !system(d)).cloned().collect(), 1_000)
    };
    // Cap in PATH order (alphabetical within a directory), not alphabetically overall: the
    // user's own directories come first on PATH, and an alphabetical cut would drop late-alphabet
    // tools such as uv, yq or zoxide on a Homebrew-heavy machine.
    let mut extra: Vec<String> = Vec::new();
    for d in &extra_dirs {
        let mut names: Vec<String> = executables(std::slice::from_ref(d))
            .into_iter()
            .filter(|n| !documented.contains(n.as_str()) && !extra.contains(n))
            .collect();
        names.sort();
        extra.extend(names);
    }
    let omitted = extra.len().saturating_sub(cap);
    extra.truncate(cap);
    tools.extend(extra.into_iter().map(|name| Tool {
        name,
        summary: "(no man page)".into(),
    }));
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    if tools.is_empty() {
        return Err(JevifyError::Input("no executables found on PATH".into()));
    }
    let inventory = Inventory { tools, omitted };
    if let Some(f) = cache_file.filter(|_| !names_only) {
        if let Some(p) = f.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::write(&f, serde_json::to_vec(&inventory).unwrap_or_default());
    }
    Ok(inventory)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_macos_and_linux_whatis_lines() {
        let exe: std::collections::HashSet<String> = ["tar", "bsdtar", "curl", "drutil"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let text = "tar(1), bsdtar(1)        - manipulate tape archives\ncurl (1)             - transfer a URL\nfopen(3)                 - stream open functions\ndrutil(1)                - interact with CD/DVD burners\n";
        let tools = parse_whatis(text, &exe);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["bsdtar", "curl", "drutil", "tar"]);
        assert_eq!(tools[3].summary, "manipulate tape archives");
    }
    #[test]
    fn executables_follow_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        std::fs::write(&real, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&real, d.path().join("link")).unwrap();
        let set = executables(&[d.path().to_path_buf()]);
        assert!(set.contains("real") && set.contains("link"));
    }

    /// A directory of `count` empty executables named `t0000`.. and a `cache` subdirectory.
    fn bin_dir(count: usize) -> PathBuf {
        let dir = tempfile::Builder::new()
            .prefix("jevify-inventory-")
            .tempdir()
            .unwrap()
            .keep();
        for i in 0..count {
            let file = dir.join(format!("t{i:04}"));
            std::fs::write(&file, "").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::create_dir(dir.join("cache")).unwrap();
        dir
    }

    fn script(dir: &Path, name: &str, body: &str) {
        let file = dir.join(name);
        std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn undocumented_names_above_the_cap_are_counted_not_hidden() {
        let dir = bin_dir(1_501);
        let cache = dir.join("cache");
        let deadline = Instant::now() + INDEX_TIMEOUT;
        // No man index on this PATH: names only, cap 1,500, nothing cached.
        let inventory = load_with(dir.as_os_str(), Some(&cache), deadline).unwrap();
        assert_eq!((inventory.tools.len(), inventory.omitted), (1_500, 1));
        assert_eq!(inventory.tools[0].name, "t0000");
        assert!(inventory.tools.iter().all(|t| t.summary == "(no man page)"));
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        // A man index that documents `man` itself: cap 1,000 for the rest, and the count of
        // dropped names survives the cache round trip.
        script(&dir, "man", "printf 'man(1) - format manual pages\\n'");
        let inventory = load_with(dir.as_os_str(), Some(&cache), deadline).unwrap();
        assert_eq!((inventory.tools.len(), inventory.omitted), (1_001, 501));
        assert_eq!(
            inventory.tools.iter().filter(|t| t.name == "man").count(),
            1
        );
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
        let cached = load_with(dir.as_os_str(), Some(&cache), Instant::now()).unwrap();
        assert_eq!(cached, inventory);
        assert_eq!(
            load_with(OsStr::new("/nonexistent"), None, deadline)
                .unwrap_err()
                .exit()
                .code(),
            6
        );
    }

    #[test]
    fn a_slow_man_index_is_absent_within_the_deadline_and_never_cached() {
        let dir = bin_dir(3);
        let cache = dir.join("cache");
        script(&dir, "man", "exec /bin/sleep 5");
        let start = Instant::now();
        let budget = Duration::from_millis(300);
        let inventory = load_with(dir.as_os_str(), Some(&cache), start + budget).unwrap();
        assert!(
            start.elapsed() < budget + crate::source::READER_GRACE,
            "{:?}",
            start.elapsed()
        );
        assert_eq!(inventory.omitted, 0);
        let names: Vec<_> = inventory.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["man", "t0000", "t0001", "t0002"]);
        assert!(inventory.tools.iter().all(|t| t.summary == "(no man page)"));
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
        // An expired deadline runs no command at all.
        assert!(output_within(Command::new("man"), Instant::now(), 1024).is_none());
    }
}
