use crate::output::Format;
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "jevify",
    version,
    about = "Answer questions about text you already have: find a line, an error, a command or a folder by meaning. jevify selects and never generates, with backend-specific decision scores.",
    after_help = "Examples:\n  gh run view --log-failed | jevify why\n  jevify fill --dry-run -- git switch '@{branch:the payment timeout fix}'\n  jevify pick --from tool \"keep my mac awake for an hour\"\n  git branch | jevify pick \"the payment timeout fix\"\n  cargo test 2>&1 | jevify filter \"reports a failed assertion\"\n  jevify add --dry-run \"the token expiry fix\"\n\nExit codes: 0 ok, 1 no, 2 usage, 3 nothing fits or unsure, 4 API unavailable, 5 auth, 6 input, 130 declined.\nAgents: jevify capabilities --json | jevify init agents"
)]
pub struct Cli {
    #[command(flatten)]
    pub g: GlobalOpts,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Args, Debug, Clone)]
pub struct GlobalOpts {
    /// Machine output: one JSON envelope on stdout (alias: --robot)
    #[arg(long, global = true, alias = "robot")]
    pub json: bool,
    /// Decision threshold on the backend's score (calibration depends on task and backend)
    #[arg(short = 't', long, global = true, env = "JEVIFY_THRESHOLD")]
    pub threshold: Option<f64>,
    /// TypeSafe model or alias (default jev-1.13.0); unsupported by classifier.dev
    #[arg(long, global = true, env = "JEVIFY_MODEL")]
    pub model: Option<String>,
    /// Skip the local answer cache
    #[arg(long, global = true)]
    pub no_cache: bool,
    /// Print probabilities and timing on stderr
    #[arg(long, global = true)]
    pub verbose: bool,
}

