use crate::{
    exit::JevifyError,
    records::{Record, Split},
};
use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap, HashSet},
    ffi::{OsStr, OsString},
    io::Read,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The coded kinds, then the shipped recipes of `src/kinds.jsonl`, in file order.
pub const KINDS: &[&str] = &[
    "-",
    "branch",
    "commit",
    "file",
    "dir",
    "tool",
    "pr",
    "issue",
    "ci-run",
    "stash",
    "process",
    "container",
    "pod",
];
pub const LISTER_TIMEOUT: Duration = Duration::from_secs(20);
const OUTPUT_CAP: usize = 64 * 1024 * 1024;
/// Characters of diffstat and patch one commit finalist carries. A keyless finals window
/// budgets 30,000 characters over at most 24 finalists, 1,250 each, and a subject, a body and
/// a path list leave most of that unspent; 1,000 fills the room without crowding a finalist
/// out. Above 24 finalists the window's own per-item clip cuts the diff further.
const DIFF_CHARS: usize = 1_000;
const READER_GRACE: Duration = Duration::from_millis(200);
/// The shipped recipes, one JSON object per line.
const SHIPPED: &str = include_str!("kinds.jsonl");
/// The user's recipes, in the configuration directory only, never in the working directory.
const USER_FILE: &str = "kinds.jsonl";
const BRANCH_ARGV: [&str; 6] = [
    "git",
    "for-each-ref",
    "--sort=-committerdate",
    "--format=%(refname)%00%(symref)%00%(committerdate:unix)%00%(subject)%00",
    "refs/heads",
    "refs/remotes",
];
/// `-n <limit>` is inserted after `log`; the total comes from `git rev-list --count HEAD`.
const COMMIT_ARGV: [&str; 6] = ["git", "log", "-z", "--format=%H%x00%s", "HEAD", "--"];
const FILE_ARGV: [&str; 5] = ["git", "ls-files", "-co", "--exclude-standard", "-z"];

#[derive(Debug, Clone)]
pub struct Kind {
    pub name: Cow<'static, str>,
    pub coded: bool,
    pub ordered: bool,
    pub path_kind: bool,
    pub has_tier_two: bool,
}

const fn recipe_kind(name: &'static str, ordered: bool) -> Kind {
    Kind {
        name: Cow::Borrowed(name),
        coded: false,
        ordered,
        path_kind: false,
        has_tier_two: false,
    }
}

pub const REGISTRY: &[Kind] = &[
    Kind {
        name: Cow::Borrowed("-"),
        coded: true,
        ordered: false,
        path_kind: false,
        has_tier_two: false,
    },
    Kind {
        name: Cow::Borrowed("branch"),
        coded: true,
        ordered: true,
        // The literal before the marker scopes the listing (`origin/@{branch:x}` lists that
        // remote's refs), as for `file` and `dir`; no ref name starts with a dash.
        path_kind: true,
        has_tier_two: true,
    },
    Kind {
        name: Cow::Borrowed("commit"),
        coded: true,
        ordered: true,
        path_kind: false,
        has_tier_two: true,
    },
    Kind {
        name: Cow::Borrowed("file"),
        coded: true,
        ordered: false,
        path_kind: true,
        has_tier_two: true,
    },
    Kind {
        name: Cow::Borrowed("dir"),
        coded: true,
        ordered: false,
        path_kind: true,
        has_tier_two: true,
    },
    Kind {
        name: Cow::Borrowed("tool"),
        coded: true,
        ordered: false,
        path_kind: false,
        has_tier_two: false,
    },
    recipe_kind("pr", true),
    recipe_kind("issue", true),
    recipe_kind("ci-run", true),
    recipe_kind("stash", true),
    recipe_kind("process", false),
    recipe_kind("container", false),
    recipe_kind("pod", false),
];

/// A shipped kind: coded, or a recipe of `src/kinds.jsonl`. User recipes need [`lookup`].
pub fn kind(name: &str) -> Option<&'static Kind> {
    REGISTRY.iter().find(|kind| kind.name == name)
}

/// A kind as one JSON line: the lister argv and the handle.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub kind: String,
    pub list: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default)]
    pub ordered: bool,
}

fn valid_kind_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b == b'-')
}

fn parse_recipe(line: &[u8]) -> Result<Recipe, String> {
    let recipe: Recipe = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    if !valid_kind_name(&recipe.kind) {
        return Err(format!(
            "kind {:?} does not match [a-z][a-z-]*",
            recipe.kind
        ));
    }
    if recipe.list.is_empty() {
        return Err(format!("kind {}: list is empty", recipe.kind));
    }
    if recipe.field.is_some() && recipe.key.is_some() {
        return Err(format!(
            "kind {}: field and key are mutually exclusive",
            recipe.kind
        ));
    }
    if recipe.field == Some(0) {
        return Err(format!("kind {}: field numbers start at 1", recipe.kind));
    }
    Ok(recipe)
}

fn shipped() -> &'static [Recipe] {
    static SHIPPED_RECIPES: LazyLock<Vec<Recipe>> = LazyLock::new(|| {
        SHIPPED
            .lines()
            .map(|line| parse_recipe(line.as_bytes()).expect("a shipped recipe parses"))
            .collect()
    });
    &SHIPPED_RECIPES
}

/// Read the user's `kinds.jsonl` from `env.config_dir`. Every line must parse and no line may
/// name a shipped kind, or the whole file is `recipe_invalid`. A missing file holds no recipe.
fn user_recipes(env: &Env) -> Result<Vec<Recipe>, JevifyError> {
    let Some(dir) = &env.config_dir else {
        return Ok(Vec::new());
    };
    let path = dir.join(USER_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(JevifyError::recipe_invalid(format!(
                "{}: {e}",
                path.display()
            )));
        }
    };
    let mut recipes: Vec<Recipe> = Vec::new();
    for (index, line) in bytes.split(|b| *b == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let invalid = |message: String| {
            JevifyError::recipe_invalid(format!("{} line {}: {message}", path.display(), index + 1))
        };
        let recipe = parse_recipe(line).map_err(invalid)?;
        if kind(&recipe.kind).is_some() {
            return Err(invalid(format!(
                "kind {} is built in and cannot be replaced",
                recipe.kind
            )));
        }
        if recipes.iter().any(|r| r.kind == recipe.kind) {
            return Err(invalid(format!("kind {} is defined twice", recipe.kind)));
        }
        recipes.push(recipe);
    }
    Ok(recipes)
}

/// Find a kind: coded kinds and shipped recipes first, and only for a name in neither, the
/// user's `kinds.jsonl`. `Ok(None)` is a name found nowhere.
pub fn lookup(name: &str, env: &Env) -> Result<Option<Kind>, JevifyError> {
    if let Some(kind) = kind(name) {
        return Ok(Some(kind.clone()));
    }
    Ok(user_recipes(env)?
        .into_iter()
        .find(|recipe| recipe.kind == name)
        .map(|recipe| Kind {
            name: Cow::Owned(recipe.kind),
            coded: false,
            ordered: recipe.ordered,
            path_kind: false,
            has_tier_two: false,
        }))
}

/// The recipe of a kind that is not coded, with the same read rules as [`lookup`].
fn recipe(name: &str, env: &Env) -> Result<Option<Recipe>, JevifyError> {
    if kind(name).is_some_and(|kind| kind.coded) {
        return Ok(None);
    }
    if let Some(recipe) = shipped().iter().find(|recipe| recipe.kind == name) {
        return Ok(Some(recipe.clone()));
    }
    Ok(user_recipes(env)?
        .into_iter()
        .find(|recipe| recipe.kind == name))
}

/// One kind as `capabilities` prints it: where it comes from and the argv of its lister.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CatalogEntry {
    pub name: String,
    /// `coded`, `shipped` or `user`.
    pub origin: &'static str,
    /// Empty for `-`, which reads stdin or `--candidates`, and for `tool`, which reads the PATH
    /// and the man index in process.
    pub list: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Catalog {
    pub kinds: Vec<CatalogEntry>,
    /// Why the user's `kinds.jsonl` was not listed; a bad file never fails the caller.
    pub error: Option<String>,
}

/// Every kind with its lister argv, the user's recipes included.
pub fn catalog(env: &Env) -> Catalog {
    let coded = |name: &str, argv: &[&str]| CatalogEntry {
        name: name.into(),
        origin: "coded",
        list: argv.iter().map(|arg| (*arg).to_owned()).collect(),
    };
    let mut kinds = vec![
        coded("-", &[]),
        coded("branch", &BRANCH_ARGV),
        coded("commit", &COMMIT_ARGV),
        coded("file", &FILE_ARGV),
        coded("dir", &FILE_ARGV),
        coded("tool", &[]),
    ];
    let entry = |recipe: &Recipe, origin| CatalogEntry {
        name: recipe.kind.clone(),
        origin,
        list: recipe.list.clone(),
    };
    kinds.extend(shipped().iter().map(|recipe| entry(recipe, "shipped")));
    let error = match user_recipes(env) {
        Ok(recipes) => {
            kinds.extend(recipes.iter().map(|recipe| entry(recipe, "user")));
            None
        }
        Err(e) => Some(e.to_string()),
    };
    Catalog { kinds, error }
}

pub enum Scope {
    Input {
        bytes: Vec<u8>,
        split: Split,
        field: Option<usize>,
        key: Option<String>,
    },
    Prefix(Option<PathBuf>),
}

#[derive(Debug)]
pub struct Listing {
    pub records: Vec<Record>,
    pub total: usize,
    pub omitted: usize,
    pub ordered: bool,
}

#[derive(Clone)]
pub struct Env {
    pub path: OsString,
    pub config_dir: Option<PathBuf>,
    /// The value of `JEVIFY_CACHE_DIR`, for the inventory of `tool`; `None` in inline tests.
    pub cache_dir: Option<PathBuf>,
    pub deadline: Instant,
    pub cwd: PathBuf,
}

impl Env {
    pub fn from_process(timeout: Duration) -> Self {
        let vars: std::collections::HashMap<_, _> = std::env::vars_os().collect();
        Self {
            path: vars
                .get(std::ffi::OsStr::new("PATH"))
                .cloned()
                .unwrap_or_default(),
            config_dir: crate::config::config_dir(
                vars.get(std::ffi::OsStr::new("JEVIFY_CONFIG_DIR"))
                    .and_then(|value| value.to_str()),
            ),
            cache_dir: vars
                .get(std::ffi::OsStr::new("JEVIFY_CACHE_DIR"))
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            deadline: Instant::now() + timeout,
            // An unavailable cwd must fail in Command, never silently list another directory.
            cwd: std::env::current_dir().unwrap_or_default(),
        }
    }
}

