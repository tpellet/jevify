#![deny(unsafe_code)]

pub mod argv;
pub mod cli;
pub mod cmd;
pub mod config;
pub mod exit;
pub mod gitdiff;
pub mod input;
pub mod inventory;
pub mod jev;
pub mod manpage;
pub mod marker;
pub mod output;
pub mod records;
pub mod save;
pub mod source;
pub mod tournament;

use clap::Parser;
use cli::{Cli, Cmd};
use exit::{Exit, JevifyError};
use output::{Envelope, ErrorBody, Format, Meta};
use std::ffi::OsString;
use std::io::Write;
use std::time::Instant;

const VERBS: [&str; 10] = [
    "fill",
    "pick",
    "why",
    "filter",
    "label",
    "is",
    "add",
    "capabilities",
    "health",
    "init",
];

/// What bare `jevify` prints: enough to make a first call, in about 130 tokens. `--help` has the rest.
pub const QUICK_START: &str = concat!(
    "jevify ",
    env!("CARGO_PKG_VERSION"),
    r#": answer questions about text you already have. Selects, never generates.
  jevify fill -- CMD '@{kind:description}' resolve an argument and run CMD
  <list> | jevify pick "<description>"    find one line by meaning
  <cmd> 2>&1 | jevify why                 find the line that caused a failure
  jevify is "<statement>" < file          yes / no / unsure as exit code 0 / 1 / 3
  jevify pick --from tool "<task>"       find the installed command for a task
  <list> | jevify filter "<statement>"    keep matching records
  <list> | jevify label a,b,c             tag each record with a label
  jevify add --dry-run "<topic>"          stage only the git changes about a topic
Add --json for one JSON object on stdout. No key needed.
Exit: 0 ok, 1 no, 2 usage, 3 nothing fits or unsure, 4 API unavailable, 5 auth, 6 input.
More: jevify <verb> --help | agents: jevify capabilities --json, jevify init agents
"#
);

pub fn main_exit() -> i32 {
    // Bare `jevify` stays a usage error (exit 2, stderr), as it was with clap's full help.
    if std::env::args_os().len() == 1 {
        eprint!("{QUICK_START}");
        return Exit::Usage.code();
    }
    // A verb option written before the verb moves after it, where clap reads it.
    let args = argv::reorder(&std::env::args_os().skip(1).collect::<Vec<_>>());
    let name = raw_command(&args);
    let removed = if name == "why" && args.iter().any(|a| a == "--") {
        Some(("why", "use CMD 2>&1 | jevify why"))
    } else {
        None
    };
    if let Some((command, message)) = removed {
        if let Some(format) = machine_format(&args) {
            return report_error(
                format,
                command,
                &JevifyError::Usage(message.into()),
                Meta::default(),
            );
        }
        eprintln!("jevify: {message}");
        return Exit::Usage.code();
    }
    let program = std::env::args_os()
        .next()
        .unwrap_or_else(|| OsString::from("jevify"));
    let cli = match Cli::try_parse_from(std::iter::once(program).chain(args.iter().cloned())) {
        Ok(c) => c,
        Err(e) => {
            // A usage error under --json must still be exactly one envelope, not clap's text.
            if e.use_stderr() {
                let message = clap_message(&e);
                if name == "fill" {
                    return report_fill_error(
                        machine_format(&args).unwrap_or(Format::Human),
                        &JevifyError::Kinded {
                            kind: "usage",
                            exit: Exit::Usage,
                            message,
                            hint: "put the command after --: jevify fill -- git switch '@{branch:the auth refactor}'",
                            example: "jevify fill -- git switch '@{branch:the auth refactor}'",
                        },
                        Meta::default(),
                        raw_quiet(&args),
                    );
                }
                let unknown_verb = e.kind() == clap::error::ErrorKind::InvalidSubcommand;
                if VERBS.contains(&name) || machine_format(&args).is_some() || unknown_verb {
                    let format = machine_format(&args).unwrap_or(Format::Human);
                    let name = if VERBS.contains(&name) {
                        name
                    } else {
                        "jevify"
                    };
                    return report_error(
                        format,
                        name,
                        &JevifyError::Usage(message),
                        Meta::default(),
                    );
                }
            }
            if let Err(error) = e.print() {
                return if e.use_stderr() {
                    Exit::Usage.code()
                } else {
                    stdout_error(error)
                };
            }
            return if e.use_stderr() {
                Exit::Usage.code()
            } else {
                Exit::Ok.code()
            };
        }
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(run_cli(cli))
}

/// Clap's error as one line: its first line, with the missing arguments it lists after a colon.
fn clap_message(e: &clap::Error) -> String {
    let text = e.to_string();
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines
        .next()
        .unwrap_or("usage error")
        .trim_start_matches("error: ");
    if first.ends_with(':') {
        let listed: Vec<&str> = lines.take_while(|l| !l.starts_with("Usage:")).collect();
        format!("{first} {}", listed.join(", "))
    } else {
        first.to_owned()
    }
}

/// Clap failed before a `Cli` existed, so the requested machine format is read from the raw args.
fn machine_format(args: &[OsString]) -> Option<Format> {
    args.iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json" || arg == "--robot")
        .then_some(Format::Json)
}

fn raw_command(args: &[OsString]) -> &str {
    let mut args = args.iter().take_while(|arg| *arg != "--");
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--threshold" | "-t" | "--model") => {
                args.next();
            }
            Some(value) if value.starts_with('-') => {}
            Some(value) => return value,
            None => return "jevify",
        }
    }
    "jevify"
}