impl GlobalOpts {
    pub fn format(&self) -> Format {
        if self.json {
            Format::Json
        } else {
            Format::Human
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Resolve marked arguments and become the command; --dry-run prints it
    Fill {
        #[arg(long)]
        dry_run: bool,
        #[arg(short = 'q')]
        quiet: bool,
        #[arg(long, value_name = "FILE")]
        candidates: Option<std::path::PathBuf>,
        #[arg(long, value_name = "FILE")]
        context: Option<std::path::PathBuf>,
        #[arg(long, conflicts_with = "key")]
        field: Option<usize>,
        #[arg(long)]
        key: Option<String>,
        #[arg(short = '0', conflicts_with = "para")]
        nul: bool,
        #[arg(long)]
        para: bool,
        /// Run as if started in DIR: listers, excerpts and the command
        #[arg(short = 'C', long = "repo", value_name = "DIR")]
        repo: Option<std::path::PathBuf>,
        #[arg(last = true, required = true)]
        cmd: Vec<std::ffi::OsString>,
    },
    /// Find one line in a list by describing it: stdin lines in, the matching line out
    #[command(
        after_help = "Examples:\n  git branch | jevify pick \"the payment timeout fix\"\n  git log --oneline | jevify pick -n 3 \"when we changed the pricing\"\n  git ls-files | jevify pick --files \"where man pages are parsed\"\n  jevify pick --from commit made folder moves atomic\n\nThe description and the record need no word in common; quote it or leave it as words. --files ranks stdin paths first (with nothing on stdin, the files under the current directory), then reads excerpts of the finalists; hidden or secret-looking paths and symlink files receive no excerpt, and a file that cannot be read (missing, a directory, denied) is named on stderr; both count in excerpts withheld: N. Selected records keep their bytes and input order. Input is not saved.\nExit: 0 found, 3 no record fits. --json data: matches[{line, text, ordinal, p, lossy?}], any, source. Non-UTF-8 records have lossy: true."
    )]
    Pick {
        /// List candidates of this kind instead of reading stdin
        #[arg(long, conflicts_with_all = ["files", "index", "nul", "para"])]
        from: Option<String>,
        /// Describe the line you want, e.g. "the branch with the payment timeout fix"; unquoted words are joined
        #[arg(required = true, num_args = 1.., value_name = "INTENT")]
        intent: Vec<String>,
        /// Print up to N matches, each ranked above "nothing fits"
        #[arg(short = 'n', long, default_value_t = 1)]
        top: usize,
        /// Print 1-based line numbers instead of lines
        #[arg(long, conflicts_with = "files")]
        index: bool,
        /// Read paths from stdin and use file excerpts as evidence
        #[arg(long)]
        files: bool,
        /// Split stdin on NUL bytes
        #[arg(short = '0', conflicts_with = "para")]
        nul: bool,
        /// Split stdin into paragraphs
        #[arg(long)]
        para: bool,
        /// Run as if started in DIR: the --from lister, the --files listing and excerpts
        #[arg(short = 'C', long = "repo", value_name = "DIR")]
        repo: Option<std::path::PathBuf>,
    },
    /// Find the line that caused a failure in build, test or CI output on stdin
    #[command(
        after_help = "Examples:\n  cargo build 2>&1 | jevify why\n  gh run view --log-failed | jevify why --json\n\nPipe 2>&1: compilers write errors to stderr. Prints numbered context; takes no split option. Saves raw input, secrets included, for seven days, unless --no-save or JEVIFY_NO_SAVE=1 (JEVIFY_NO_CACHE does not stop it); stderr names the full output path.\nExit: 0 found, 3 no line looks like a failure. --json data: causes[{line, text, p, context[]}], any, considered, total, hint, saved_input, complete. A skipped or failed save sets complete: false."
    )]
    Why {
        /// Lines of context around the root cause
        #[arg(short = 'C', long, default_value_t = 3)]
        context: usize,
        /// Report up to N causes, each ranked above "no failure"
        #[arg(short = 'n', long, default_value_t = 1)]
        top: usize,
        /// Do not save the full input
        #[arg(long)]
        no_save: bool,
    },
    /// Keep stdin records that satisfy a statement
    #[command(
        after_help = "Example:\n  cargo test 2>&1 | jevify filter 'reports a failed assertion'\n\n-v inverts; -c prints the count. Unsure records stay unless --strict. --verbose has no short flag. --files reads stdin paths; hidden or secret-looking paths and symlink files receive no excerpt, and a file that cannot be read is named on stderr and comes out unsure (both count in excerpts withheld: N; records carry unreadable: REASON). Saves raw input, secrets included, for seven days, unless --no-save or JEVIFY_NO_SAVE=1 (JEVIFY_NO_CACHE does not stop it). Status: jevify filter: kept N of M, U unsure, full output: PATH.\nExit: 0 kept some, 1 kept none, 3 every record unsure. --json data: records[{text, ordinal, p, verdict, lossy?}], kept, total, unsure, complete, saved_input, excerpts_withheld. A skipped or failed save sets complete: false. Non-UTF-8 records have lossy: true."
    )]
    Filter {
        /// What must be true of a kept record; unquoted words are joined
        #[arg(required = true, num_args = 1.., value_name = "STATEMENT")]
        statement: Vec<String>,
        #[arg(short = 'v')]
        invert: bool,
        #[arg(short = 'c')]
        count: bool,
        #[arg(long)]
        strict: bool,
        #[arg(short = '0', conflicts_with = "para")]
        nul: bool,
        #[arg(long)]
        para: bool,
        #[arg(long)]
        files: bool,
        #[arg(long)]
        no_save: bool,
    },
    /// Tag each stdin record with one of the given labels: a label, a tab, the record; ? when unsure
    #[command(
        after_help = "Example:\n  gh issue list | jevify label bug,feature,question | cut -f1 | sort | uniq -c\n\nLabels are comma-separated: at least two, distinct, none empty, none ? or NONE. Each record comes out as LABEL<TAB>RECORD, in input order and unchanged after the tab; an unsure record gets ?. --files reads stdin paths and judges each file's first lines. Saves nothing.\nExit: 0 labelled, 3 every record unsure. --json data: records[{label, text, ordinal, p, lossy?}], labelled, total, unsure. Non-UTF-8 records have lossy: true."
    )]
    Label {
        /// The labels, comma-separated, e.g. bug,feature,question
        #[arg(value_name = "LABELS", value_parser = parse_labels)]
        labels: Labels,
        /// Split stdin on NUL bytes
        #[arg(short = '0', conflicts_with = "para")]
        nul: bool,
        /// Split stdin into paragraphs
        #[arg(long)]
        para: bool,
        /// Read paths from stdin and use file excerpts as evidence
        #[arg(long)]
        files: bool,
    },
    /// Ask a yes-or-no question about the text on stdin; the answer is the exit code (0 yes, 1 no, 3 unsure)
    #[command(
        after_help = "Examples:\n  jevify is \"the customer asks for a refund\" < mail.txt && ./refund\n  jevify is 'asks for a refund' 'mentions an order' --context mail.txt\n\nWrite the condition so that yes means act. Each statement is judged literally. No counting, arithmetic, dates or quality judgments. Oversized input is not judged.\nOne statement prints nothing on human stdout; several print VERDICT<TAB>STATEMENT lines. Exit: 0 all yes, 1 one no, 3 otherwise. --json data: p, verdict, truncated, reason (when oversized); several: statements[{statement, verdict, p}], verdict, truncated."
    )]
    Is {
        /// A statement that must be true of the text, e.g. "the customer asks for a refund"; quote each of several, unquoted words are one statement
        #[arg(required = true, num_args = 1..)]
        statements: Vec<String>,
        /// Read the context from a file instead of stdin
        #[arg(long, value_name = "FILE")]
        context: Option<std::path::PathBuf>,
        /// Unsure band around the threshold (0..=0.5)
        #[arg(long, default_value_t = 0.15)]
        band: f64,
    },
    /// Stage only the git changes that belong to one topic, like `git add -p` without the questions
    #[command(
        after_help = "Examples:\n  jevify add --dry-run \"the token expiry fix\"\n  jevify add --yes \"the token expiry fix\" && git commit\n\nStages single hunks of tracked files, so it can split the changes of one file. Index only, never commits. Rejects hunks above 3000 characters and batches above the backend evidence budget before API requests or staging; no hunk evidence is clipped.\nExit: 0 staged (or scored with --dry-run), 3 no change is about the topic, 6 empty or oversized input, 130 declined. --json data: hunks[{file, header, p, staged}]."
    )]
    Add {
        /// The topic of the changes to stage, e.g. "the token expiry fix"; unquoted words are joined
        #[arg(required = true, num_args = 1.., value_name = "TOPIC")]
        topic: Vec<String>,
        /// Stage without asking
        #[arg(short, long)]
        yes: bool,
        /// Score the hunks; stage nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Describe commands, flags, exit codes, env and limits for agents
    Capabilities,
    /// Check which backend answers, whether a key is needed, and how fast it replies
    Health,
    /// Print an agent instruction block
    Init { shell: Shell },
}