pub async fn enumerate(
    kind: &str,
    scope: Scope,
    limit: usize,
    env: &Env,
) -> Result<Listing, JevifyError> {
    let kind = kind.to_owned();
    let env = env.clone();
    tokio::task::spawn_blocking(move || match (kind.as_str(), scope) {
        (
            "-",
            Scope::Input {
                bytes,
                split,
                field,
                key,
            },
        ) => input_listing(&bytes, split, field, key.as_deref(), false, usize::MAX),
        ("branch", Scope::Prefix(prefix)) => branches(prefix.as_deref(), limit, &env),
        ("commit", Scope::Prefix(None)) => commits(limit, &env),
        ("file", Scope::Prefix(prefix)) => paths(prefix.as_deref(), false, limit, &env),
        ("dir", Scope::Prefix(prefix)) => paths(prefix.as_deref(), true, limit, &env),
        ("tool", Scope::Prefix(None)) => tools(&env),
        (name, Scope::Prefix(None)) => match recipe(name, &env)? {
            Some(recipe) => recipe_listing(&recipe, limit, &env),
            None => Err(JevifyError::Usage(format!("unknown kind {name}"))),
        },
        _ => Err(JevifyError::Usage(format!(
            "invalid kind or scope for {kind}"
        ))),
    })
    .await
    .map_err(|e| JevifyError::lister_failed(e.to_string()))?
}

pub async fn enrich(kind: &str, handles: &[OsString]) -> Vec<String> {
    enrich_with_env(kind, handles, &Env::from_process(LISTER_TIMEOUT)).await
}

/// Missing enrichment is empty evidence; no partial lister output is returned.
pub async fn enrich_with_env(kind: &str, handles: &[OsString], env: &Env) -> Vec<String> {
    enrich_in(kind, Path::new(""), handles, env).await.0
}

/// Tier-two evidence for the given finalists only, and the number of finalists whose excerpt
/// was withheld. `file` and `dir` handles are relative to `prefix` (the literal of the marker,
/// resolved as in enumeration) under `env.cwd`; the other kinds ignore `prefix` and withhold
/// nothing. `file` finalists carry their first lines, `dir` finalists the names of their first
/// children. Missing enrichment is empty evidence.
pub async fn enrich_in(
    kind: &str,
    prefix: &Path,
    handles: &[OsString],
    env: &Env,
) -> (Vec<String>, usize) {
    let empty = || (vec![String::new(); handles.len()], 0);
    let evidence: fn(&OsStr, &Env) -> Result<String, JevifyError> = match kind {
        "branch" => branch_evidence,
        "commit" => commit_evidence,
        "dir" => {
            let prefix = (!prefix.as_os_str().is_empty()).then_some(prefix);
            let Ok(relative) = resolve_prefix(prefix, env) else {
                return empty();
            };
            let root = env.cwd.join(relative);
            let handles = handles.to_vec();
            let count = handles.len();
            return tokio::task::spawn_blocking(move || {
                let mut withheld = 0;
                let values = handles
                    .iter()
                    .map(|handle| {
                        let (evidence, kept_out) = dir_children(&root, Path::new(handle));
                        withheld += usize::from(kept_out);
                        evidence
                    })
                    .collect();
                (values, withheld)
            })
            .await
            .unwrap_or_else(|_| (vec![String::new(); count], 0));
        }
        "file" => {
            let prefix = (!prefix.as_os_str().is_empty()).then_some(prefix);
            let Ok(relative) = resolve_prefix(prefix, env) else {
                return empty();
            };
            let mut records: Vec<Record> = handles
                .iter()
                .map(|handle| Record {
                    handle: relative.join(handle).into_os_string(),
                    evidence: String::new(),
                    raw: 0..0,
                })
                .collect();
            return match crate::records::excerpts(&mut records, &env.cwd).await {
                Ok(unread) => (
                    records.into_iter().map(|r| r.evidence).collect(),
                    unread.count,
                ),
                Err(_) => empty(),
            };
        }
        _ => return empty(),
    };
    let env = env.clone();
    // A branch under a literal prefix is listed by the rest of its ref name: the ref is both.
    let scoped = matches!(kind, "branch") && !prefix.as_os_str().is_empty();
    let handles: Vec<OsString> = if scoped {
        handles
            .iter()
            .map(|handle| {
                let mut rev = prefix.as_os_str().to_owned();
                rev.push(handle);
                rev
            })
            .collect()
    } else {
        handles.to_vec()
    };
    let count = handles.len();
    let values = tokio::task::spawn_blocking(move || {
        handles
            .iter()
            .map(|handle| evidence(handle, &env).unwrap_or_default())
            .collect()
    })
    .await
    .unwrap_or_else(|_| vec![String::new(); count]);
    (values, 0)
}

fn input_listing(
    bytes: &[u8],
    split: Split,
    field: Option<usize>,
    key: Option<&str>,
    ordered: bool,
    limit: usize,
) -> Result<Listing, JevifyError> {
    if field.is_some() && key.is_some() {
        return Err(JevifyError::Usage(
            "field and key are mutually exclusive".into(),
        ));
    }
    let (mut records, mut omitted) = if let Some(key) = key {
        crate::records::key(bytes, key)?
    } else {
        (crate::records::parse(bytes, split)?, 0)
    };
    if let Some(field) = field {
        omitted += crate::records::field(&mut records, field)?;
    }
    Ok(listing(records, omitted, ordered, limit))
}

/// The `-` path over a lister's output. The evidence is the whole record; the limit only cuts
/// an ordered listing, and the argv is never rewritten.
fn recipe_listing(recipe: &Recipe, limit: usize, env: &Env) -> Result<Listing, JevifyError> {
    let argv: Vec<OsString> = recipe.list.iter().map(OsString::from).collect();
    let bytes = run_lister_blocking(&argv, env, OUTPUT_CAP)?;
    input_listing(
        &bytes,
        Split::Lines,
        recipe.field,
        recipe.key.as_deref(),
        recipe.ordered,
        limit,
    )
    .map_err(|e| JevifyError::lister_failed(format!("{}: {e}", recipe.list[0])))
}

fn listing(mut records: Vec<Record>, mut omitted: usize, ordered: bool, limit: usize) -> Listing {
    let mut seen = HashSet::new();
    records.retain(|record| {
        if record
            .handle
            .as_bytes()
            .iter()
            .any(|b| matches!(b, b'\n' | b'\r' | 0))
        {
            omitted += 1;
            return false;
        }
        seen.insert((record.handle.clone(), record.evidence.clone()))
    });
    let total = records.len();
    if ordered {
        records.truncate(limit);
    }
    Listing {
        records,
        total,
        omitted,
        ordered,
    }
}

/// Run an argv on the blocking pool. Success requires exit 0 and EOF on both pipes.
pub async fn run_lister(argv: &[OsString], env: &Env) -> Result<Vec<u8>, JevifyError> {
    let argv = argv.to_vec();
    let env = env.clone();
    tokio::task::spawn_blocking(move || run_lister_blocking(&argv, &env, OUTPUT_CAP))
        .await
        .map_err(|e| JevifyError::lister_failed(e.to_string()))?
}

fn read_pipe(mut pipe: impl Read, cap: usize, size: &AtomicUsize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let n = match pipe.read(&mut buffer) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        };
        if n == 0 {
            return Ok(bytes);
        }
        if size.fetch_add(n, Ordering::Relaxed).saturating_add(n) > cap {
            return Err("output exceeds lister byte cap".into());
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
}

fn run_lister_blocking(argv: &[OsString], env: &Env, cap: usize) -> Result<Vec<u8>, JevifyError> {
    let Some(program) = argv.first() else {
        return Err(JevifyError::lister_failed("empty lister argv".into()));
    };
    let fail = |message: String| {
        JevifyError::lister_failed(format!("{}: {message}", program.to_string_lossy()))
    };
    if Instant::now() >= env.deadline {
        return Err(fail("deadline exceeded".into()));
    }
    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .current_dir(&env.cwd)
        .env("PATH", &env.path)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &env.config_dir {
        command.env("JEVIFY_CONFIG_DIR", dir);
    } else {
        command.env_remove("JEVIFY_CONFIG_DIR");
    }
    let mut child = command.spawn().map_err(|e| fail(e.to_string()))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = mpsc::channel();
    let size = Arc::new(AtomicUsize::new(0));
    let out_size = Arc::clone(&size);
    let out_tx = tx.clone();
    std::thread::spawn(move || {
        let _ = out_tx.send((0, read_pipe(stdout, cap, &out_size)));
    });
    std::thread::spawn(move || {
        let _ = tx.send((1, read_pipe(stderr, cap, &size)));
    });
    let mut streams = [None, None];
    let mut failure = None;
    let status = loop {
        while let Ok((index, result)) = rx.try_recv() {
            streams[index] = Some(result);
        }
        if let Some(error) = streams.iter().flatten().find_map(|s| s.as_ref().err()) {
            failure = Some(error.clone());
        }
        if Instant::now() >= env.deadline {
            failure = Some("deadline exceeded".into());
        }
        if failure.is_some() {
            let _ = child.kill();
            break child.wait().map_err(|e| fail(e.to_string()))?;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(
                Duration::from_millis(20)
                    .min(env.deadline.saturating_duration_since(Instant::now())),
            ),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                failure = Some(e.to_string());
                break std::process::ExitStatus::default();
            }
        }
    };
    let grace = Instant::now() + READER_GRACE;
    while streams.iter().any(Option::is_none) {
        match rx.recv_timeout(grace.saturating_duration_since(Instant::now())) {
            Ok((index, result)) => streams[index] = Some(result),
            Err(_) => {
                failure.get_or_insert("lister pipes did not reach EOF".into());
                break;
            }
        }
    }
    for error in streams.iter().flatten().filter_map(|s| s.as_ref().err()) {
        failure.get_or_insert(error.clone());
    }
    if !status.success() {
        failure.get_or_insert(format!("exited with {status}"));
    }
    if let Some(mut message) = failure {
        if let Some(Ok(stderr)) = &streams[1] {
            let tail = &stderr[stderr.len().saturating_sub(4096)..];
            let tail = String::from_utf8_lossy(tail);
            let tail = tail.trim_end();
            if !tail.is_empty() {
                message.push_str(&format!(": {tail}"));
            }
        }
        return Err(fail(message));
    }
    streams[0]
        .take()
        .expect("both readers finished")
        .map_err(fail)
}