fn raw_quiet(args: &[OsString]) -> bool {
    args.iter().take_while(|a| *a != "--").any(|a| a == "-q")
}

fn command_name(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::Fill { .. } => "fill",
        Cmd::Pick { .. } => "pick",
        Cmd::Why { .. } => "why",
        Cmd::Filter { .. } => "filter",
        Cmd::Label { .. } => "label",
        Cmd::Is { .. } => "is",
        Cmd::Add { .. } => "add",
        Cmd::Capabilities => "capabilities",
        Cmd::Health => "health",
        Cmd::Init { .. } => "init",
    }
}

async fn run_cli(cli: Cli) -> i32 {
    let start = Instant::now();
    let format = cli.g.format();
    let name = command_name(&cli.cmd);
    let quiet = matches!(cli.cmd, Cmd::Fill { quiet: true, .. });
    let report = |e: &JevifyError, meta| {
        if name == "fill" {
            report_fill_error(format, e, meta, quiet)
        } else {
            report_error(format, name, e, meta)
        }
    };
    // -C DIR: everything that follows runs as if jevify had been started in DIR.
    if let Cmd::Fill {
        repo: Some(dir), ..
    }
    | Cmd::Pick {
        repo: Some(dir), ..
    } = &cli.cmd
    {
        if let Err(e) = std::env::set_current_dir(dir) {
            return report(
                &JevifyError::Input(format!("-C {}: {e}", dir.display())),
                Meta::default(),
            );
        }
    }
    let ctx = match config::Config::load(&cli.g) {
        Ok(c) => c,
        Err(e) => return report(&e, Meta::default()),
    };
    let result = dispatch(&cli, &ctx).await;
    let mut meta = ctx.meta();
    meta.elapsed_ms = start.elapsed().as_millis();
    meta.decision.verb = name.into();
    match result {
        Ok(out) => {
            if format == Format::Human {
                if let Err(e) = std::io::stdout().lock().write_all(&out.human) {
                    return stdout_error(e);
                }
                if cli.g.verbose && name != "fill" {
                    eprintln!("jevify: {}", out.data);
                    eprintln!(
                        "jevify: {} ms, {} requests, {} cached",
                        meta.elapsed_ms, meta.requests, meta.cache_hits,
                    );
                }
            } else {
                let env = Envelope {
                    // `ok` means "jevify itself reached the end without an error of its own",
                    // so it is true here for every verb outcome: a yes, an `is` that answers no
                    // (exit 1), and an abstention (exit 3), where `data` carries a reason and
                    // not a result. It is not the field to branch on, and `capabilities`,
                    // ROBOT_MODE.md and the agents guide all say so where a reader meets it.
                    // `exit_code` is the branch.
                    ok: true,
                    command: name,
                    version: env!("CARGO_PKG_VERSION"),
                    exit_code: out.exit.code(),
                    data: out.data,
                    meta: meta.clone(),
                    error: None,
                };
                if let Err(e) = writeln!(
                    std::io::stdout().lock(),
                    "{}",
                    output::render(format, &env).expect("render envelope")
                ) {
                    return stdout_error(e);
                }
            }
            if let Some(exec) = out.exec {
                if let Err(e) = std::io::stdout().flush() {
                    return stdout_error(e);
                }
                return report(&exec_command(&exec), meta);
            }
            out.exit.code()
        }
        Err(e) => report(&e, meta),
    }
}

