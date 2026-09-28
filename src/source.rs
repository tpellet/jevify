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

pub const LISTER_TIMEOUT: Duration = Duration::from_secs(20);
const OUTPUT_CAP: usize = 64 * 1024 * 1024;
/// Characters of diffstat and patch one commit finalist carries. A keyless finals window
/// budgets 30,000 characters over at most 24 finalists, 1,250 each, and a subject, a body and
/// a path list leave most of that unspent; 1,000 fills the room without crowding a finalist
/// out. Above 24 finalists the window's own per-item clip cuts the diff further.
const DIFF_CHARS: usize = 1_000;
pub(crate) const READER_GRACE: Duration = Duration::from_millis(200);
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
/// The options of the `git log` that enriches a branch finalist; the revision follows
/// `--end-of-options`, then `--`.
const BRANCH_LOG: [&str; 8] = [
    "git",
    "log",
    "-5",
    "--format=%x00%s%x00",
    "--name-only",
    "-z",
    "--no-renames",
    "--no-ext-diff",
];
/// What `capabilities` says of every recipe's evidence.
const RECIPE_EVIDENCE: &str =
    "the whole line of the listing; the handle is field N or key KEY of the recipe";

/// A kind this file lists, rather than a recipe, and what `capabilities` says of it.
struct Coded {
    name: &'static str,
    family: &'static str,
    /// The lister argv; empty for `-`, which reads stdin or `--candidates`, and for `tool`,
    /// which reads the PATH and the man index in process.
    list: &'static [&'static str],
    /// The command that enriches a finalist, when it is one, as `capabilities` shows it.
    enrich: &'static [&'static str],
    input: Option<&'static str>,
    evidence: Option<&'static str>,
    forms: Option<&'static str>,
    /// Newest first, so a limit keeps the head; `None` for `-`, in its input's order.
    ordered: Option<bool>,
    /// The literal before the marker scopes the listing (`src/@{file:x}`).
    path_kind: bool,
    /// Finalists carry evidence the listing does not ([`enrich_in`]).
    tier_two: bool,
}

const LISTED: Coded = Coded {
    name: "",
    family: "existing things",
    list: &[],
    enrich: &[],
    input: None,
    evidence: None,
    forms: None,
    ordered: Some(false),
    path_kind: false,
    tier_two: false,
};

/// The coded kinds in `capabilities` order; the shipped recipes of `src/kinds.jsonl` follow.
const CODED: [Coded; 6] = [
    Coded {
        name: "-",
        family: "input records",
        input: Some(
            "stdin or --candidates FILE; --field is 1-based whitespace, --key selects a JSON handle",
        ),
        ordered: None,
        ..LISTED
    },
    Coded {
        name: "branch",
        list: &BRANCH_ARGV,
        enrich: &BRANCH_LOG,
        evidence: Some("name, subject, age; local and remote twins collapse; newest first"),
        forms: Some(
            "bare '@{branch:x}' substitutes the short name a branch-taking command accepts (git switch, checkout, push); a literal prefix 'origin/@{branch:x}' lists that remote's refs and substitutes the qualified ref a revision-taking command resolves (git log, rev-parse, diff)",
        ),
        ordered: Some(true),
        // `origin/@{branch:x}` lists that remote's refs; no ref name starts with a dash.
        path_kind: true,
        tier_two: true,
        ..LISTED
    },
    Coded {
        name: "commit",
        list: &COMMIT_ARGV,
        evidence: Some(
            "full OID and subject; finalists add body, changed paths and the diffstat with the first 1,000 characters of the patch, so a subject that claims a change another commit holds loses the finals; -n <limit> after log, the total from git rev-list --count HEAD; newest first",
        ),
        ordered: Some(true),
        tier_two: true,
        ..LISTED
    },
    Coded {
        name: "file",
        list: &FILE_ARGV,
        evidence: Some(
            "path; finalists add first lines, withheld for the patterns of withheld; a literal prefix ending in / narrows the walk; outside a work tree a no-follow walk",
        ),
        path_kind: true,
        tier_two: true,
        ..LISTED
    },
    Coded {
        name: "dir",
        list: &FILE_ARGV,
        evidence: Some(
            "directory path; finalists add the names of their first children, withheld for the patterns of withheld; a literal prefix ending in / narrows the walk",
        ),
        path_kind: true,
        tier_two: true,
        ..LISTED
    },
    Coded {
        name: "tool",
        evidence: Some(
            "name and one-line manual summary from the PATH and the man index, cached under JEVIFY_CACHE_DIR",
        ),
        ..LISTED
    },
];