/// Without a prefix, a branch is its name: `git switch` and `git checkout` resolve the short
/// name of a remote-only branch by their DWIM rule, but no other git command does, and no one
/// spelling satisfies both (`git switch origin/x` refuses a remote ref). jevify does not parse
/// the command, so the caller says which by the literal it writes: `origin/@{branch:x}` lists
/// the refs under `refs/remotes/origin/` by the rest of their name, and the argument becomes
/// the ref (`origin/ticket/TPE-791`), which every command that takes a revision resolves. A
/// local branch named `origin/x` is not under that prefix; a prefix with no remote ref under
/// it fails and names the remotes that exist.
fn branches(prefix: Option<&Path>, limit: usize, env: &Env) -> Result<Listing, JevifyError> {
    let bytes = run_lister_blocking(&BRANCH_ARGV.map(OsString::from), env, OUTPUT_CAP)?;
    let mut refs = Vec::new();
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        let parts: Vec<_> = line.split(|b| *b == 0).collect();
        if parts.len() != 5 || !parts[4].is_empty() {
            return Err(JevifyError::lister_failed(
                "malformed git ref listing".into(),
            ));
        }
        if !parts[1].is_empty() {
            continue;
        }
        let timestamp = std::str::from_utf8(parts[2])
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| JevifyError::lister_failed("invalid git committer date".into()))?;
        refs.push((parts[0], parts[3], timestamp));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let record = |handle: &[u8], shown: &[u8], subject: &[u8], timestamp: u64| Record {
        handle: OsString::from_vec(handle.to_vec()),
        evidence: format!(
            "{} — {} — {}",
            String::from_utf8_lossy(shown),
            String::from_utf8_lossy(subject),
            age(now, timestamp)
        ),
        raw: 0..0,
    };
    if let Some(prefix) = prefix.map(|p| p.as_os_str().as_bytes()) {
        // Remote refs only: a local branch named `origin/x` lives under `refs/heads/` and is
        // not what `origin/` names.
        let records: Vec<_> = refs
            .iter()
            .filter_map(|(name, subject, timestamp)| {
                let shown = name.strip_prefix(b"refs/remotes/")?;
                let handle = shown.strip_prefix(prefix)?;
                (!handle.is_empty()).then(|| record(handle, shown, subject, *timestamp))
            })
            .collect();
        if records.is_empty() {
            let mut remotes: Vec<_> = refs
                .iter()
                .filter_map(|(name, _, _)| name.strip_prefix(b"refs/remotes/"))
                .filter_map(|remote| remote.iter().position(|b| *b == b'/').map(|i| &remote[..i]))
                .map(|remote| String::from_utf8_lossy(remote).into_owned())
                .collect();
            remotes.sort();
            remotes.dedup();
            return Err(JevifyError::lister_failed(format!(
                "prefix {} names no remote ref; remotes: {}",
                String::from_utf8_lossy(prefix),
                if remotes.is_empty() {
                    "none".to_owned()
                } else {
                    remotes.join(", ")
                }
            )));
        }
        return Ok(listing(records, 0, true, limit));
    }
    let locals: HashSet<_> = refs
        .iter()
        .filter_map(|(name, _, _)| name.strip_prefix(b"refs/heads/"))
        .collect();
    // `refs/remotes/<remote>/<short>` splits at the first slash. How many remotes track each
    // short name decides whether git's DWIM (`git switch <short>`) can resolve it.
    fn remote_short(remote: &[u8]) -> &[u8] {
        remote
            .iter()
            .position(|b| *b == b'/')
            .map_or(remote, |slash| &remote[slash + 1..])
    }
    let mut tracked: HashMap<&[u8], usize> = HashMap::new();
    for (name, _, _) in &refs {
        if let Some(remote) = name.strip_prefix(b"refs/remotes/") {
            *tracked.entry(remote_short(remote)).or_default() += 1;
        }
    }
    let mut records = Vec::new();
    for (name, subject, timestamp) in &refs {
        let (handle, shown) = if let Some(local) = name.strip_prefix(b"refs/heads/") {
            (local, local)
        } else if let Some(remote) = name.strip_prefix(b"refs/remotes/") {
            let short = remote_short(remote);
            if locals.contains(short) {
                continue;
            }
            // One remote tracks it: the short name, which `git switch` and `git checkout`
            // resolve to a local tracking branch. Several do: git refuses the short name as
            // ambiguous, so the qualified ref stays.
            if tracked.get(short) == Some(&1) {
                (short, remote)
            } else {
                (remote, remote)
            }
        } else {
            return Err(JevifyError::lister_failed(
                "unexpected git ref namespace".into(),
            ));
        };
        records.push(record(handle, shown, subject, *timestamp));
    }
    Ok(listing(records, 0, true, limit))
}

fn age(now: u64, timestamp: u64) -> String {
    let seconds = now.saturating_sub(timestamp);
    let (count, unit) = if seconds >= 86400 {
        (seconds / 86400, "day")
    } else if seconds >= 3600 {
        (seconds / 3600, "hour")
    } else if seconds >= 60 {
        (seconds / 60, "minute")
    } else {
        (seconds, "second")
    };
    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}

fn branch_evidence(handle: &OsStr, env: &Env) -> Result<String, JevifyError> {
    let log = |revs: [OsString; 2]| {
        let mut argv: Vec<OsString> = [
            "git",
            "log",
            "-5",
            "--format=%x00%s%x00",
            "--name-only",
            "-z",
            "--no-renames",
            "--no-ext-diff",
        ]
        .map(OsString::from)
        .to_vec();
        argv.extend(revs);
        argv.push("--".into());
        run_lister_blocking(&argv, env, OUTPUT_CAP)
    };
    // A remote-only branch is listed by its short name, which is not a rev: read it from the
    // remote-tracking refs instead.
    let bytes = match log(["--end-of-options".into(), handle.to_owned()]) {
        Ok(bytes) => bytes,
        Err(error) => {
            let mut remotes = OsString::from("--remotes=*/");
            remotes.push(handle);
            log([remotes, "--end-of-options".into()]).map_err(|_| error)?
        }
    };
    // Each commit starts with an empty NUL field, then its subject. Paths follow.
    let fields: Vec<_> = bytes.split(|b| *b == 0).collect();
    let mut subjects = Vec::new();
    let mut paths = BTreeSet::new();
    let mut index = 0;
    while index + 1 < fields.len() {
        if fields[index].is_empty() {
            index += 1;
            subjects.push(String::from_utf8_lossy(fields[index]).into_owned());
            index += 1;
            // -z adds a NUL after the formatted subject's own NUL.
            if index < fields.len() && fields[index].is_empty() {
                index += 1;
            }
        } else {
            let path = fields[index].strip_prefix(b"\n").unwrap_or(fields[index]);
            if !path.is_empty() {
                paths.insert(
                    String::from_utf8_lossy(path.split(|b| *b == b'/').next().unwrap_or(path))
                        .into_owned(),
                );
            }
            index += 1;
        }
    }
    Ok(format!(
        "{}\nLast 5 subjects:\n{}\nChanged top-level paths: {}",
        handle.to_string_lossy(),
        subjects.join("\n"),
        paths.into_iter().collect::<Vec<_>>().join(", ")
    ))
}

/// The newest `limit` commits of HEAD, full OIDs and subjects, and the exact total of the
/// history. The listing and the count run at the same time; an empty history fails in git.
fn commits(limit: usize, env: &Env) -> Result<Listing, JevifyError> {
    let mut log: Vec<OsString> = COMMIT_ARGV.map(OsString::from).to_vec();
    log.splice(2..2, ["-n".into(), limit.to_string().into()]);
    let count_argv = ["git", "rev-list", "--count", "HEAD", "--"].map(OsString::from);
    let (log, count) = std::thread::scope(|scope| {
        let count = scope.spawn(|| run_lister_blocking(&count_argv, env, OUTPUT_CAP));
        let log = run_lister_blocking(&log, env, OUTPUT_CAP);
        let count = count
            .join()
            .unwrap_or_else(|_| Err(JevifyError::lister_failed("git: count failed".into())));
        (log, count)
    });
    let bytes = log?;
    let total: usize = String::from_utf8_lossy(&count?)
        .trim()
        .parse()
        .map_err(|_| JevifyError::lister_failed("git: malformed commit count".into()))?;
    let fields: Vec<_> = bytes.split(|b| *b == 0).collect();
    let mut records = Vec::new();
    for pair in fields.chunks(2) {
        match pair {
            [oid, subject] if !oid.is_empty() => records.push(Record {
                handle: OsString::from_vec(oid.to_vec()),
                evidence: String::from_utf8_lossy(subject).into_owned(),
                raw: 0..0,
            }),
            [[]] => {}
            _ => {
                return Err(JevifyError::lister_failed(
                    "malformed git log listing".into(),
                ));
            }
        }
    }
    let mut listing = listing(records, 0, true, limit);
    listing.total = total.max(listing.total);
    Ok(listing)
}

fn commit_evidence(handle: &OsStr, env: &Env) -> Result<String, JevifyError> {
    let bytes = run_lister_blocking(
        &[
            "git".into(),
            "log".into(),
            "-1".into(),
            "--format=%b%x00".into(),
            "--name-only".into(),
            "-z".into(),
            "--no-renames".into(),
            "--no-ext-diff".into(),
            "--end-of-options".into(),
            handle.to_owned(),
            "--".into(),
        ],
        env,
        OUTPUT_CAP,
    )?;
    // The body ends at the format's NUL; -z adds one more, then the paths, NUL-terminated.
    let (body, rest) = bytes
        .iter()
        .position(|b| *b == 0)
        .map_or((bytes.as_slice(), [].as_slice()), |i| {
            (&bytes[..i], &bytes[i + 1..])
        });
    let paths: Vec<_> = rest
        .split(|b| *b == 0)
        .map(|path| path.strip_prefix(b"\n").unwrap_or(path))
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect();
    Ok(format!(
        "{}\n{}\nChanged paths: {}\nDiff:\n{}",
        handle.to_string_lossy(),
        String::from_utf8_lossy(body).trim_end(),
        paths.join(", "),
        commit_diff(handle, env)
    ))
}

/// The diffstat and the start of the patch of one commit, clipped to `DIFF_CHARS`. A subject
/// says what a commit claims; the patch says what it did, and the two disagree often enough
/// that the finals round cannot decide on the claim alone. The stat comes first, so a patch
/// too long for the budget still leaves every changed file and its line counts in view.
/// Unreadable output is an empty diff, never an error: the rest of the evidence still stands.
fn commit_diff(handle: &OsStr, env: &Env) -> String {
    let bytes = run_lister_blocking(
        &[
            "git".into(),
            "show".into(),
            "--format=".into(),
            "--no-color".into(),
            "--no-ext-diff".into(),
            "--no-renames".into(),
            "--unified=0".into(),
            "--stat=100".into(),
            "--patch".into(),
            "--end-of-options".into(),
            handle.to_owned(),
            "--".into(),
        ],
        env,
        OUTPUT_CAP,
    )
    .unwrap_or_default();
    crate::tournament::clip(
        String::from_utf8_lossy(&bytes).trim_matches(['\n', ' ']),
        DIFF_CHARS,
    )
}