fn build_command(exec: &cmd::Exec) -> std::process::Command {
    let mut command = std::process::Command::new(&exec.argv[0]);
    command.args(&exec.argv[1..]);
    if exec.stdin_null {
        command.stdin(std::process::Stdio::null());
    }
    command
}

fn exec_command(exec: &cmd::Exec) -> JevifyError {
    use std::os::unix::process::CommandExt;
    let error = build_command(exec).exec();
    JevifyError::cannot_run(format!("{}: {error}", exec.argv[0].to_string_lossy()))
}

/// The hint and the example of an error: built from the caller's own argv when jevify can
/// correct it, the error's static pair otherwise.
fn advice(e: &JevifyError) -> (String, String, bool) {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    match argv::advice(&args, e, &cwd) {
        Some((hint, example)) => (hint, example, true),
        None => (e.hint().to_owned(), e.example().to_owned(), false),
    }
}

fn fill_error_text(e: &JevifyError, quiet: bool, advice: &(String, String, bool)) -> String {
    let mut text = format!(
        "jevify fill: not run: {}: {}\n",
        e.kind(),
        output::status_escape(&e.to_string())
    );
    if !quiet {
        let (hint, example, tailored) = advice;
        let line = if *tailored {
            format!("{hint}; try: {example}")
        } else {
            hint.clone()
        };
        text.push_str(&format!("jevify fill: {}\n", output::status_escape(&line)));
    }
    text
}

fn report_fill_error(format: Format, e: &JevifyError, meta: Meta, quiet: bool) -> i32 {
    if format == Format::Human {
        eprint!("{}", fill_error_text(e, quiet, &advice(e)));
        e.exit().code()
    } else {
        report_error(format, "fill", e, meta)
    }
}

fn report_error(format: Format, name: &str, e: &JevifyError, mut meta: Meta) -> i32 {
    // A usage error under --json still names the verb it was decided for.
    meta.decision.verb = name.into();
    let (hint, example, _) = advice(e);
    if format == Format::Human {
        let prefix = if name == "jevify" {
            "jevify".to_owned()
        } else {
            format!("jevify {name}")
        };
        eprintln!(
            "{prefix}: error: {}\n  hint: {hint}\n  try:  {example}",
            e.to_string().trim_end()
        );
        if let Some(id) = &meta.request_id {
            eprintln!("  request: {id}");
        }
    } else {
        let env = Envelope {
            ok: false,
            command: name,
            version: env!("CARGO_PKG_VERSION"),
            exit_code: e.exit().code(),
            data: serde_json::Value::Null,
            meta,
            error: Some(ErrorBody {
                kind: e.kind(),
                message: e.to_string(),
                hint,
                example,
            }),
        };
        if let Err(e) = writeln!(
            std::io::stdout().lock(),
            "{}",
            output::render(format, &env).expect("render envelope")
        ) {
            return stdout_error(e);
        }
    }
    e.exit().code()
}

fn stdout_error(error: std::io::Error) -> i32 {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        Exit::Ok.code()
    } else {
        let _ = writeln!(std::io::stderr().lock(), "jevify: output error: {error}");
        Exit::Input.code()
    }
}

