//! The caller's own argv, read without clap: moved into the order clap accepts before parsing,
//! and rebuilt into a corrected command after a failure, so that every error names a command
//! the caller can run next instead of a pointer to the manual.

use crate::cli::Cli;
use crate::exit::JevifyError;
use clap::CommandFactory;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// An argv split the way jevify reads it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Argv {
    /// Global options (with their values) written before the verb.
    pub global: Vec<String>,
    pub verb: String,
    /// Options of the verb and global options written after it, each with its value.
    pub flags: Vec<String>,
    /// Positional words: an intent, a statement, labels, a directory.
    pub words: Vec<String>,
    /// Everything after `--`, `None` without one.
    pub tail: Option<Vec<String>>,
    /// How many tokens at the start of `flags` were written before the verb.
    pub lifted: usize,
}

/// Whether `token` is an option of `command` and takes a separate value.
fn option(command: &clap::Command, token: &str) -> Option<bool> {
    let (name, attached) = match token.split_once('=') {
        Some((name, _)) => (name, true),
        None => (token, false),
    };
    command.get_arguments().find_map(|arg| {
        let long = name.strip_prefix("--").is_some_and(|long| {
            arg.get_long() == Some(long)
                || arg
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&long))
        });
        let short = name.len() == 2
            && name.starts_with('-')
            && !name.starts_with("--")
            && arg.get_short() == name.chars().nth(1);
        (long || short).then(|| arg.get_action().takes_values() && !attached)
    })
}

/// Split UTF-8 args (without the program name). `None` when there is no verb.
pub fn split(args: &[String]) -> Option<Argv> {
    let root = Cli::command();
    let mut argv = Argv::default();
    let mut pending = Vec::new();
    let mut it = args.iter().peekable();
    while let Some(token) = it.next() {
        if token == "--" {
            return None;
        }
        if token.starts_with('-') && token.len() > 1 {
            pending.push(token.clone());
            // Before the verb, a verb option's value is the next word unless that is a verb.
            let takes = match option(&root, token) {
                Some(takes) => takes,
                None => {
                    it.peek()
                        .is_some_and(|next| root.find_subcommand(next.as_str()).is_none())
                        && root
                            .get_subcommands()
                            .any(|sub| option(sub, token) == Some(true))
                }
            };
            if takes {
                pending.extend(it.next().cloned());
            }
            continue;
        }
        argv.verb = token.clone();
        break;
    }
    if argv.verb.is_empty() {
        return None;
    }
    let sub = root.find_subcommand(&argv.verb).cloned();
    // A verb option written before the verb moves after it; a global option stays.
    let mut pre = pending.into_iter().peekable();
    while let Some(token) = pre.next() {
        let verb_option = sub.as_ref().and_then(|sub| option(sub, &token));
        match verb_option {
            Some(takes) if option(&root, &token).is_none() => {
                argv.flags.push(token);
                if takes {
                    argv.flags.extend(pre.next());
                }
                argv.lifted = argv.flags.len();
            }
            _ => {
                let takes = option(&root, &token) == Some(true);
                argv.global.push(token);
                if takes {
                    argv.global.extend(pre.next());
                }
            }
        }
    }
    while let Some(token) = it.next() {
        if token == "--" {
            argv.tail = Some(it.cloned().collect());
            break;
        }
        if token.starts_with('-') && token.len() > 1 {
            argv.flags.push(token.clone());
            let takes = sub
                .as_ref()
                .and_then(|sub| option(sub, token))
                .or_else(|| option(&root, token))
                == Some(true);
            if takes {
                argv.flags.extend(it.next().cloned());
            }
            continue;
        }
        argv.words.push(token.clone());
    }
    Some(argv)
}