/// The directory a literal prefix names, relative to `env.cwd` (empty for none): the whole
/// text when it ends with `/` and names a directory, else the part after the first `=`. A
/// literal that does not end with `/` is not a prefix.
fn resolve_prefix(prefix: Option<&Path>, env: &Env) -> Result<PathBuf, JevifyError> {
    let Some(prefix) = prefix else {
        return Ok(PathBuf::new());
    };
    let text = prefix.as_os_str().as_bytes();
    if !text.ends_with(b"/") {
        return Ok(PathBuf::new());
    }
    let after_equals = text.iter().position(|b| *b == b'=').map(|i| &text[i + 1..]);
    for candidate in std::iter::once(text).chain(after_equals) {
        if env.cwd.join(OsStr::from_bytes(candidate)).is_dir() {
            return Ok(PathBuf::from(OsStr::from_bytes(candidate)));
        }
    }
    Err(JevifyError::lister_failed(format!(
        "prefix {} names no directory",
        prefix.display()
    )))
}

/// The most children a `dir` finalist names; the rest is a count.
const DIR_CHILDREN: usize = 24;

/// Tier-two evidence of one `dir` finalist: the names of its first children in name order,
/// subdirectories marked with `/`, `.git` left out, and whether the policy withheld it. A
/// directory whose path has a withheld component or that is a symbolic link is withheld and
/// carries no evidence; one that cannot be read carries none and is not withheld.
fn dir_children(root: &Path, handle: &Path) -> (String, bool) {
    if crate::records::withheld(handle) {
        return (String::new(), true);
    }
    let path = root.join(handle);
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_symlink() => return (String::new(), true),
        Ok(metadata) if metadata.is_dir() => {}
        _ => return (String::new(), false),
    }
    let Ok(entries) = std::fs::read_dir(&path) else {
        return (String::new(), false);
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != ".git")
        .map(|entry| {
            let name: String = entry
                .file_name()
                .to_string_lossy()
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if is_dir { format!("{name}/") } else { name }
        })
        .collect();
    names.sort();
    let total = names.len();
    let mut text = format!(
        "{} entries: {}",
        total,
        names[..total.min(DIR_CHILDREN)].join(", ")
    );
    if total > DIR_CHILDREN {
        text.push_str(&format!(", +{} more", total - DIR_CHILDREN));
    }
    (crate::input::redact(&text), false)
}

/// A lister failure that means "no repository here", where a walk lists instead.
fn outside_work_tree(error: &JevifyError) -> bool {
    let text = error.to_string();
    text.contains("not a git repository") || text.contains("os error 2)")
}

