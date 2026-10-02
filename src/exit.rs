use serde::Serialize;

/// The phrase the overall-deadline sentence carries, so that a reader of stderr and a reader of
/// `error.message` meet the same words. `tests/deadline.rs` drives a real expiry and pins the
/// sentence, the kind and this constant together.
pub const DEADLINE_PREFIX: &str = "overall deadline of";

pub const NO_MATCH: &str = "no_match";
pub const AMBIGUOUS: &str = "ambiguous";
pub const UNSURE_FLAG: &str = "unsure_flag";
pub const INSUFFICIENT_EVIDENCE: &str = "insufficient_evidence";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
    Ok = 0,
    No = 1,
    Usage = 2,
    Abstain = 3,
    Unavailable = 4,
    Auth = 5,
    Input = 6,
    Interrupted = 130,
}

impl Exit {
    pub fn code(self) -> i32 {
        self as i32
    }
    pub const ALL: [(Exit, &'static str); 8] = [
        (Exit::Ok, "success: yes / found"),
        (Exit::No, "`is`: the condition does not hold"),
        (Exit::Usage, "usage error: bad flag or missing argument"),
        (Exit::Abstain, "abstain: nothing fits, or unsure"),
        (
            Exit::Unavailable,
            "API unavailable, deadline passed or quota exhausted",
        ),
        (Exit::Auth, "API key missing or rejected"),
        (Exit::Input, "input error: empty, too large, or unreadable"),
        (Exit::Interrupted, "interrupted or declined at confirmation"),
    ];
}

#[derive(Debug, thiserror::Error)]
pub enum JevifyError {
    #[error("{message}")]
    Kinded {
        kind: &'static str,
        exit: Exit,
        message: String,
        hint: &'static str,
        example: &'static str,
    },
    /// Only reachable with `JEVIFY_BACKEND=typesafe`: without a key jevify uses classifier.dev.
    #[error("the typesafe backend needs a key: set TYPESAFE_API_KEY or TYPESAFE_API_KEY_FILE")]
    MissingKey,
    #[error("the API rejected the key (HTTP {0})")]
    BadKey(u16),
    #[error("API unavailable: {0}")]
    Unavailable(String),
    /// The verb's own budget ran out, which is not an outage: the API was never asked, or was
    /// asked and cancelled. It shares exit 4 with `Unavailable`, and its own sentence so that a
    /// person reading stderr learns the next move the way `api_deadline` already tells a machine.
    /// The field is the budget in seconds, formatted.
    #[error(
        "the overall deadline of {0} s passed before the answer was ready; JEVIFY_DEADLINE sets it"
    )]
    Deadline(String),
    #[error("unexpected response from the API: {0}")]
    Protocol(String),
    #[error("no input: {0}")]
    EmptyInput(&'static str),
    #[error("input too large: {0}")]
    InputTooLarge(String),
    /// HTTP 413/422: the API rejected the request body. Usually the state is over the token
    /// budget, sometimes the request is malformed (a jevify bug); an input error, not an outage.
    #[error("the API rejected the request (HTTP {0}): {1}")]
    RejectedRequest(u16, String),
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    Usage(String),
    #[error("declined")]
    Declined,
}