/// The args clap is given: verb options written before the verb move after it
/// (`jevify --no-save filter x` is `jevify filter --no-save x`). Anything else is unchanged.
pub fn reorder(args: &[OsString]) -> Vec<OsString> {
    let Some(text) = args
        .iter()
        .map(|a| a.to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
    else {
        return args.to_vec();
    };
    let Some(argv) = split(&text) else {
        return args.to_vec();
    };
    if argv.lifted == 0 {
        return args.to_vec();
    }
    let before = argv.global.len() + argv.lifted;
    argv.global
        .iter()
        .chain(std::iter::once(&argv.verb))
        .chain(&argv.flags[..argv.lifted])
        .chain(&text[before + 1..])
        .map(OsString::from)
        .collect()
}

/// Quote a word for a POSIX shell only when it needs it.
pub fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:,=@%+-~".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

impl Argv {
    /// The command line, with the words joined into one quoted argument.
    pub fn render(&self) -> String {
        let mut parts = vec!["jevify".to_owned()];
        parts.extend(self.global.iter().map(|g| quote(g)));
        parts.push(self.verb.clone());
        parts.extend(self.flags.iter().map(|f| quote(f)));
        if let Some(words) = crate::cli::words(&self.words) {
            parts.push(quote(&words));
        }
        if let Some(tail) = &self.tail {
            parts.push("--".into());
            parts.extend(tail.iter().map(|t| quote(t)));
        }
        parts.join(" ")
    }

    /// The same command without `flag` and, when it takes one, its value.
    fn without(&self, flag: &str) -> Self {
        let drop = |list: &[String]| {
            let mut kept = Vec::new();
            let mut it = list.iter();
            while let Some(f) = it.next() {
                if f == flag || f.starts_with(&format!("{flag}=")) {
                    if !f.contains('=') && self.takes(flag) {
                        it.next();
                    }
                    continue;
                }
                kept.push(f.clone());
            }
            kept
        };
        Self {
            global: drop(&self.global),
            flags: drop(&self.flags),
            lifted: 0,
            ..self.clone()
        }
    }

    fn takes(&self, flag: &str) -> bool {
        let root = Cli::command();
        root.find_subcommand(&self.verb)
            .and_then(|sub| option(sub, flag))
            .or_else(|| option(&root, flag))
            == Some(true)
    }

    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    fn with_words(&self, words: &str) -> Self {
        Self {
            words: vec![words.to_owned()],
            ..self.clone()
        }
    }

    /// The same command with a placeholder description when its verb takes one and it has none.
    fn described(&self) -> Self {
        let takes_text = matches!(self.verb.as_str(), "pick" | "filter" | "is" | "add");
        if !takes_text || crate::cli::words(&self.words).is_some() {
            self.clone()
        } else {
            self.with_words(placeholder(&self.verb))
        }
    }
}

fn placeholder(verb: &str) -> &'static str {
    match verb {
        "filter" | "is" => "<what must be true of the text>",
        "add" => "<the topic of the changes>",
        _ => "<what the line you want says>",
    }
}

/// The directories under `root`, at most two levels down, that hold a `.git`: where a caller
/// that ran jevify from a parent directory most likely meant it to run. Hidden directories and
/// symlinks are not entered; at most `limit` are returned, sorted.
pub fn git_repos_below(root: &Path, limit: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![root.to_path_buf()];
    for _ in 0..2 {
        let mut next = Vec::new();
        for dir in level {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut children: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .map(|e| e.path())
                .collect();
            children.sort();
            for child in children {
                if child.join(".git").exists() {
                    found.push(child);
                } else {
                    next.push(child);
                }
            }
        }
        level = next;
    }
    found.sort();
    found.truncate(limit);
    found
}