/// Every file under `root`, relative, without following symlinks and without `.git`.
fn walk(root: &Path) -> Result<Vec<Vec<u8>>, JevifyError> {
    let mut stack = vec![(root.to_path_buf(), Vec::new())];
    let mut found = Vec::new();
    while let Some((dir, relative)) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| JevifyError::lister_failed(format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| JevifyError::lister_failed(format!("{}: {e}", dir.display())))?;
            let name = entry.file_name();
            let mut path = relative.clone();
            if !path.is_empty() {
                path.push(b'/');
            }
            path.extend_from_slice(name.as_bytes());
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if name != ".git" {
                    stack.push((entry.path(), path));
                }
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// `file` and `dir`: tracked and untracked files under the prefix, hidden ones included,
/// ignored ones and `.git/` excluded; outside a work tree, a no-follow walk. Handles are paths
/// relative to the prefix; `dir` lists the directories those paths lie in.
fn paths(
    prefix: Option<&Path>,
    dirs: bool,
    limit: usize,
    env: &Env,
) -> Result<Listing, JevifyError> {
    let relative = resolve_prefix(prefix, env)?;
    let scoped = Env {
        cwd: env.cwd.join(&relative),
        ..env.clone()
    };
    let files = match run_lister_blocking(&FILE_ARGV.map(OsString::from), &scoped, OUTPUT_CAP) {
        Ok(bytes) => bytes
            .split(|b| *b == 0)
            .filter(|path| !path.is_empty())
            .map(<[u8]>::to_vec)
            .collect(),
        Err(error) if outside_work_tree(&error) => walk(&scoped.cwd)?,
        Err(error) => return Err(error),
    };
    let handles: Vec<Vec<u8>> = if dirs {
        let mut set = BTreeSet::new();
        for path in &files {
            for (i, b) in path.iter().enumerate() {
                if *b == b'/' && i > 0 {
                    set.insert(path[..i].to_vec());
                }
            }
        }
        set.into_iter().collect()
    } else {
        files
    };
    let records = handles
        .into_iter()
        .map(|path| Record {
            evidence: String::from_utf8_lossy(&path).into_owned(),
            handle: OsString::from_vec(path),
            raw: 0..0,
        })
        .collect();
    Ok(listing(records, 0, false, limit))
}

/// `tool`: the inventory of the injected PATH. Names the cap dropped count in `omitted` and in
/// `total`, so the listing never presents a capped list as complete.
fn tools(env: &Env) -> Result<Listing, JevifyError> {
    let inventory = crate::inventory::load_with(&env.path, env.cache_dir.as_deref(), env.deadline)?;
    let records = inventory
        .tools
        .into_iter()
        .map(|tool| Record {
            evidence: format!("{}: {}", tool.name, tool.summary),
            handle: tool.name.into(),
            raw: 0..0,
        })
        .collect();
    let mut listing = listing(records, 0, false, usize::MAX);
    listing.total += inventory.omitted;
    listing.omitted += inventory.omitted;
    Ok(listing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    fn scratch() -> PathBuf {
        tempfile::Builder::new()
            .prefix("jevify-source-")
            .tempdir()
            .unwrap()
            .keep()
            .canonicalize()
            .unwrap()
    }

    fn environment(cwd: &Path) -> Env {
        Env {
            path: "/usr/bin:/bin".into(),
            cwd: cwd.into(),
            config_dir: None,
            cache_dir: None,
            deadline: Instant::now() + LISTER_TIMEOUT,
        }
    }

    fn fake_git(body: &str) -> Env {
        let dir = scratch();
        let git = dir.join("git");
        fs::write(&git, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
        Env {
            path: dir.as_os_str().to_owned(),
            ..environment(&dir)
        }
    }

    fn input(bytes: &[u8], split: Split, field: Option<usize>, key: Option<&str>) -> Scope {
        Scope::Input {
            bytes: bytes.to_vec(),
            split,
            field,
            key: key.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn input_records_preserve_bytes_and_ranges_and_ignore_unordered_limit() {
        let bytes = b"first\nfirst\n-option\n\xff\n";
        let env = environment(Path::new("."));
        let result = enumerate("-", input(bytes, Split::Lines, None, None), 1, &env)
            .await
            .unwrap();
        assert_eq!(result.total, 3);
        assert_eq!(result.omitted, 0);
        assert!(!result.ordered);
        assert_eq!(result.records.len(), 3);
        assert_eq!(result.records[0].handle, "first");
        assert_eq!(&bytes[result.records[0].raw.clone()], b"first\n");
        assert_eq!(result.records[1].handle, "-option");
        assert_eq!(result.records[2].handle.as_bytes(), b"\xff");
        assert!(
            enumerate("-", input(b"", Split::Lines, None, None), 1, &env)
                .await
                .unwrap()
                .records
                .is_empty()
        );
        let result = enumerate(
            "-",
            input(b"a\0bad\nname\0bad\rname\0", Split::Nul, None, None),
            10,
            &env,
        )
        .await
        .unwrap();
        assert_eq!((result.total, result.omitted), (1, 2));
        let result = enumerate(
            "-",
            input(b"bad\0name\ngood\n", Split::Lines, None, None),
            10,
            &env,
        )
        .await
        .unwrap();
        assert_eq!((result.total, result.omitted), (1, 1));
    }

    #[tokio::test]
    async fn field_and_json_handles_keep_whole_record_evidence() {
        let env = environment(Path::new("."));
        let bytes = b"1 first choice\n2 second choice\nshort\n";
        let result = enumerate("-", input(bytes, Split::Lines, Some(2), None), 10, &env)
            .await
            .unwrap();
        assert_eq!(result.records[0].handle, "first");
        assert_eq!(result.records[0].evidence, "1 first choice");
        assert_eq!((result.total, result.omitted), (2, 1));
        for bytes in [
            b"{\"id\":1,\"title\":\"first\"}\n{\"id\":2,\"title\":\"second\"}\n".as_slice(),
            b"[{\"id\":1,\"title\":\"first\"},{\"id\":2,\"title\":\"second\"}]".as_slice(),
        ] {
            let result = enumerate("-", input(bytes, Split::Lines, None, Some("id")), 10, &env)
                .await
                .unwrap();
            assert_eq!(result.records[0].handle, "1");
            assert_eq!(result.records[1].handle, "2");
            assert!(result.records[0].evidence.contains("first"));
            assert!(bytes[result.records[0].raw.clone()].starts_with(b"{"));
        }
        let result = enumerate("-", input(br#"[{"id":"bad\nname"},{"id":"bad\rname"},{"id":"bad\u0000name"},{"id":"-ok"},{}]"#, Split::Lines, None, Some("id")), 10, &env).await.unwrap();
        assert_eq!((result.total, result.omitted), (1, 4));
        assert_eq!(result.records[0].handle, "-ok");
        for scope in [
            input(b"x", Split::Lines, Some(0), None),
            input(b"x", Split::Lines, Some(1), Some("id")),
            Scope::Prefix(None),
        ] {
            assert_eq!(
                enumerate("-", scope, 10, &env).await.unwrap_err().kind(),
                "usage"
            );
        }
        assert_eq!(
            enumerate(
                "-",
                input(b"invalid json", Split::Lines, None, Some("id")),
                10,
                &env
            )
            .await
            .unwrap_err()
            .kind(),
            "input"
        );
        assert_eq!(
            enumerate("commit", Scope::Prefix(Some("src".into())), 10, &env)
                .await
                .unwrap_err()
                .kind(),
            "usage"
        );
        assert_eq!(
            enumerate("unknown", Scope::Prefix(None), 10, &env)
                .await
                .unwrap_err()
                .kind(),
            "usage"
        );
        assert_eq!(
            enrich_with_env("-", &["x".into(), "y".into()], &env).await,
            ["", ""]
        );
    }

    #[test]
    fn deduplication_uses_handle_and_evidence_and_age_is_computed() {
        let record = |handle: &str, evidence: &str| Record {
            handle: handle.into(),
            evidence: evidence.into(),
            raw: 0..0,
        };
        let result = listing(
            vec![
                record("a", "same"),
                record("b", "same"),
                record("a", "same"),
                record("a", "different"),
            ],
            0,
            false,
            1,
        );
        assert_eq!(result.total, 3);
        assert_eq!(age(172800, 0), "2 days ago");
        assert_eq!(age(3600, 0), "1 hour ago");
        assert_eq!(age(60, 0), "1 minute ago");
        assert_eq!(age(0, 10), "0 seconds ago");
        assert_eq!(REGISTRY.iter().map(|k| &k.name).collect::<Vec<_>>(), KINDS);
        assert!(kind("branch").unwrap().ordered);
        assert!(kind("branch").unwrap().has_tier_two);
        assert!(kind("-").unwrap().coded);
        assert!(!kind("-").unwrap().path_kind);
        assert!(kind("unknown").is_none());
    }

    fn git(dir: &Path, args: &[&str], timestamp: u64) -> Vec<u8> {
        let output = Command::new("/usr/bin/git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .env("GIT_AUTHOR_DATE", format!("@{timestamp} +0000"))
            .env("GIT_COMMITTER_DATE", format!("@{timestamp} +0000"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    #[tokio::test]
    async fn real_branches_fold_twins_order_limit_and_enrich_last_five() {
        let dir = scratch();
        git(&dir, &["init", "--initial-branch=main"], 1700000000);
        let mut commits = Vec::new();
        for i in 0..6 {
            let top = if i == 0 { "sixth-only" } else { "recent" };
            fs::create_dir_all(dir.join(top)).unwrap();
            let file = format!("{top}/file-{i}");
            fs::write(dir.join(&file), format!("{i}\n")).unwrap();
            git(&dir, &["add", "--", &file], 1700000000);
            git(
                &dir,
                &[
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-m",
                    &format!("subject-{i}"),
                ],
                1700000000 + i * 86400,
            );
            commits.push(
                String::from_utf8(git(&dir, &["rev-parse", "HEAD"], 1700000000))
                    .unwrap()
                    .trim()
                    .to_owned(),
            );
        }
        for (name, commit) in [
            ("refs/heads/older", &commits[1]),
            ("refs/heads/middle", &commits[3]),
            // A local branch whose name looks like a remote ref.
            ("refs/heads/origin/x", &commits[2]),
            ("refs/remotes/origin/remote-only", &commits[4]),
            ("refs/remotes/upstream/ancient", &commits[0]),
            ("refs/remotes/origin/ancient", &commits[0]),
            ("refs/remotes/origin/main", &commits[5]),
        ] {
            git(&dir, &["update-ref", name, commit], 1700000000);
        }
        git(
            &dir,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
            1700000000,
        );
        let env = environment(&dir);
        let result = enumerate("branch", Scope::Prefix(None), 3, &env)
            .await
            .unwrap();
        assert_eq!((result.total, result.omitted, result.ordered), (7, 0, true));
        assert_eq!(
            result
                .records
                .iter()
                .map(|r| r.handle.as_os_str())
                .collect::<Vec<_>>(),
            [
                OsStr::new("main"),
                OsStr::new("remote-only"),
                OsStr::new("middle")
            ]
        );
        for record in &result.records {
            assert_eq!(record.raw, 0..0);
        }
        assert!(result.records[0].evidence.contains("subject-5"));
        assert!(result.records[0].evidence.contains("days ago"));
        // The short name is the handle; the evidence keeps the remote-tracking ref.
        assert!(
            result.records[1]
                .evidence
                .starts_with("origin/remote-only — subject-4"),
            "{}",
            result.records[1].evidence
        );
        // Two remotes track `ancient`: git refuses the short name, so the refs stay qualified.
        let all = enumerate("branch", Scope::Prefix(None), 10, &env)
            .await
            .unwrap();
        let handles: HashSet<_> = all.records.iter().map(|r| r.handle.clone()).collect();
        assert!(
            handles.contains(OsStr::new("origin/ancient")),
            "{handles:?}"
        );
        assert!(
            handles.contains(OsStr::new("upstream/ancient")),
            "{handles:?}"
        );
        assert!(!handles.contains(OsStr::new("ancient")), "{handles:?}");
        // The bare marker lists the local `origin/x` by its name, next to the remote-only `y`.
        assert!(handles.contains(OsStr::new("origin/x")), "{handles:?}");
        assert!(handles.contains(OsStr::new("remote-only")), "{handles:?}");
        // A literal prefix names a remote: its refs, by the rest of their name, unfolded, so
        // the argument `origin/<handle>` is a rev that `git log` resolves. The local branch
        // `origin/x` is not under `refs/remotes/origin/` and stays out.
        let origin = enumerate("branch", Scope::Prefix(Some("origin/".into())), 10, &env)
            .await
            .unwrap();
        assert_eq!(
            origin
                .records
                .iter()
                .map(|r| r.handle.as_os_str())
                .collect::<Vec<_>>(),
            [
                OsStr::new("main"),
                OsStr::new("remote-only"),
                OsStr::new("ancient")
            ]
        );
        assert!(
            origin.records[2]
                .evidence
                .starts_with("origin/ancient — subject-0"),
            "{}",
            origin.records[2].evidence
        );
        // A prefix under which no remote ref lives fails and names the remotes that exist.
        let none = enumerate("branch", Scope::Prefix(Some("nothing/".into())), 10, &env)
            .await
            .unwrap_err();
        assert_eq!(none.kind(), "lister_failed");
        assert_eq!(
            none.to_string(),
            "prefix nothing/ names no remote ref; remotes: origin, upstream"
        );
        // A deeper prefix is a literal under `refs/remotes/`.
        let deep = enumerate(
            "branch",
            Scope::Prefix(Some("origin/remote-".into())),
            10,
            &env,
        )
        .await
        .unwrap();
        assert_eq!(
            deep.records
                .iter()
                .map(|r| r.handle.as_os_str())
                .collect::<Vec<_>>(),
            [OsStr::new("only")]
        );
        let (prefixed, withheld) = enrich_in(
            "branch",
            Path::new("origin/"),
            &["remote-only".into()],
            &env,
        )
        .await;
        assert_eq!(withheld, 0);
        assert!(prefixed[0].contains("subject-4"), "{}", prefixed[0]);
        let remote = enrich_with_env("branch", &["remote-only".into()], &env).await;
        assert!(remote[0].starts_with("remote-only\n"), "{}", remote[0]);
        assert!(remote[0].contains("subject-4"), "{}", remote[0]);
        assert!(!remote[0].contains("subject-5"), "{}", remote[0]);
        assert!(
            enumerate("branch", Scope::Prefix(None), 0, &env)
                .await
                .unwrap()
                .records
                .is_empty()
        );
        let evidence = enrich_with_env("branch", &["main".into()], &env).await;
        for i in 1..6 {
            assert!(
                evidence[0].contains(&format!("subject-{i}")),
                "{}",
                evidence[0]
            );
        }
        assert!(!evidence[0].contains("subject-0"));
        assert!(evidence[0].contains("Changed top-level paths: recent"));
        assert!(!evidence[0].contains("sixth-only"));
        let outside = environment(&scratch());
        let error = enumerate("branch", Scope::Prefix(None), 3, &outside)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "lister_failed");
        assert!(error.to_string().contains("not a git repository"));
    }

    #[tokio::test]
    async fn only_finalists_get_tier_two_and_keep_their_order() {
        let env = fake_git(
            r#"
printf '%s\n' "$*" >> calls
if [ "$1" = for-each-ref ]; then
    i=50
    while [ "$i" -gt 0 ]; do
        printf 'refs/heads/b%s\000\000%s\000tip-%s\000\n' "$i" "$i" "$i"
        i=$((i - 1))
    done
else
    for arg in "$@"; do
        case "$arg" in b*) printf '\000subject-%s\000\000\npath/file\000' "$arg";; esac
    done
fi
"#,
        );
        let result = enumerate("branch", Scope::Prefix(None), 50, &env)
            .await
            .unwrap();
        assert_eq!(result.total, 50);
        assert_eq!(
            fs::read_to_string(env.cwd.join("calls"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        let evidence =
            enrich_with_env("branch", &["b9".into(), "b45".into(), "b2".into()], &env).await;
        for (value, name) in evidence.iter().zip(["b9", "b45", "b2"]) {
            assert!(value.contains(&format!("subject-{name}")), "{value}");
        }
        let calls = fs::read_to_string(env.cwd.join("calls")).unwrap();
        assert_eq!(calls.lines().count(), 4);
        for (call, name) in calls.lines().skip(1).zip(["b9", "b45", "b2"]) {
            assert!(call.ends_with(&format!("{name} --")));
        }
    }

    #[tokio::test]
    async fn runner_drains_both_pipes_and_injects_environment_and_null_stdin() {
        let mut env = fake_git(
            r#"
[ "$GH_PROMPT_DISABLED" = 1 ] && [ "$GIT_TERMINAL_PROMPT" = 0 ] && [ "$NO_COLOR" = 1 ] || exit 2
[ "$JEVIFY_CONFIG_DIR" = "$PWD/config" ] || exit 3
if read -r line; then exit 4; fi
i=0
while [ "$i" -lt 12000 ]; do
    printf 'stdout stream\n'
    printf 'stderr stream\n' >&2
    i=$((i + 1))
done
"#,
        );
        env.config_dir = Some(env.cwd.join("config"));
        let result = run_lister(&["git".into()], &env).await.unwrap();
        assert_eq!(result.len(), 12000 * b"stdout stream\n".len());
    }

    #[tokio::test]
    async fn runner_deadline_kills_and_does_not_block_runtime() {
        let mut env = fake_git("exec /bin/sleep 2");
        env.deadline = Instant::now() + Duration::from_millis(100);
        let start = Instant::now();
        let argv = ["git".into()];
        let (result, ticks) = tokio::join!(run_lister(&argv, &env), async {
            let mut ticks = 0;
            while start.elapsed() < Duration::from_millis(80) {
                tokio::time::sleep(Duration::from_millis(5)).await;
                ticks += 1;
            }
            ticks
        });
        assert!(ticks >= 2);
        assert!(start.elapsed() < Duration::from_millis(700));
        let error = result.unwrap_err();
        assert_eq!(error.kind(), "lister_failed");
        assert!(error.to_string().contains("deadline"));
    }

    #[tokio::test]
    async fn runner_discards_partial_output_after_exit_or_inherited_pipe() {
        // The second body exits 0 while a background child keeps the pipes open for
        // 5 s. Had the runner waited for EOF instead of giving the readers READER_GRACE
        // after exit, the pipes would close with exit 0 and `enumerate` would succeed,
        // so `unwrap_err` plus the "EOF" message prove the early return without a
        // wall-clock bound that a loaded machine could miss.
        for body in [
            "printf 'refs/heads/x\\000\\0001\\000tip\\000\\n'; printf 'tool failed' >&2; exit 1",
            "printf 'refs/heads/x\\000\\0001\\000tip\\000\\n'; /bin/sleep 5 & exit 0",
        ] {
            let env = fake_git(body);
            let start = Instant::now();
            let error = enumerate("branch", Scope::Prefix(None), 10, &env)
                .await
                .unwrap_err();
            assert_eq!(error.kind(), "lister_failed");
            assert!(start.elapsed() < Duration::from_secs(5));
            if body.contains("exit 1") {
                assert!(error.to_string().contains("tool failed"));
            } else {
                assert!(error.to_string().contains("EOF"));
            }
        }
    }

    /// A scratch directory holding executable `name` (first on the injected PATH) and a
    /// configuration directory `config` whose `kinds.jsonl` is `recipes`.
    fn fake_tool(name: &str, body: &str, recipes: Option<&str>) -> Env {
        let dir = scratch();
        let tool = dir.join(name);
        fs::write(&tool, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(dir.join("config")).unwrap();
        if let Some(recipes) = recipes {
            fs::write(dir.join("config").join("kinds.jsonl"), recipes).unwrap();
        }
        Env {
            path: format!("{}:/usr/bin:/bin", dir.display()).into(),
            config_dir: Some(dir.join("config")),
            ..environment(&dir)
        }
    }

    fn handles(listing: &Listing) -> Vec<&OsStr> {
        listing
            .records
            .iter()
            .map(|r| r.handle.as_os_str())
            .collect()
    }

    #[test]
    fn shipped_recipes_parse_are_unique_and_match_the_registry() {
        assert_eq!(SHIPPED.lines().count(), 7);
        let recipes = shipped();
        assert_eq!(recipes.len(), 7);
        let mut names = HashSet::new();
        for recipe in recipes {
            assert!(valid_kind_name(&recipe.kind), "{}", recipe.kind);
            assert!(names.insert(recipe.kind.as_str()), "{}", recipe.kind);
            let kind = kind(&recipe.kind).unwrap();
            assert!(!kind.coded && !kind.path_kind && !kind.has_tier_two);
            assert_eq!(kind.ordered, recipe.ordered, "{}", recipe.kind);
        }
        let coded: Vec<_> = REGISTRY
            .iter()
            .filter(|k| k.coded)
            .map(|k| &k.name)
            .collect();
        assert_eq!(coded, ["-", "branch", "commit", "file", "dir", "tool"]);
        assert!(kind("commit").unwrap().ordered && kind("commit").unwrap().has_tier_two);
        assert!(kind("branch").unwrap().path_kind);
        assert!(kind("file").unwrap().path_kind && kind("file").unwrap().has_tier_two);
        assert!(kind("dir").unwrap().path_kind && kind("dir").unwrap().has_tier_two);
        assert!(!kind("tool").unwrap().ordered && !kind("tool").unwrap().has_tier_two);
        let shipped_names: Vec<_> = recipes.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(&KINDS[coded.len()..], shipped_names.as_slice());
        assert_eq!(
            shipped_names,
            [
                "pr",
                "issue",
                "ci-run",
                "stash",
                "process",
                "container",
                "pod"
            ]
        );
        for name in ["", "Pr", "-x", "1pr", "pr_x", "pr x"] {
            assert!(!valid_kind_name(name), "{name}");
        }
        assert!(valid_kind_name("ci-run") && valid_kind_name("x-"));
    }

    #[tokio::test]
    async fn a_bad_user_line_is_recipe_invalid_with_its_number_for_any_user_kind() {
        let good = r#"{"kind":"widget","list":["widget"]}"#;
        for bad in [
            r#"{"kind":"gadget","list":["gadget"],"evidence":"x"}"#,
            r#"{"kind":"gadget","list":["gadget"],"field":1,"key":"id"}"#,
            r#"{"kind":"gadget","list":[]}"#,
            r#"{"kind":"Gadget","list":["gadget"]}"#,
            r#"{"kind":"gadget","list":["gadget"]"#,
        ] {
            let env = fake_tool("widget", "echo w", Some(&format!("{good}\n\n{bad}\n")));
            for name in ["widget", "gadget", "other"] {
                let error = lookup(name, &env).unwrap_err();
                assert_eq!(error.kind(), "recipe_invalid", "{bad}");
                assert_eq!(error.exit().code(), 6);
                assert!(error.to_string().contains("line 3"), "{error}");
            }
            let error = enumerate("widget", Scope::Prefix(None), 10, &env)
                .await
                .unwrap_err();
            assert_eq!(error.kind(), "recipe_invalid");
            assert!(catalog(&env).error.unwrap().contains("line 3"));
        }
    }

    #[tokio::test]
    async fn shipped_and_coded_kinds_never_open_the_user_file() {
        // kinds.jsonl is a directory, and a config_dir that is a file: reading either fails.
        let in_place = fake_tool("gh", "printf '[{\"number\":7,\"title\":\"fix\"}]'", None);
        fs::create_dir(in_place.config_dir.as_ref().unwrap().join("kinds.jsonl")).unwrap();
        let not_a_dir = Env {
            config_dir: Some(in_place.cwd.join("gh")),
            ..in_place.clone()
        };
        for env in [&in_place, &not_a_dir] {
            for name in ["-", "branch", "pr", "pod"] {
                assert_eq!(lookup(name, env).unwrap().unwrap().name, name);
            }
            let listing = enumerate("pr", Scope::Prefix(None), 10, env).await.unwrap();
            assert_eq!(handles(&listing), ["7"]);
            assert_eq!(lookup("widget", env).unwrap_err().kind(), "recipe_invalid");
            let catalog = catalog(env);
            assert_eq!(catalog.kinds.len(), KINDS.len());
            assert!(catalog.error.is_some());
        }
    }

    #[tokio::test]
    async fn user_recipes_resolve_but_never_shadow_and_never_come_from_the_cwd() {
        for shadow in ["branch", "pr"] {
            let env = fake_tool(
                "widget",
                "echo w",
                Some(&format!(
                    "{{\"kind\":\"widget\",\"list\":[\"widget\"]}}\n{{\"kind\":\"{shadow}\",\"list\":[\"x\"]}}\n"
                )),
            );
            let error = lookup("widget", &env).unwrap_err();
            assert_eq!(error.kind(), "recipe_invalid");
            assert!(error.to_string().contains("line 2"), "{error}");
            // The shipped kind itself is unaffected: the file is not read.
            assert!(lookup(shadow, &env).unwrap().unwrap().ordered);
        }
        let twice = fake_tool(
            "widget",
            "echo w",
            Some(
                "{\"kind\":\"widget\",\"list\":[\"a\"]}\n{\"kind\":\"widget\",\"list\":[\"b\"]}\n",
            ),
        );
        assert!(
            lookup("widget", &twice)
                .unwrap_err()
                .to_string()
                .contains("line 2")
        );
        let env = fake_tool(
            "widget",
            "[ \"$1\" = --all ] || exit 9\nprintf 'w1 first widget\\nw2 second widget\\n'",
            Some("{\"kind\":\"widget\",\"list\":[\"widget\",\"--all\"],\"field\":1}\n"),
        );
        let widget = lookup("widget", &env).unwrap().unwrap();
        assert_eq!(widget.name, "widget");
        assert!(!widget.coded && !widget.ordered);
        // The const registry never sees a user recipe.
        assert!(kind("widget").is_none());
        assert!(!KINDS.contains(&"widget"));
        let listing = enumerate("widget", Scope::Prefix(None), 1, &env)
            .await
            .unwrap();
        assert_eq!(handles(&listing), ["w1", "w2"]);
        assert_eq!(listing.records[1].evidence, "w2 second widget");
        assert_eq!((listing.total, listing.ordered), (2, false));
        let catalog = catalog(&env);
        assert_eq!(catalog.error, None);
        let last = catalog.kinds.last().unwrap();
        assert_eq!(
            (last.name.as_str(), last.origin, last.list.as_slice()),
            (
                "widget",
                "user",
                ["widget".to_owned(), "--all".to_owned()].as_slice()
            )
        );
        assert_eq!(catalog.kinds[1].list[0], "git");
        // A kinds.jsonl in the working directory is never read.
        let cwd = fake_tool("widget", "echo w", None);
        fs::write(
            cwd.cwd.join("kinds.jsonl"),
            "{\"kind\":\"widget\",\"list\":[\"widget\"]}\n",
        )
        .unwrap();
        assert!(lookup("widget", &cwd).unwrap().is_none());
        assert_eq!(
            enumerate("widget", Scope::Prefix(None), 10, &cwd)
                .await
                .unwrap_err()
                .kind(),
            "usage"
        );
        let unset = Env {
            config_dir: None,
            ..cwd
        };
        assert!(lookup("widget", &unset).unwrap().is_none());
    }

    #[tokio::test]
    async fn recipe_handles_key_field_whole_line_and_ordered_limit() {
        let recipes = [
            r#"{"kind":"lines","list":["tool","lines"],"key":"id","ordered":true}"#,
            r#"{"kind":"array","list":["tool","array"],"key":"id"}"#,
            r#"{"kind":"fields","list":["tool","fields"],"field":2}"#,
            r#"{"kind":"whole","list":["tool","whole"],"ordered":true}"#,
        ]
        .join("\n");
        let env = fake_tool(
            "tool",
            r#"case "$1" in
lines) printf '{"id":3,"title":"c"}\n{"id":2,"title":"b"}\n{"id":1,"title":"a"}\n';;
array) printf '[{"id":"x","t":"one"},{"id":"y","t":"two"}]';;
fields) printf 'a b c\nd e f\nshort\n';;
whole) printf 'newest line\nolder line\noldest line\n';;
esac"#,
            Some(&recipes),
        );
        let lines = enumerate("lines", Scope::Prefix(None), 2, &env)
            .await
            .unwrap();
        assert_eq!(handles(&lines), ["3", "2"]);
        assert_eq!((lines.total, lines.ordered), (3, true));
        assert!(lines.records[0].evidence.contains("\"title\":\"c\""));
        let array = enumerate("array", Scope::Prefix(None), 1, &env)
            .await
            .unwrap();
        assert_eq!(handles(&array), ["x", "y"]);
        assert_eq!((array.total, array.ordered), (2, false));
        assert!(array.records[1].evidence.contains("two"));
        let fields = enumerate("fields", Scope::Prefix(None), 10, &env)
            .await
            .unwrap();
        assert_eq!(handles(&fields), ["b", "e"]);
        assert_eq!(fields.records[0].evidence, "a b c");
        assert_eq!(fields.omitted, 1);
        let whole = enumerate("whole", Scope::Prefix(None), 1, &env)
            .await
            .unwrap();
        assert_eq!(handles(&whole), ["newest line"]);
        assert_eq!((whole.total, whole.ordered), (3, true));
    }

    #[tokio::test]
    async fn a_failing_or_garbled_lister_is_lister_failed_with_its_text() {
        let env = fake_tool(
            "gh",
            "echo 'To get started with GitHub CLI, please run: gh auth login' >&2\necho 'not logged in' >&2\nexit 1",
            None,
        );
        for name in ["pr", "issue", "ci-run"] {
            let error = enumerate(name, Scope::Prefix(None), 10, &env)
                .await
                .unwrap_err();
            assert_eq!(error.kind(), "lister_failed");
            assert!(error.to_string().contains("not logged in"), "{error}");
        }
        let missing = Env {
            path: "/nonexistent".into(),
            ..env
        };
        assert_eq!(
            enumerate("pr", Scope::Prefix(None), 10, &missing)
                .await
                .unwrap_err()
                .kind(),
            "lister_failed"
        );
        let garbled = fake_tool("gh", "echo 'not json'", None);
        let error = enumerate("pr", Scope::Prefix(None), 10, &garbled)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "lister_failed");
        assert!(error.to_string().contains("gh"));
    }

    fn commit(dir: &Path, subject: &str, body: &str, timestamp: u64) -> String {
        git(dir, &["add", "-A"], timestamp);
        git(
            dir,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                subject,
                "-m",
                body,
            ],
            timestamp,
        );
        String::from_utf8(git(dir, &["rev-parse", "HEAD"], timestamp))
            .unwrap()
            .trim()
            .to_owned()
    }

    #[tokio::test]
    async fn commits_list_newest_first_with_exact_total_and_tier_two_bodies() {
        let dir = scratch();
        git(&dir, &["init", "--initial-branch=main"], 1700000000);
        let env = environment(&dir);
        let error = enumerate("commit", Scope::Prefix(None), 10, &env)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "lister_failed");
        assert!(error.to_string().contains("git"), "{error}");
        let mut oids = Vec::new();
        for i in 0..5 {
            fs::create_dir_all(dir.join("src")).unwrap();
            fs::write(dir.join("src").join(format!("f{i}.rs")), format!("{i}\n")).unwrap();
            oids.push(commit(
                &dir,
                &format!("subject-{i}"),
                &format!("body-{i} line one\n\nbody-{i} line three"),
                1700000000 + i * 60,
            ));
            if i == 2 {
                let listing = enumerate("commit", Scope::Prefix(None), 10, &env)
                    .await
                    .unwrap();
                assert_eq!(
                    (listing.total, listing.records.len(), listing.ordered),
                    (3, 3, true)
                );
            }
        }
        let listing = enumerate("commit", Scope::Prefix(None), 3, &env)
            .await
            .unwrap();
        assert_eq!((listing.total, listing.records.len()), (5, 3));
        assert_eq!(
            handles(&listing),
            [oids[4].as_str(), oids[3].as_str(), oids[2].as_str()]
        );
        assert_eq!(oids[4].len(), 40);
        assert_eq!(listing.records[0].evidence, "subject-4");
        assert!(!listing.records[0].evidence.contains("body"));
        let (evidence, withheld) = enrich_in(
            "commit",
            Path::new("ignored/"),
            &[oids[1].clone().into(), oids[4].clone().into()],
            &env,
        )
        .await;
        assert_eq!(withheld, 0);
        assert!(
            evidence[0].contains("body-1 line one\n\nbody-1 line three"),
            "{}",
            evidence[0]
        );
        assert!(
            evidence[0].contains("Changed paths: src/f1.rs"),
            "{}",
            evidence[0]
        );
        assert!(evidence[1].starts_with(&oids[4]));
        assert!(evidence[1].contains("src/f4.rs"));
        assert_eq!(
            enumerate("commit", Scope::Prefix(Some("src/".into())), 3, &env)
                .await
                .unwrap_err()
                .kind(),
            "usage"
        );
        // `enrich` keeps its signature: process environment, no prefix.
        assert_eq!(enrich("branch", &["HEAD".into()]).await.len(), 1);
    }

    #[tokio::test]
    async fn commit_tier_two_runs_for_finalists_only() {
        let env = fake_git(
            r#"
printf '%s\n' "$*" >> calls
case "$1 $2" in
"log -n") i=0; while [ "$i" -lt 200 ]; do printf 'oid%s\000subject %s\000' "$i" "$i"; i=$((i + 1)); done;;
"log -1") printf 'body of %s\000\000\nsrc/a.rs\000src/b.rs\000' "$9";;
"show --format=") printf ' src/a.rs | 2 +-\n@@ -1 +1 @@ diff of %s\n' "${10}";;
"rev-list --count") echo 200;;
*) exit 9;;
esac
"#,
        );
        let listing = enumerate("commit", Scope::Prefix(None), 100, &env)
            .await
            .unwrap();
        assert_eq!((listing.total, listing.records.len()), (200, 100));
        assert_eq!(listing.records[0].handle, "oid0");
        assert_eq!(listing.records[0].evidence, "subject 0");
        let calls = fs::read_to_string(env.cwd.join("calls")).unwrap();
        assert_eq!(calls.lines().count(), 2);
        assert!(calls.contains("log -n 100 -z"));
        let (evidence, withheld) = enrich_in(
            "commit",
            Path::new(""),
            &["oid7".into(), "oid150".into(), "oid2".into()],
            &env,
        )
        .await;
        assert_eq!(withheld, 0);
        for (value, name) in evidence.iter().zip(["oid7", "oid150", "oid2"]) {
            assert_eq!(
                value,
                &format!(
                    "{name}\nbody of {name}\nChanged paths: src/a.rs, src/b.rs\n\
                     Diff:\nsrc/a.rs | 2 +-\n@@ -1 +1 @@ diff of {name}"
                )
            );
        }
        // Two calls per finalist: the body with its paths, then the diff.
        let calls = fs::read_to_string(env.cwd.join("calls")).unwrap();
        assert_eq!(calls.lines().count(), 8);
        for (call, name) in calls.lines().skip(2).zip(
            ["oid7", "oid150", "oid2"]
                .into_iter()
                .flat_map(|name| [name, name]),
        ) {
            assert!(
                call.ends_with(&format!("--end-of-options {name} --")),
                "{call}"
            );
        }
    }

    /// A diff longer than the per-finalist budget keeps its stat and loses its tail.
    #[tokio::test]
    async fn commit_evidence_clips_a_long_diff() {
        let env = fake_git(
            r#"
case "$1 $2" in
"log -1") printf 'body\000\000\nsrc/a.rs\000';;
"show --format=") printf ' src/a.rs | 9999 +\n'; i=0; while [ "$i" -lt 400 ]; do printf '+a line of patch text that is long enough to matter\n'; i=$((i + 1)); done;;
*) exit 9;;
esac
"#,
        );
        let (evidence, _) = enrich_in("commit", Path::new(""), &["oid0".into()], &env).await;
        let diff = evidence[0].split_once("Diff:\n").unwrap().1;
        assert_eq!(diff.chars().count(), DIFF_CHARS);
        assert!(diff.starts_with("src/a.rs | 9999 +"));
        assert!(diff.ends_with('…'));
    }

    /// A git that cannot show the diff still yields the subject, body and paths.
    #[tokio::test]
    async fn commit_evidence_survives_a_failing_diff() {
        let env = fake_git(
            r#"
case "$1 $2" in
"log -1") printf 'body\000\000\nsrc/a.rs\000';;
*) exit 9;;
esac
"#,
        );
        let (evidence, _) = enrich_in("commit", Path::new(""), &["oid0".into()], &env).await;
        assert_eq!(evidence[0], "oid0\nbody\nChanged paths: src/a.rs\nDiff:\n");
    }

    /// A repository with hidden, ignored, untracked, secret and non-UTF-8 paths.
    fn tree() -> Env {
        let dir = scratch();
        git(&dir, &["init", "--initial-branch=main"], 1700000000);
        for (path, content) in [
            ("src/cmd/a.rs", "MARKER-A fn a() {}\n"),
            ("src/cmd/.hidden/b.txt", "SECRET-HIDDEN\n"),
            ("src/lib.rs", "MARKER-LIB pub mod cmd;\n"),
            (".npmrc", "SECRET-NPMRC\n"),
            (".env.local", "SECRET-ENV\n"),
            ("id_rsa", "SECRET-RSA\n"),
            ("x.pem", "SECRET-PEM\n"),
            ("a/.hidden/b.txt", "SECRET-A\n"),
            (".gitignore", "*.log\n"),
        ] {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, content).unwrap();
        }
        commit(&dir, "tree", "", 1700000000);
        fs::write(dir.join("ignored.log"), "MARKER-IGNORED\n").unwrap();
        fs::write(dir.join("src/cmd/new.rs"), "MARKER-NEW untracked\n").unwrap();
        // APFS refuses a name that is not UTF-8; the case runs where the file system allows it.
        let _ = fs::write(
            dir.join("src/cmd").join(OsStr::from_bytes(b"\xff.rs")),
            "MARKER-BYTES\n",
        );
        environment(&dir)
    }

    /// The expected handles, plus the non-UTF-8 one where the file system could create it.
    fn with_bytes(env: &Env, prefix: &str, mut expected: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        if env
            .cwd
            .join("src/cmd")
            .join(OsStr::from_bytes(b"\xff.rs"))
            .exists()
        {
            expected.push([prefix.as_bytes(), b"\xff.rs"].concat());
        }
        expected
    }

    fn sorted(listing: &Listing) -> Vec<Vec<u8>> {
        let mut handles: Vec<_> = listing
            .records
            .iter()
            .map(|r| r.handle.as_bytes().to_vec())
            .collect();
        handles.sort();
        handles
    }

    #[tokio::test]
    async fn files_and_dirs_honour_the_prefix_rule_and_list_hidden_but_not_ignored() {
        let env = tree();
        let scoped = enumerate("file", Scope::Prefix(Some("src/cmd/".into())), 1, &env)
            .await
            .unwrap();
        let expected = with_bytes(
            &env,
            "",
            vec![
                b".hidden/b.txt".to_vec(),
                b"a.rs".to_vec(),
                b"new.rs".to_vec(),
            ],
        );
        assert_eq!(sorted(&scoped), expected);
        assert_eq!(
            (scoped.total, scoped.omitted, scoped.ordered),
            (expected.len(), 0, false)
        );
        for record in &scoped.records {
            assert!(!record.evidence.contains("MARKER"), "{}", record.evidence);
            assert_eq!(record.raw, 0..0);
        }
        let option = enumerate(
            "file",
            Scope::Prefix(Some("--config=src/".into())),
            10,
            &env,
        )
        .await
        .unwrap();
        let mut expected = with_bytes(
            &env,
            "cmd/",
            vec![
                b"cmd/.hidden/b.txt".to_vec(),
                b"cmd/a.rs".to_vec(),
                b"cmd/new.rs".to_vec(),
            ],
        );
        expected.push(b"lib.rs".to_vec());
        assert_eq!(sorted(&option), expected);
        for prefix in ["nope/", "--config=nope/", "--config=src/lib.rs/"] {
            let error = enumerate("file", Scope::Prefix(Some(prefix.into())), 10, &env)
                .await
                .unwrap_err();
            assert_eq!(error.kind(), "lister_failed", "{prefix}");
            assert!(error.to_string().contains("names no directory"), "{error}");
        }
        // A literal that does not end with `/` is no prefix.
        let bare = enumerate("file", Scope::Prefix(Some("--config=".into())), 10, &env)
            .await
            .unwrap();
        let whole = enumerate("file", Scope::Prefix(None), 10, &env)
            .await
            .unwrap();
        assert_eq!(sorted(&bare), sorted(&whole));
        let names = sorted(&whole);
        for expected in [
            ".npmrc",
            ".env.local",
            "id_rsa",
            "x.pem",
            "a/.hidden/b.txt",
            ".gitignore",
            "src/cmd/new.rs",
        ] {
            assert!(names.contains(&expected.as_bytes().to_vec()), "{expected}");
        }
        assert!(!names.contains(&b"ignored.log".to_vec()));
        assert!(names.iter().all(|n| !n.starts_with(b".git/")));
        let dirs = enumerate("dir", Scope::Prefix(None), 10, &env)
            .await
            .unwrap();
        assert_eq!(
            sorted(&dirs),
            [
                b"a".to_vec(),
                b"a/.hidden".to_vec(),
                b"src".to_vec(),
                b"src/cmd".to_vec(),
                b"src/cmd/.hidden".to_vec()
            ]
        );
        let dirs = enumerate("dir", Scope::Prefix(Some("src/".into())), 10, &env)
            .await
            .unwrap();
        assert_eq!(sorted(&dirs), [b"cmd".to_vec(), b"cmd/.hidden".to_vec()]);
        // The non-UTF-8 name renders lossily (U+FFFD sorts after ASCII) where it exists.
        let listing = if with_bytes(&env, "", Vec::new()).is_empty() {
            "3 entries: .hidden/, a.rs, new.rs"
        } else {
            "4 entries: .hidden/, a.rs, new.rs, \u{FFFD}.rs"
        };
        assert_eq!(
            enrich_in("dir", Path::new("src/"), &["cmd".into()], &env).await,
            (vec![listing.to_owned()], 0)
        );
    }

    #[tokio::test]
    async fn file_tier_two_reads_finalists_only_and_withholds_secrets() {
        let env = tree();
        let (evidence, withheld) = enrich_in(
            "file",
            Path::new(""),
            &[".npmrc".into(), "src/lib.rs".into(), "src/cmd/a.rs".into()],
            &env,
        )
        .await;
        assert_eq!(withheld, 1);
        assert!(!evidence[0].contains("SECRET"), "{}", evidence[0]);
        assert!(evidence[1].contains("MARKER-LIB"), "{}", evidence[1]);
        assert!(evidence[2].contains("MARKER-A"), "{}", evidence[2]);
        let (evidence, withheld) = enrich_in(
            "file",
            Path::new(""),
            &[
                ".npmrc".into(),
                ".env.local".into(),
                "id_rsa".into(),
                "x.pem".into(),
                "a/.hidden/b.txt".into(),
            ],
            &env,
        )
        .await;
        assert_eq!(withheld, 5);
        assert!(
            evidence.iter().all(|e| !e.contains("SECRET")),
            "{evidence:?}"
        );
        let (evidence, withheld) = enrich_in(
            "file",
            Path::new("--config=src/cmd/"),
            &["a.rs".into(), "new.rs".into(), ".hidden/b.txt".into()],
            &env,
        )
        .await;
        assert_eq!(withheld, 1);
        assert!(evidence[0].contains("MARKER-A"), "{}", evidence[0]);
        assert!(evidence[1].contains("MARKER-NEW"), "{}", evidence[1]);
        assert!(!evidence[2].contains("SECRET"), "{}", evidence[2]);
        // A prefix that names no directory yields empty evidence rather than a read elsewhere.
        assert_eq!(
            enrich_in("file", Path::new("nope/"), &["a.rs".into()], &env).await,
            (vec![String::new()], 0)
        );
    }

    #[tokio::test]
    async fn dir_tier_two_names_children_of_finalists_only_and_withholds_hidden_dirs() {
        let env = tree();
        let (evidence, withheld) = enrich_in(
            "dir",
            Path::new(""),
            &[
                "src".into(),
                "src/cmd".into(),
                "src/cmd/.hidden".into(),
                "a/.hidden".into(),
                "src/lib.rs".into(),
                "missing".into(),
            ],
            &env,
        )
        .await;
        assert_eq!(withheld, 2);
        assert!(
            evidence[0].contains("cmd/") && evidence[0].contains("lib.rs"),
            "{}",
            evidence[0]
        );
        assert!(evidence[0].starts_with("2 entries: "), "{}", evidence[0]);
        assert!(
            evidence[1].contains("a.rs")
                && evidence[1].contains("new.rs")
                && evidence[1].contains(".hidden/"),
            "{}",
            evidence[1]
        );
        assert!(!evidence[1].contains("MARKER"), "{}", evidence[1]);
        assert_eq!(evidence[2], "");
        assert_eq!(evidence[3], "");
        // A file and a missing path carry nothing and are not withheld.
        assert_eq!(evidence[4], "");
        assert_eq!(evidence[5], "");
        // Under a prefix, handles are relative to it.
        let (evidence, withheld) =
            enrich_in("dir", Path::new("--config=src/"), &["cmd".into()], &env).await;
        assert_eq!(withheld, 0);
        assert!(evidence[0].contains("a.rs"), "{}", evidence[0]);
        assert_eq!(
            enrich_in("dir", Path::new("nope/"), &["cmd".into()], &env).await,
            (vec![String::new()], 0)
        );
        // A symbolic link to a directory is withheld; a long listing ends in a count.
        let dir = scratch();
        fs::create_dir(dir.join("real")).unwrap();
        for i in 0..30 {
            fs::write(dir.join("real").join(format!("f{i:02}.txt")), "x\n").unwrap();
        }
        std::os::unix::fs::symlink("real", dir.join("link")).unwrap();
        let env = environment(&dir);
        let (evidence, withheld) =
            enrich_in("dir", Path::new(""), &["real".into(), "link".into()], &env).await;
        assert_eq!(withheld, 1);
        assert!(
            evidence[0].starts_with("30 entries: f00.txt, "),
            "{}",
            evidence[0]
        );
        assert!(evidence[0].ends_with("f23.txt, +6 more"), "{}", evidence[0]);
        assert!(!evidence[0].contains("f24.txt"), "{}", evidence[0]);
        assert_eq!(evidence[1], "");
    }

    #[tokio::test]
    async fn outside_a_work_tree_a_walk_lists_without_following_symlinks() {
        let dir = scratch();
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/config"), "never\n").unwrap();
        fs::write(dir.join("a.txt"), "a\n").unwrap();
        fs::write(dir.join("sub/b.txt"), "b\n").unwrap();
        fs::write(dir.join(".hidden"), "h\n").unwrap();
        std::os::unix::fs::symlink("..", dir.join("sub/loop")).unwrap();
        let env = environment(&dir);
        let listing = enumerate("file", Scope::Prefix(None), 10, &env)
            .await
            .unwrap();
        assert_eq!(
            sorted(&listing),
            [
                b".hidden".to_vec(),
                b"a.txt".to_vec(),
                b"sub/b.txt".to_vec(),
                b"sub/loop".to_vec()
            ]
        );
        let dirs = enumerate("dir", Scope::Prefix(Some("./".into())), 10, &env)
            .await
            .unwrap();
        assert_eq!(sorted(&dirs), [b"sub".to_vec()]);
        // No git on the PATH at all: the walk lists too.
        let no_git = Env {
            path: "/nonexistent".into(),
            ..environment(&dir)
        };
        assert_eq!(
            enumerate("file", Scope::Prefix(Some("sub/".into())), 10, &no_git)
                .await
                .unwrap()
                .records
                .len(),
            2
        );
        let missing = environment(&dir.join("missing"));
        assert_eq!(
            enumerate("file", Scope::Prefix(None), 10, &missing)
                .await
                .unwrap_err()
                .kind(),
            "lister_failed"
        );
    }

    #[tokio::test]
    async fn tools_come_from_the_injected_path_and_count_the_capped_names() {
        let dir = scratch();
        for i in 0..1_501 {
            let file = dir.join(format!("t{i:04}"));
            fs::write(&file, "").unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = Env {
            path: dir.as_os_str().to_owned(),
            ..environment(&dir)
        };
        let listing = enumerate("tool", Scope::Prefix(None), 10, &env)
            .await
            .unwrap();
        assert_eq!(
            (
                listing.records.len(),
                listing.total,
                listing.omitted,
                listing.ordered
            ),
            (1_500, 1_501, 1, false)
        );
        assert_eq!(listing.records[0].handle, "t0000");
        assert_eq!(listing.records[0].evidence, "t0000: (no man page)");
        assert_eq!(
            enumerate("tool", Scope::Prefix(Some("src/".into())), 10, &env)
                .await
                .unwrap_err()
                .kind(),
            "usage"
        );
        assert_eq!(
            enrich_in("tool", Path::new(""), &["t0000".into()], &env).await,
            (vec![String::new()], 0)
        );
        let catalog = catalog(&env);
        let names: Vec<_> = catalog.kinds.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(&names[..6], &KINDS[..6]);
        assert!(catalog.kinds[5].list.is_empty());
        assert_eq!(catalog.kinds[2].list[1], "log");
    }

    #[test]
    fn runner_rejects_oversize_missing_program_expired_deadline_and_bounds_stderr() {
        let env = fake_git("printf '1234567890'; printf '1234567890' >&2");
        let error = run_lister_blocking(&["git".into()], &env, 15).unwrap_err();
        assert!(error.to_string().contains("cap"));
        let error = run_lister_blocking(&["missing-program".into()], &env, 100).unwrap_err();
        assert_eq!(error.kind(), "lister_failed");
        assert_eq!(
            run_lister_blocking(&[], &env, 100).unwrap_err().kind(),
            "lister_failed"
        );
        let expired = Env {
            deadline: Instant::now(),
            ..env
        };
        assert!(
            run_lister_blocking(&["git".into()], &expired, 100)
                .unwrap_err()
                .to_string()
                .contains("deadline")
        );
        let env = fake_git(
            "i=0; while [ \"$i\" -lt 5000 ]; do printf x >&2; i=$((i + 1)); done; printf tail >&2; exit 1",
        );
        let error = run_lister_blocking(&["git".into()], &env, OUTPUT_CAP).unwrap_err();
        assert!(error.to_string().ends_with("tail"));
        assert!(error.to_string().len() < 4200);
    }
}