fn coded(name: &str) -> Option<&'static Coded> {
    CODED.iter().find(|coded| coded.name == name)
}

/// What a caller needs of a kind to list it and judge its finalists.
#[derive(Debug, Clone)]
pub struct Kind {
    pub name: Cow<'static, str>,
    pub path_kind: bool,
    pub has_tier_two: bool,
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
        if coded(&recipe.kind).is_some() || shipped().iter().any(|r| r.kind == recipe.kind) {
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
    if let Some(coded) = coded(name) {
        return Ok(Some(Kind {
            name: Cow::Borrowed(coded.name),
            path_kind: coded.path_kind,
            has_tier_two: coded.tier_two,
        }));
    }
    Ok(recipe(name, env)?.map(|recipe| Kind {
        name: Cow::Owned(recipe.kind),
        path_kind: false,
        has_tier_two: false,
    }))
}

/// The recipe of a kind that is not coded, with the same read rules as [`lookup`].
fn recipe(name: &str, env: &Env) -> Result<Option<Recipe>, JevifyError> {
    if coded(name).is_some() {
        return Ok(None);
    }
    if let Some(recipe) = shipped().iter().find(|recipe| recipe.kind == name) {
        return Ok(Some(recipe.clone()));
    }
    Ok(user_recipes(env)?
        .into_iter()
        .find(|recipe| recipe.kind == name))
}

/// One kind as `capabilities.kinds` prints it: where it comes from, its lister argv and its
/// evidence.
#[derive(Debug, serde::Serialize)]
pub struct CatalogEntry {
    pub name: String,
    /// `coded`, `shipped` or `user`.
    pub origin: &'static str,
    pub family: &'static str,
    pub list: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub enrich: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forms: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
}

#[derive(Debug)]
pub struct Catalog {
    pub kinds: Vec<CatalogEntry>,
    /// Why the user's `kinds.jsonl` was not listed; a bad file never fails the caller.
    pub error: Option<String>,
}

/// Every kind with its lister argv, the user's recipes included.
pub fn catalog(env: &Env) -> Catalog {
    let strings = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut kinds: Vec<CatalogEntry> = CODED
        .iter()
        .map(|coded| CatalogEntry {
            name: coded.name.into(),
            origin: "coded",
            family: coded.family,
            list: strings(coded.list),
            input: coded.input,
            enrich: match coded.enrich {
                [] => Vec::new(),
                options => strings(&[options, &["--end-of-options", "<handle>", "--"]].concat()),
            },
            evidence: coded.evidence,
            forms: coded.forms,
            ordered: coded.ordered,
        })
        .collect();
    let entry = |recipe: &Recipe, origin| CatalogEntry {
        name: recipe.kind.clone(),
        origin,
        family: LISTED.family,
        list: recipe.list.clone(),
        input: None,
        enrich: Vec::new(),
        evidence: Some(RECIPE_EVIDENCE),
        forms: None,
        ordered: Some(recipe.ordered),
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

/// Tier-two evidence for the given finalists only, and the number of finalists whose excerpt
/// was withheld. `file` and `dir` handles are relative to `prefix` (the literal of the marker,
/// resolved as in enumeration) under `env.cwd`; the other kinds ignore `prefix` and withhold
/// nothing. `file` finalists carry their first lines, `dir` finalists the names of their first
/// children. Missing enrichment is empty evidence; no partial lister output is returned.
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
    let bytes = run_lister(&argv, env)?;
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

/// Run a lister argv with the environment hardened: no prompt, no colour, the injected PATH and
/// configuration directory. Its stdout, or `lister_failed` naming the program.
fn run_lister(argv: &[OsString], env: &Env) -> Result<Vec<u8>, JevifyError> {
    let Some(program) = argv.first() else {
        return Err(JevifyError::lister_failed("empty lister argv".into()));
    };
    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .current_dir(&env.cwd)
        .env("PATH", &env.path)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("NO_COLOR", "1");
    if let Some(dir) = &env.config_dir {
        command.env("JEVIFY_CONFIG_DIR", dir);
    } else {
        command.env_remove("JEVIFY_CONFIG_DIR");
    }
    supervise(command, Mode::Strict, env.deadline, OUTPUT_CAP).map_err(|stopped| {
        let (Stopped::Spawn(message) | Stopped::Failed(message)) = stopped;
        JevifyError::lister_failed(format!("{}: {message}", program.to_string_lossy()))
    })
}

/// What a caller may take from a child's run.
pub(crate) enum Mode {
    /// Stdout only on exit 0 with both pipes at EOF, within `cap` bytes of stdout and stderr
    /// together and before the deadline; anything else is an error ending in stderr's tail,
    /// never a partial output.
    Strict,
    /// The first `cap` bytes of whatever stdout the child wrote before it exited, whatever its
    /// status, the rest drained and stderr discarded; nothing past the deadline.
    BestEffort,
}

#[derive(Debug)]
pub(crate) enum Stopped {
    /// The child never started: the spawn error.
    Spawn(String),
    /// The run gave no output its mode accepts, and why.
    Failed(String),
}

/// The one supervisor of the children jevify reads from: stdin at `/dev/null`, the pipes read
/// on their own threads so a chatty child never blocks on a full one, the child killed at the
/// deadline (a strict run also past the cap), and readers given `READER_GRACE` after the exit,
/// so a grandchild that keeps a pipe open cannot hang the caller.
pub(crate) fn supervise(
    mut command: Command,
    mode: Mode,
    deadline: Instant,
    cap: usize,
) -> Result<Vec<u8>, Stopped> {
    if Instant::now() >= deadline {
        return Err(Stopped::Failed("deadline exceeded".into()));
    }
    let strict = matches!(mode, Mode::Strict);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if strict {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command.spawn().map_err(|e| Stopped::Spawn(e.to_string()))?;
    let (tx, rx) = mpsc::channel();
    let size = Arc::new(AtomicUsize::new(0));
    let pipes: [Option<Box<dyn Read + Send>>; 2] = [
        child.stdout.take().map(|pipe| Box::new(pipe) as _),
        child.stderr.take().map(|pipe| Box::new(pipe) as _),
    ];
    let readers = pipes.iter().flatten().count();
    for (index, pipe) in pipes.into_iter().enumerate() {
        let Some(pipe) = pipe else { continue };
        let (tx, size) = (tx.clone(), Arc::clone(&size));
        std::thread::spawn(move || {
            let _ = tx.send((index, read_pipe(pipe, cap, strict, &size)));
        });
    }
    let mut streams = [None, None];
    let mut failure = None;
    let status = loop {
        while let Ok((index, result)) = rx.try_recv() {
            streams[index] = Some(result);
        }
        if let Some(error) = streams.iter().flatten().find_map(|s| s.as_ref().err()) {
            failure = Some(error.clone());
        }
        if Instant::now() >= deadline {
            failure = Some("deadline exceeded".into());
        }
        if failure.is_some() {
            let _ = child.kill();
            break child.wait().map_err(|e| Stopped::Failed(e.to_string()))?;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(
                Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())),
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
    while streams.iter().flatten().count() < readers {
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
    if strict && !status.success() {
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
        return Err(Stopped::Failed(message));
    }
    Ok(streams[0].take().and_then(Result::ok).unwrap_or_default())
}

/// Read a pipe to EOF. Strict, past `cap` bytes of all the child's pipes together is an error;
/// otherwise the bytes past `cap` are drained and dropped.
fn read_pipe(
    mut pipe: impl Read,
    cap: usize,
    strict: bool,
    size: &AtomicUsize,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let n = match pipe.read(&mut buffer) {
            Ok(0) => return Ok(bytes),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        };
        let before = size.fetch_add(n, Ordering::Relaxed);
        if strict && before.saturating_add(n) > cap {
            return Err("output exceeds lister byte cap".into());
        }
        bytes.extend_from_slice(&buffer[..cap.saturating_sub(before).min(n)]);
    }
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
    let bytes = run_lister(&BRANCH_ARGV.map(OsString::from), env)?;
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
        let mut argv: Vec<OsString> = BRANCH_LOG.map(OsString::from).to_vec();
        argv.extend(revs);
        argv.push("--".into());
        run_lister(&argv, env)
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
        let count = scope.spawn(|| run_lister(&count_argv, env));
        let log = run_lister(&log, env);
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
    let bytes = run_lister(
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
    let bytes = run_lister(
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
    let files = match run_lister(&FILE_ARGV.map(OsString::from), &scoped) {
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

    /// The fake git of `env` as a command, for the supervisor itself.
    fn command(env: &Env) -> Command {
        Command::new(env.cwd.join("git"))
    }

    #[test]
    fn a_lister_runs_hardened_with_null_stdin_and_drains_both_pipes() {
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
        let result = run_lister(&["git".into()], &env).unwrap();
        assert_eq!(result.len(), 12000 * b"stdout stream\n".len());
    }

    /// A lister that fails, overflows or outlives the deadline is an error, never a partial
    /// list, and waiting for it never blocks the runtime.
    #[tokio::test]
    async fn a_lister_failure_overflow_or_deadline_is_an_error_never_a_partial_list() {
        // Exit 1 after a valid ref; exit 0 while a background child keeps the pipes open.
        for (body, text) in [
            (
                "printf 'refs/heads/x\\000\\0001\\000tip\\000\\n'; printf 'tool failed' >&2; exit 1",
                "tool failed",
            ),
            (
                "printf 'refs/heads/x\\000\\0001\\000tip\\000\\n'; /bin/sleep 5 & exit 0",
                "EOF",
            ),
        ] {
            let error = enumerate("branch", Scope::Prefix(None), 10, &fake_git(body))
                .await
                .unwrap_err();
            assert_eq!(error.kind(), "lister_failed");
            assert!(error.to_string().contains(text), "{error}");
        }
        let mut env = fake_git("exec /bin/sleep 2");
        env.deadline = Instant::now() + Duration::from_millis(100);
        let start = Instant::now();
        let (result, ticks) =
            tokio::join!(enumerate("branch", Scope::Prefix(None), 10, &env), async {
                let mut ticks = 0;
                while start.elapsed() < Duration::from_millis(80) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    ticks += 1;
                }
                ticks
            });
        assert!(ticks >= 2);
        assert!(start.elapsed() < Duration::from_millis(700));
        assert!(result.unwrap_err().to_string().contains("deadline"));
        assert!(
            run_lister(&["git".into()], &env)
                .unwrap_err()
                .to_string()
                .contains("deadline")
        );
        // The cap counts stdout and stderr together.
        let env = fake_git("printf '1234567890'; printf '1234567890' >&2");
        let far = Instant::now() + LISTER_TIMEOUT;
        let overflow = supervise(command(&env), Mode::Strict, far, 15);
        assert!(
            matches!(&overflow, Err(Stopped::Failed(message)) if message.contains("cap")),
            "{overflow:?}"
        );
        for argv in [vec![OsString::from("missing-program")], vec![]] {
            assert_eq!(run_lister(&argv, &env).unwrap_err().kind(), "lister_failed");
        }
        let env = fake_git(
            "i=0; while [ \"$i\" -lt 5000 ]; do printf x >&2; i=$((i + 1)); done; printf tail >&2; exit 1",
        );
        let error = run_lister(&["git".into()], &env).unwrap_err().to_string();
        assert!(error.ends_with("tail") && error.len() < 4200, "{error}");
        // Output a recipe cannot read is the lister's failure too.
        let garbled = fake_tool("gh", "echo 'not json'", None);
        let error = enumerate("pr", Scope::Prefix(None), 10, &garbled)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "lister_failed");
        assert!(error.to_string().starts_with("gh: "), "{error}");
    }

    #[test]
    fn best_effort_keeps_the_first_bytes_whatever_the_status_and_nothing_past_the_deadline() {
        let env = fake_git("yes | head -c 200000; exit 3");
        let far = Instant::now() + LISTER_TIMEOUT;
        let out = supervise(command(&env), Mode::BestEffort, far, 100).unwrap();
        assert_eq!(out.len(), 100);
        assert!(out.iter().all(|byte| b"y\n".contains(byte)));
        let out = supervise(command(&env), Mode::BestEffort, far, usize::MAX).unwrap();
        assert_eq!(out.len(), 200_000);
        let slow = fake_git("printf early; exec /bin/sleep 2");
        let soon = Instant::now() + Duration::from_millis(100);
        assert!(supervise(command(&slow), Mode::BestEffort, soon, 100).is_err());
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
        let (remote, _) = enrich_in("branch", Path::new(""), &["remote-only".into()], &env).await;
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
        let (evidence, _) = enrich_in("branch", Path::new(""), &["main".into()], &env).await;
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
            assert_eq!(catalog.kinds.len(), CODED.len() + shipped().len());
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
            assert_eq!(lookup(shadow, &env).unwrap().unwrap().name, shadow);
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
        assert!(!widget.path_kind && !widget.has_tier_two);
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

    /// A repository with hidden, untracked and secret paths.
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
        ] {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, content).unwrap();
        }
        git(&dir, &["add", "-A"], 1700000000);
        let commit = ["-c", "commit.gpgsign=false", "commit", "-m", "tree"];
        git(&dir, &commit, 1700000000);
        fs::write(dir.join("src/cmd/new.rs"), "MARKER-NEW untracked\n").unwrap();
        environment(&dir)
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
        assert_eq!(evidence[0], "2 entries: cmd/, lib.rs");
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
        // A symbolic link to a directory is withheld.
        std::os::unix::fs::symlink("src", env.cwd.join("link")).unwrap();
        let link = enrich_in("dir", Path::new(""), &["link".into()], &env).await;
        assert_eq!(link, (vec![String::new()], 1));
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
}