/// A free-text argument as the caller wrote it, quoted as one word or left as several: the
/// words are joined with single spaces. A leading lone `-` is dropped, since stdin is already
/// the default source. `None` when nothing is left to describe.
pub fn words(words: &[String]) -> Option<String> {
    let skip = usize::from(words.first().is_some_and(|w| w == "-"));
    let text = words[skip..]
        .iter()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}

/// The statements of `is`: each argument is one statement, as documented, and a leading lone
/// `-` is dropped, as in [`words`]. The second value is set when every one of several
/// statements is a single word, which reads like one sentence left unquoted: the caller is told
/// so on stderr, and the statements stay separate.
pub fn statements(args: &[String]) -> (Vec<String>, bool) {
    let args = match args {
        [dash, rest @ ..] if dash == "-" && !rest.is_empty() => rest,
        _ => args,
    };
    let unquoted = args.len() > 2 && !args.iter().any(|a| a.contains(char::is_whitespace));
    (args.to_vec(), unquoted)
}

/// The labels of `label`, validated once by the parser: at least two, distinct, none empty,
/// none `?` (the unsure mark) or `NONE` (the internal option). Their count against the
/// backend's window is checked in `cmd::label`, where the backend is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labels(pub Vec<String>);

const LABELS_FORM: &str = "at least two comma-separated labels, distinct, none empty, none ? or NONE: jevify label bug,feature";

fn parse_labels(text: &str) -> Result<Labels, String> {
    let labels: Vec<String> = text.split(',').map(str::to_owned).collect();
    let repeated = labels
        .iter()
        .enumerate()
        .find(|(i, l)| labels[..*i].contains(l))
        .map(|(_, l)| l);
    let problem = if labels.len() < 2 {
        Some("one label".to_owned())
    } else if let Some(empty) = labels.iter().position(String::is_empty) {
        Some(format!("label {} is empty", empty + 1))
    } else if let Some(reserved) = labels.iter().find(|l| *l == "?" || *l == "NONE") {
        Some(format!("{reserved} is reserved"))
    } else {
        repeated.map(|repeated| format!("{repeated} is repeated"))
    };
    match problem {
        Some(problem) => Err(format!("{problem}; expected {LABELS_FORM}")),
        None => Ok(Labels(labels)),
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum Shell {
    Agents,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_text_is_the_words_joined_and_a_leading_dash_is_stdin() {
        let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert_eq!(words(&s(&["the", "borrow"])).unwrap(), "the borrow");
        assert_eq!(words(&s(&["-", "the", " borrow "])).unwrap(), "the borrow");
        // `-` after the first word is a word; `-` alone, blanks and nothing describe nothing.
        assert_eq!(words(&s(&["a", "-", "b"])).unwrap(), "a - b");
        assert_eq!(words(&s(&["-"])), None);
        assert_eq!(words(&s(&["", "  "])), None);
        assert_eq!(words(&[]), None);
        // is: every argument is a statement; three or more one-word ones are flagged.
        let (four, unquoted) = statements(&s(&["asks", "for", "a", "refund"]));
        assert_eq!((four.len(), unquoted), (4, true));
        assert_eq!(
            statements(&s(&["-", "asks for a refund"])),
            (s(&["asks for a refund"]), false)
        );
        assert_eq!(statements(&s(&["-"])), (s(&["-"]), false));
    }
}