impl JevifyError {
    /// Every `error.kind` a caller can receive, with the exit code it carries: the enumeration
    /// `capabilities` publishes, built from this one table. `kind()` is the only producer, and
    /// the tests below prove the two agree in both directions, so a kind cannot reach a caller
    /// without appearing here, and nothing here is unreachable.
    pub const KINDS: [(&'static str, Exit); 18] = [
        ("usage", Exit::Usage),
        ("api_unavailable", Exit::Unavailable),
        ("quota_exhausted", Exit::Unavailable),
        ("api_deadline", Exit::Unavailable),
        ("api_protocol", Exit::Unavailable),
        ("missing_api_key", Exit::Auth),
        ("bad_api_key", Exit::Auth),
        ("empty_input", Exit::Input),
        ("input_too_large", Exit::Input),
        ("api_rejected_request", Exit::Input),
        ("input", Exit::Input),
        ("too_many", Exit::Input),
        ("stdin_is_tty", Exit::Input),
        ("lister_failed", Exit::Input),
        ("cannot_run", Exit::Input),
        ("recipe_invalid", Exit::Input),
        ("status_file_unwritable", Exit::Input),
        ("declined", Exit::Interrupted),
    ];

    pub fn quota_exhausted(message: &str, hint: &'static str) -> Self {
        Self::Kinded {
            kind: "quota_exhausted",
            exit: Exit::Unavailable,
            message: message.into(),
            hint,
            example: "TYPESAFE_API_KEY_FILE=/path/to/key jevify health",
        }
    }

    pub fn stdin_is_tty(message: String) -> Self {
        Self::Kinded {
            kind: "stdin_is_tty",
            exit: Exit::Input,
            message,
            hint: "pipe candidates or context, or provide a file",
            example: "jevify fill --candidates input -- CMD '@{-:description}'",
        }
    }
    pub fn lister_failed(message: String) -> Self {
        Self::Kinded {
            kind: "lister_failed",
            exit: Exit::Input,
            message,
            hint: "check the lister and narrow its scope",
            example: "jevify pick --from branch 'description'",
        }
    }
    pub fn cannot_run(message: String) -> Self {
        Self::Kinded {
            kind: "cannot_run",
            exit: Exit::Input,
            message,
            hint: "check the command path and executable permissions",
            example: "jevify fill --dry-run -- CMD '@{-:description}'",
        }
    }
    pub fn recipe_invalid(message: String) -> Self {
        Self::Kinded {
            kind: "recipe_invalid",
            exit: Exit::Input,
            message,
            hint: "correct the kind recipe",
            example: "jevify capabilities --json",
        }
    }
    /// `fill` could not write the status file `JEVIFY_STATUS_FILE` names. Nothing runs: jevify
    /// never starts a command while unable to record that it started one.
    pub fn status_file_unwritable(message: String) -> Self {
        Self::Kinded {
            kind: "status_file_unwritable",
            exit: Exit::Input,
            message,
            hint: "JEVIFY_STATUS_FILE must name a writable path inside an existing directory; nothing ran",
            example: "JEVIFY_STATUS_FILE=$(mktemp) jevify fill -- git switch '@{branch:the auth refactor}'",
        }
    }
    /// Whether a rejected request body named a size limit. The two readings of a 413, 422 or
    /// classifier 400 need different answers, and only the service's own text tells them apart.
    fn names_a_size_limit(message: &str) -> bool {
        [
            "too_long",
            "too_many",
            "too large",
            "token",
            "length",
            "size",
        ]
        .iter()
        .any(|needle| message.contains(needle))
    }
    pub fn exit(&self) -> Exit {
        match self {
            Self::Kinded { exit, .. } => *exit,
            Self::MissingKey | Self::BadKey(_) => Exit::Auth,
            Self::Unavailable(_) | Self::Deadline(_) | Self::Protocol(_) => Exit::Unavailable,
            Self::EmptyInput(_)
            | Self::InputTooLarge(_)
            | Self::RejectedRequest(..)
            | Self::Input(_) => Exit::Input,
            Self::Usage(_) => Exit::Usage,
            Self::Declined => Exit::Interrupted,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Kinded { kind, .. } => kind,
            Self::MissingKey => "missing_api_key",
            Self::BadKey(_) => "bad_api_key",
            // The kind, not exit 4 alone, tells callers whether retrying can help.
            Self::Deadline(_) => "api_deadline",
            Self::Unavailable(_) => "api_unavailable",
            Self::Protocol(_) => "api_protocol",
            Self::EmptyInput(_) => "empty_input",
            Self::InputTooLarge(_) => "input_too_large",
            Self::RejectedRequest(..) => "api_rejected_request",
            Self::Input(_) => "input",
            Self::Usage(_) => "usage",
            Self::Declined => "declined",
        }
    }
    pub fn hint(&self) -> &'static str {
        match self {
            Self::Kinded { hint, .. } => hint,
            Self::MissingKey => {
                "unset JEVIFY_BACKEND to run keyless through classifier.dev, or create a key at https://console.typesafe.ai/keys and export it in your shell profile; jevify never prints it"
            }
            Self::BadKey(_) => {
                "check the key at https://console.typesafe.ai/keys; `jevify health` verifies it"
            }
            Self::Deadline(_) => {
                "the work was cancelled, not refused: raise JEVIFY_DEADLINE, or split the input into smaller runs"
            }
            Self::Unavailable(_) => {
                "retry later; classifier.dev's free budget is $0.50 per IP per UTC day, with up to $0.01 per request; narrow the input, or get a TypeSafe key at https://console.typesafe.ai/keys and set TYPESAFE_API_KEY_FILE"
            }
            Self::Protocol(_) => {
                "the API may have changed, or JEVIFY_BASE_URL points at the wrong server; run `jevify health` and report the issue with `jevify --version`"
            }
            Self::EmptyInput(msg) if msg.starts_with("no unstaged changes") => {
                "nothing to stage: `git diff` is empty (untracked files are never staged by add)"
            }
            Self::EmptyInput(_) => "pipe text into jevify",
            Self::InputTooLarge(_) => "filter the input first, e.g. with rg or tail",
            // The service names a size limit or it does not; jevify says which reading its
            // text supports and never guesses "too large" for an input of a few bytes.
            Self::RejectedRequest(_, m) if Self::names_a_size_limit(m) => {
                "the input is over the API's budget, as the message says: filter it first, e.g. with rg or tail"
            }
            Self::RejectedRequest(..) => {
                "the API rejected the request body without naming a size limit; filter a large input first with rg or tail, and report a small one with `jevify --version`, since the request is then malformed"
            }
            Self::Input(_) => "check the input path and encoding",
            Self::Usage(_) => "see `jevify --help` or `jevify capabilities --json`",
            Self::Declined => "re-run with --yes to skip confirmation",
        }
    }
    pub fn example(&self) -> &'static str {
        match self {
            Self::Kinded { example, .. } => example,
            Self::MissingKey => "export TYPESAFE_API_KEY=...; jevify health",
            Self::EmptyInput(msg) if msg.starts_with("no unstaged changes") => {
                "jevify add \"finish the login flow\""
            }
            Self::EmptyInput(_) => "ls | jevify pick \"what I paid a streaming service\"",
            Self::Deadline(_) => {
                "JEVIFY_DEADLINE=1800 jevify filter 'reports a crash' < issues.txt"
            }
            Self::InputTooLarge(_) => "tail -n 20000 build.log | jevify why",
            Self::RejectedRequest(_, m) if Self::names_a_size_limit(m) => {
                "tail -n 20000 build.log | jevify why"
            }
            Self::RejectedRequest(..) => "jevify --version",
            _ => "jevify capabilities --json",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// One error per reachable kind, including every kind string a `Kinded` site in `src/`
    /// writes. `tests/agent.rs` scans the sources so a new literal cannot stay out of this list.
    fn representatives() -> Vec<JevifyError> {
        let kinded = |kind| JevifyError::Kinded {
            kind,
            exit: Exit::Input,
            message: "representative".into(),
            hint: "narrow the input",
            example: "head -n 100 input | jevify pick 'q'",
        };
        vec![
            kinded("too_many"),
            JevifyError::stdin_is_tty("terminal".into()),
            JevifyError::lister_failed("failed".into()),
            JevifyError::cannot_run("missing".into()),
            JevifyError::recipe_invalid("invalid".into()),
            JevifyError::status_file_unwritable("read-only".into()),
            JevifyError::MissingKey,
            JevifyError::BadKey(401),
            JevifyError::Unavailable("connection refused".into()),
            JevifyError::quota_exhausted("no credits", "add credits"),
            JevifyError::Deadline("600".into()),
            JevifyError::Protocol(String::new()),
            JevifyError::EmptyInput(""),
            JevifyError::InputTooLarge(String::new()),
            JevifyError::RejectedRequest(422, "state: input_too_long".into()),
            JevifyError::Input(String::new()),
            JevifyError::Usage(String::new()),
            JevifyError::Declined,
        ]
    }
    #[test]
    fn the_published_kind_table_is_exactly_what_the_errors_produce() {
        // Adding a variant makes this match non-exhaustive, so a new error cannot be written
        // without visiting `representatives` and `KINDS`.
        for e in representatives() {
            match e {
                JevifyError::Kinded { .. }
                | JevifyError::MissingKey
                | JevifyError::BadKey(_)
                | JevifyError::Unavailable(_)
                | JevifyError::Deadline(_)
                | JevifyError::Protocol(_)
                | JevifyError::EmptyInput(_)
                | JevifyError::InputTooLarge(_)
                | JevifyError::RejectedRequest(..)
                | JevifyError::Input(_)
                | JevifyError::Usage(_)
                | JevifyError::Declined => {}
            }
        }
        let mut produced: Vec<_> = representatives()
            .iter()
            .map(|e| (e.kind(), e.exit().code()))
            .collect();
        produced.sort_unstable();
        produced.dedup();
        let mut published: Vec<_> = JevifyError::KINDS
            .iter()
            .map(|(kind, exit)| (*kind, exit.code()))
            .collect();
        published.sort_unstable();
        assert_eq!(produced, published);
    }
}