/// A hint and a runnable example built from the caller's argv for `error`, or `None` when the
/// error's own static pair is as specific as jevify can be (an outage, a bad key).
pub fn advice(args: &[String], error: &JevifyError, cwd: &Path) -> Option<(String, String)> {
    let argv = split(args)?;
    let message = error.to_string();
    let verb = argv.verb.as_str();
    match error.kind() {
        "usage" => usage(&argv, &message),
        "empty_input" if message.contains("no unstaged changes") => None,
        "empty_input" | "stdin_is_tty" => Some(no_input(&argv)),
        "lister_failed" if message.contains("not a git repository") => {
            let repos = git_repos_below(cwd, 3);
            let dir = repos
                .first()
                .map(|r| r.strip_prefix(cwd).unwrap_or(r).display().to_string())
                .unwrap_or_else(|| "/path/to/repo".into());
            let mut fixed = argv.clone();
            if !fixed.has("-C") && !fixed.has("--repo") {
                fixed.flags.insert(0, "-C".into());
                fixed.flags.insert(1, dir.clone());
            }
            let found = if repos.is_empty() {
                "no repository below it either".to_owned()
            } else {
                format!(
                    "repositories below it: {}",
                    repos
                        .iter()
                        .map(|r| r.strip_prefix(cwd).unwrap_or(r).display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            Some((
                format!(
                    "the lister ran in {}, which is not inside a git work tree ({found}); -C DIR runs {verb} as if started in DIR",
                    cwd.display()
                ),
                fixed.render(),
            ))
        }
        "input" if message.starts_with("-C ") => Some((
            format!("-C names the directory {verb} runs in, and it must exist"),
            argv.without("-C").without("--repo").render(),
        )),
        "input" if message.starts_with("--context takes a file path") => Some((
            format!("{verb} reads the text on stdin; --context names a file"),
            format!(
                "printf '%s\\n' \"$TEXT\" | {}",
                argv.without("--context").render()
            ),
        )),
        "input" if message.contains("; nearby: ") && argv.has("--context") => {
            let nearest = message
                .split("; nearby: ")
                .nth(1)
                .and_then(|list| list.split(", ").next())
                .unwrap_or_default();
            let mut fixed = argv.without("--context");
            fixed.flags.push("--context".into());
            fixed.flags.push(nearest.to_owned());
            Some((
                "--context names a file that does not exist".into(),
                fixed.render(),
            ))
        }
        "declined" => {
            let mut fixed = argv.without("--dry-run");
            fixed.flags.insert(0, "--yes".into());
            Some((
                "nothing was staged; --yes stages without asking, --dry-run only scores".into(),
                fixed.render(),
            ))
        }
        _ => None,
    }
}

fn usage(argv: &Argv, message: &str) -> Option<(String, String)> {
    let verb = argv.verb.as_str();
    if verb == "fill" && argv.tail.is_none() && !argv.words.is_empty() {
        let fixed = Argv {
            words: vec![],
            tail: Some(argv.words.clone()),
            ..argv.clone()
        };
        return Some((
            "put the command after --, with each marker argument in single quotes".into(),
            fixed.render(),
        ));
    }
    if let Some(unknown) = between(message, "unrecognized subcommand '", "'") {
        let nearest = Cli::command()
            .get_subcommands()
            .map(|s| s.get_name().to_owned())
            .filter(|name| name != "help")
            .min_by_key(|name| crate::cmd::pick::distance(unknown, name))
            .unwrap_or_else(|| "pick".into());
        let fixed = Argv {
            verb: nearest.clone(),
            ..argv.clone()
        };
        return Some((
            format!(
                "jevify {} has no verb {unknown}; nearest: {nearest}. A guide written for a newer jevify may name verbs this one lacks: cargo install jevify updates it",
                env!("CARGO_PKG_VERSION")
            ),
            fixed.render(),
        ));
    }
    if let Some(unexpected) = between(message, "unexpected argument '", "'") {
        if unexpected.starts_with('-') {
            let fixed = argv.without(unexpected).described();
            return Some((
                format!("{verb} has no option {unexpected}; `jevify {verb} --help` lists them"),
                fixed.render(),
            ));
        }
        return Some((
            "quote the whole description as one argument".into(),
            argv.described().render(),
        ));
    }
    if message.contains("required arguments were not provided") {
        return Some((
            format!("{verb} needs a description"),
            argv.described().render(),
        ));
    }
    if message.contains("needs a description") {
        let mut fixed = argv.clone();
        fixed.words.clear();
        return Some((
            "stdin is already the default source; write the description after the verb".into(),
            fixed.described().render(),
        ));
    }
    if message.contains("-n must be at least 1") {
        let mut fixed = argv.without("-n").without("--top");
        fixed.flags.push("-n".into());
        fixed.flags.push("1".into());
        return Some(("-n counts matches from 1".into(), fixed.render()));
    }
    if message.contains("machine output requires --dry-run") {
        let mut fixed = argv.clone();
        fixed.flags.insert(0, "--dry-run".into());
        return Some((
            "fill runs the command; under --json it only previews it".into(),
            fixed.render(),
        ));
    }
    if message.contains("--index numbers stdin lines") {
        return Some((
            "with --files the match is a path, so --index has nothing to number".into(),
            argv.without("--index").render(),
        ));
    }
    if let Some(nearest) = between(message, "did you mean \"", "\"") {
        let mut fixed = argv.without("--from");
        fixed.flags.insert(0, "--from".into());
        fixed.flags.insert(1, nearest.into());
        return Some((
            "`jevify capabilities --json` lists every kind".into(),
            fixed.described().render(),
        ));
    }
    None
}

fn no_input(argv: &Argv) -> (String, String) {
    let verb = argv.verb.as_str();
    let fixed = argv.described().render();
    match verb {
        "why" => (
            "why reads the failing command's output on stdin; compilers write errors to stderr".into(),
            format!("cargo test 2>&1 | {fixed}"),
        ),
        "is" => (
            "is judges the text on stdin, or the file --context names".into(),
            if argv.has("--context") {
                fixed
            } else {
                format!("{fixed} --context FILE")
            },
        ),
        "fill" => (
            "a '@{-:...}' marker reads candidates from stdin or --candidates FILE".into(),
            format!("git log --oneline | {fixed}"),
        ),
        "pick" if !argv.has("--files") => (
            "pick reads candidates from stdin: pipe a list, or let jevify list a kind with --from KIND (branch, commit, file, dir, tool, ...)".into(),
            format!("git log --oneline | {fixed}"),
        ),
        "filter" if argv.has("--files") => (
            format!("{verb} --files reads paths from stdin"),
            format!("git ls-files | {fixed}"),
        ),
        _ => (
            format!("{verb} reads records from stdin: pipe the output of a command into it"),
            format!("git ls-files | {fixed}"),
        ),
    }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &text[text.find(start)? + start.len()..];
    Some(&rest[..rest.find(end)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(a: &[&str]) -> Vec<String> {
        a.iter().map(|x| (*x).to_owned()).collect()
    }

    fn advise(args: &[&str], error: JevifyError) -> (String, String) {
        advice(&s(args), &error, Path::new("/nonexistent")).unwrap()
    }

    /// The corrections the binary tests in `tests/` do not reach; the rest are covered there.
    #[test]
    fn usage_and_input_errors_name_the_corrected_command() {
        let usage = |m: &str| JevifyError::Usage(m.into());
        let input = |m: &str| JevifyError::Input(m.into());
        for (args, error, example) in [
            (
                vec!["--json", "fill", "--", "git", "switch", "@{branch:x}"],
                usage("machine output requires --dry-run"),
                "jevify --json fill --dry-run -- git switch '@{branch:x}'",
            ),
            (
                vec!["pick", "--from", "branc", "x"],
                usage("unknown kind \"branc\"; did you mean \"branch\"? kinds: -, branch"),
                "jevify pick --from branch x",
            ),
            (
                vec!["pick", "--files", "--index", "x"],
                usage("--index numbers stdin lines; with --files the match is a path"),
                "jevify pick --files x",
            ),
            (
                vec![
                    "is",
                    "asks for a refund",
                    "--context",
                    "Dear team,\nrefund me",
                ],
                input("--context takes a file path, not the text: pipe the text on stdin instead"),
                "printf '%s\\n' \"$TEXT\" | jevify is 'asks for a refund'",
            ),
            (
                vec!["is", "refund", "--context", "mail.tx"],
                input("mail.tx: No such file or directory (os error 2); nearby: ./mail.txt"),
                "jevify is --context ./mail.txt refund",
            ),
        ] {
            assert_eq!(advise(&args, error).1, example);
        }
        // An error jevify cannot correct keeps its static pair.
        assert!(advice(&s(&["pick", "x"]), &usage("something else"), Path::new("/")).is_none());
        assert!(
            advice(
                &s(&["pick", "x"]),
                &JevifyError::Unavailable("down".into()),
                Path::new("/")
            )
            .is_none()
        );
        assert!(advice(&s(&[]), &usage("x"), Path::new("/")).is_none());
    }
}