/// Every verb is wired here once (Task 1). Later tasks replace stub bodies in `cmd/*.rs`
/// and never edit this function.
async fn dispatch(cli: &Cli, ctx: &config::Config) -> Result<cmd::Outcome, JevifyError> {
    let machine = cli.g.format() != Format::Human;
    match &cli.cmd {
        Cmd::Fill {
            dry_run,
            quiet,
            candidates,
            context,
            field,
            key,
            nul,
            para,
            cmd,
            repo: _,
        } => {
            cmd::fill::run(
                ctx,
                cmd::fill::FillFlags {
                    dry_run: *dry_run,
                    quiet: *quiet,
                    candidates: candidates.clone(),
                    context: context.clone(),
                    field: *field,
                    key: key.clone(),
                    split: split(*nul, *para),
                },
                cmd,
                machine,
            )
            .await
        }
        Cmd::Pick {
            from,
            intent,
            top,
            index,
            files,
            nul,
            para,
            repo: _,
        } => {
            cmd::pick::run(
                ctx,
                &described(intent, "pick")?,
                *top,
                *index,
                split(*nul, *para),
                *files,
                from.as_deref(),
            )
            .await
        }
        Cmd::Why {
            context,
            top,
            no_save,
        } => cmd::why::run(ctx, *context, *top, *no_save).await,
        Cmd::Filter {
            statement,
            invert,
            count,
            strict,
            nul,
            para,
            files,
            no_save,
        } => {
            cmd::filter::run(
                ctx,
                &described(statement, "filter")?,
                cmd::filter::FilterFlags {
                    invert: *invert,
                    count: *count,
                    strict: *strict,
                    split: split(*nul, *para),
                    files: *files,
                    no_save: *no_save,
                },
                machine,
            )
            .await
        }
        Cmd::Label {
            labels,
            nul,
            para,
            files,
        } => {
            cmd::label::run(
                ctx,
                labels.0.clone(),
                cmd::label::LabelFlags {
                    split: split(*nul, *para),
                    files: *files,
                },
                machine,
            )
            .await
        }
        Cmd::Is {
            statements,
            context,
            band,
        } => {
            let (statements, unquoted) = cli::statements(statements);
            if unquoted {
                eprintln!(
                    "jevify is: judging {} one-word statements separately; quote a sentence to judge it as one: jevify is '{}'",
                    statements.len(),
                    statements.join(" ")
                );
            }
            cmd::is::run(ctx, &statements, context.as_deref(), *band).await
        }
        Cmd::Add {
            topic,
            yes,
            dry_run,
        } => cmd::add::run(ctx, &described(topic, "add")?, *yes, *dry_run, machine).await,
        Cmd::Capabilities => Ok(cmd::agent::capabilities()),
        Cmd::Health => cmd::agent::health(ctx).await,
        Cmd::Init { shell } => Ok(cmd::agent::init(*shell)),
    }
}

/// The free text of a verb, from the words the caller wrote; nothing left is a usage error.
fn described(words: &[String], verb: &str) -> Result<String, JevifyError> {
    cli::words(words).ok_or_else(|| {
        JevifyError::Usage(format!(
            "{verb} needs a description, not only `-`: stdin is already the default source"
        ))
    })
}

fn split(nul: bool, para: bool) -> records::Split {
    if nul {
        records::Split::Nul
    } else if para {
        records::Split::Para
    } else {
        records::Split::Lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exec_runs_argv_zero_literally_and_a_missing_program_cannot_run() {
        use std::os::unix::ffi::OsStringExt;
        let exec = cmd::Exec {
            argv: vec![
                "/nonexistent-jevify-command".into(),
                "b c".into(),
                OsString::from_vec(vec![0xff]),
            ],
            stdin_null: true,
        };
        let command = build_command(&exec);
        assert_eq!(command.get_program(), &exec.argv[0]);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            exec.argv[1..].iter().collect::<Vec<_>>()
        );
        let error = exec_command(&exec);
        assert_eq!((error.exit().code(), error.kind()), (6, "cannot_run"));
    }
}
