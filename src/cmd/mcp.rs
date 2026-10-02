//! `jevify mcp`: a stdio MCP server that offers `why`, `is` and `pick` as tools.
//!
//! JSON-RPC 2.0, one message per line on stdin and stdout, nothing else on stdout; logs go
//! to stderr. The server speaks both eras of the protocol: the `initialize` handshake of
//! revision 2025-11-25 and earlier, and the per-request `_meta` of revision 2026-07-28, which
//! `server/discover` announces. It keeps no state between requests: every tool call loads its
//! own configuration from the environment and reports its own `meta`.
//!
//! A tool call answers with the jevify envelope as `structuredContent` and a short text. An
//! abstention (exit 3) is an ordinary result whose `data.shortlist` names the nearest
//! candidates; a jevify error is a tool execution error (`isError`) whose envelope carries
//! `error.kind`. `pick` only selects: no tool starts a user command.

use crate::cli::GlobalOpts;
use crate::cmd::Outcome;
use crate::config::Config;
use crate::exit::JevifyError;
use crate::output::{Envelope, ErrorBody};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::time::Instant;

/// The latest revision with an `initialize` handshake, and the earlier ones the server echoes.
pub const LEGACY_VERSION: &str = "2025-11-25";
const LEGACY_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// The revision whose requests carry their protocol version in `_meta`.
pub const MODERN_VERSION: &str = "2026-07-28";

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const INSTRUCTIONS: &str = "jevify selects existing text by meaning and never generates it. \
    why: the line that explains a failure in build, test or CI output (path or text). \
    is: a yes, no or unsure verdict on one statement about a text. \
    pick: the one item a description means, among supplied items or the real branches, \
    commits, files, PRs, CI runs or installed tools of a directory; it selects and runs nothing. \
    Branch on structuredContent.exit_code: 0 found or yes, 1 no, 3 nothing fits or unsure \
    (data.shortlist names the nearest candidates, which are not answers). An isError result \
    carries structuredContent.error.kind.";

/// The kinds `pick` lists: the tool's name for each, and jevify's.
const FROM_KINDS: [(&str, &str); 6] = [
    ("commit", "commit"),
    ("branch", "branch"),
    ("file", "file"),
    ("tool", "tool"),
    ("pr", "pr"),
    ("run", "ci-run"),
];

/// Serves until stdin closes. Reads on the blocking pool so the runtime keeps driving the
/// backend requests of the call in progress; calls run one at a time, in order.
pub async fn serve(g: &GlobalOpts) -> Result<Outcome, JevifyError> {
    loop {
        let line = tokio::task::spawn_blocking(|| {
            let mut bytes = Vec::new();
            std::io::stdin()
                .lock()
                .read_until(b'\n', &mut bytes)
                .map(|n| (n > 0).then_some(bytes))
        })
        .await
        .map_err(|e| JevifyError::Input(e.to_string()))?
        .map_err(|e| JevifyError::Input(format!("stdin: {e}")))?;
        let Some(line) = line else {
            break;
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if let Some(reply) = handle(g, &line).await {
            let mut stdout = std::io::stdout().lock();
            // `serde_json` escapes every line break inside strings: one message, one line.
            writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .map_err(|e| JevifyError::Input(format!("stdout: {e}")))?;
        }
    }
    Ok(Outcome {
        exit: crate::exit::Exit::Ok,
        data: json!({}),
        human: Vec::new(),
        exec: None,
    })
}

/// Which revision of the protocol a request speaks, from its `_meta`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    Legacy,
    Modern,
}

/// The reply to one line, or none for a notification.
async fn handle(g: &GlobalOpts, line: &[u8]) -> Option<String> {
    let message: Value = match std::str::from_utf8(line)
        .ok()
        .and_then(|text| serde_json::from_str(text).ok())
    {
        Some(message) => message,
        None => return Some(error(Value::Null, PARSE_ERROR, "Parse error", None)),
    };
    let Some(object) = message.as_object() else {
        // Batches left the protocol in 2025-06-18; an array is not one request.
        return Some(error(Value::Null, INVALID_REQUEST, "Invalid Request", None));
    };
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let method = object.get("method").and_then(Value::as_str);
    let valid_id = id.is_string() || id.as_i64().is_some();
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || method.is_none()
        || (!id.is_null() && !valid_id)
    {
        let id = if valid_id { id } else { Value::Null };
        return Some(error(id, INVALID_REQUEST, "Invalid Request", None));
    }
    let method = method.unwrap_or_default();
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    if id.is_null() {
        // A notification: `notifications/initialized`, `notifications/cancelled`, any other.
        if !method.starts_with("notifications/") {
            eprintln!("jevify mcp: ignoring notification {method}");
        }
        return None;
    }
    let meta = &params["_meta"];
    let era = match meta.get(META_VERSION) {
        None => Era::Legacy,
        Some(Value::String(version)) if version == MODERN_VERSION => Era::Modern,
        Some(requested) => {
            return Some(error(
                id,
                UNSUPPORTED_PROTOCOL_VERSION,
                "Unsupported protocol version",
                Some(json!({"supported": [MODERN_VERSION], "requested": requested})),
            ));
        }
    };
    if era == Era::Modern && !meta[META_CLIENT_CAPABILITIES].is_object() {
        return Some(error(
            id,
            INVALID_PARAMS,
            &format!("_meta.{META_CLIENT_CAPABILITIES} is required"),
            None,
        ));
    }
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "server/discover" => Ok(json!({
            "supportedVersions": [MODERN_VERSION, LEGACY_VERSION],
            "capabilities": {"tools": {}},
            "instructions": INSTRUCTIONS,
            "_meta": {META_SERVER_INFO: server_info()},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => call(g, &params).await,
        _ => Err((METHOD_NOT_FOUND, format!("Method not found: {method}"))),
    };
    Some(match result {
        Ok(mut result) => {
            if era == Era::Modern {
                result["resultType"] = "complete".into();
                result["_meta"][META_SERVER_INFO] = server_info();
            }
            json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
        }
        Err((code, message)) => error(id, code, &message, None),
    })
}

fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> String {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error}).to_string()
}

fn server_info() -> Value {
    json!({"name": "jevify", "title": "jevify", "version": env!("CARGO_PKG_VERSION"), "websiteUrl": env!("CARGO_PKG_HOMEPAGE")})
}

/// The legacy handshake: a known revision is echoed, any other gets the latest known one.
fn initialize(params: &Value) -> Value {
    let requested = params["protocolVersion"].as_str().unwrap_or_default();
    let version = if LEGACY_VERSIONS.contains(&requested) {
        requested
    } else {
        LEGACY_VERSION
    };
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS,
    })
}

/// The tool definitions, in a fixed order.
pub fn tools() -> Vec<Value> {
    let read_only = json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true});
    vec![
        json!({
            "name": "why",
            "title": "Find the line that explains a failure",
            "description": "Point at the line that states the root cause in the output of a failed build, test run or CI job: a file path or the text itself, one of the two. Returns the jevify envelope as structuredContent: data.causes[{line, text, p, context[]}] on exit_code 0; exit_code 3 when no line looks like a failure, with data.shortlist naming the nearest lines, which are not answers. Saves the raw input for seven days unless JEVIFY_NO_SAVE=1.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path of the log to read"},
                    "text": {"type": "string", "description": "The output itself"}
                },
                "additionalProperties": false
            },
            "annotations": read_only,
        }),
        json!({
            "name": "is",
            "title": "Check one statement against a text",
            "description": "Answer whether a text satisfies one literal statement, such as \"the customer asks for a refund\": structuredContent.data.verdict is yes, no or unsure, exit_code 0, 1 or 3, with the probability in data.p. The text comes as context or as context_path, one of the two. No counting, arithmetic, dates or quality judgments; oversized text is unsure without a request.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "statement": {"type": "string", "description": "What must be true of the text; write it so that yes means act"},
                    "context": {"type": "string", "description": "The text to judge"},
                    "context_path": {"type": "string", "description": "Path of a file holding the text to judge"}
                },
                "required": ["statement"],
                "additionalProperties": false
            },
            "annotations": read_only,
        }),
        json!({
            "name": "pick",
            "title": "Find the item a description means",
            "description": "Select the one item a description means, among supplied items or the real things of a kind in a directory: commit, branch, file, tool (installed commands), pr, run (CI runs). Selects only; starts no command. structuredContent.data.matches[{text, p}] on exit_code 0; exit_code 3 when nothing fits or two fit equally, with data.shortlist naming the nearest candidates, which are not answers. Describe what the item itself says, not your goal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "description": {"type": "string", "description": "The item you want, e.g. \"the branch with the payment timeout fix\""},
                    "items": {"type": "array", "items": {"type": "string"}, "description": "The candidates, one per entry; the match is returned verbatim"},
                    "from_kind": {"type": "string", "enum": FROM_KINDS.iter().map(|(name, _)| *name).collect::<Vec<_>>(), "description": "List candidates of this kind in cwd instead of items"},
                    "cwd": {"type": "string", "description": "The directory from_kind lists; the server's own directory otherwise"}
                },
                "required": ["description"],
                "additionalProperties": false
            },
            "annotations": read_only,
        }),
    ]
}

/// A tool call: its envelope as `structuredContent`, a short text, `isError` on a jevify error.
async fn call(g: &GlobalOpts, params: &Value) -> Result<Value, (i64, String)> {
    let Some(name) = params["name"].as_str() else {
        return Err((INVALID_PARAMS, "params.name is required".into()));
    };
    if !["why", "is", "pick"].contains(&name) {
        return Err((INVALID_PARAMS, format!("Unknown tool: {name}")));
    }
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Value::Object(Default::default()),
        Some(Value::Object(arguments)) => Value::Object(arguments.clone()),
        Some(_) => return Err((INVALID_PARAMS, "params.arguments must be an object".into())),
    };
    let start = Instant::now();
    let (mut meta, outcome) = match Config::load(g) {
        Ok(ctx) => {
            let outcome = match name {
                "why" => why(&ctx, &arguments).await,
                "is" => is(&ctx, &arguments).await,
                _ => pick(&ctx, &arguments).await,
            };
            (ctx.meta(), outcome)
        }
        Err(e) => (crate::output::Meta::default(), Err(e)),
    };
    meta.elapsed_ms = start.elapsed().as_millis();
    meta.decision.verb = name.into();
    Ok(match outcome {
        Ok(out) => {
            let text = summary(name, &out);
            let envelope = Envelope {
                ok: true,
                command: name,
                version: env!("CARGO_PKG_VERSION"),
                exit_code: out.exit.code(),
                data: out.data,
                meta,
                error: None,
            };
            json!({"content": [{"type": "text", "text": text}], "structuredContent": envelope, "isError": false})
        }
        Err(e) => {
            let envelope = Envelope {
                ok: false,
                command: name,
                version: env!("CARGO_PKG_VERSION"),
                exit_code: e.exit().code(),
                data: Value::Null,
                meta,
                error: Some(ErrorBody {
                    kind: e.kind(),
                    message: e.to_string(),
                    hint: e.hint().to_owned(),
                    example: e.example().to_owned(),
                }),
            };
            let text = format!("{}: {}; hint: {}", e.kind(), e, e.hint());
            json!({"content": [{"type": "text", "text": text}], "structuredContent": envelope, "isError": true})
        }
    })
}

/// The one-line form of a result: what a client without `structuredContent` reads.
fn summary(name: &str, out: &Outcome) -> String {
    let data = &out.data;
    let listed = |items: &Value, text: fn(&Value) -> String| {
        items
            .as_array()
            .into_iter()
            .flatten()
            .map(text)
            .collect::<Vec<_>>()
            .join("; ")
    };
    let scored = |c: &Value| {
        let text = c["text"].as_str().unwrap_or_default();
        match c["line"].as_u64() {
            Some(n) => format!("line {n}: {text} ({:.2})", c["p"].as_f64().unwrap_or(0.0)),
            None => format!("{text} ({:.2})", c["p"].as_f64().unwrap_or(0.0)),
        }
    };
    let abstained = |what: &str| {
        let nearest = listed(&data["shortlist"], scored);
        let nearest = if nearest.is_empty() {
            "none".to_owned()
        } else {
            nearest
        };
        let hint = data["hint"]
            .as_str()
            .map(|h| format!("; hint: {h}"))
            .unwrap_or_default();
        format!("{what} (exit 3); nearest (not chosen): {nearest}{hint}")
    };
    match (name, out.exit.code()) {
        ("why", 0) => listed(&data["causes"], |c| {
            format!(
                "line {}: {}",
                c["line"],
                c["text"].as_str().unwrap_or_default()
            )
        }),
        ("why", _) => abstained("no line explains a failure"),
        ("is", _) => match data["p"].as_f64() {
            Some(p) => format!(
                "{} (p {p:.2})",
                data["verdict"].as_str().unwrap_or("unsure")
            ),
            None => format!(
                "{}: {}",
                data["verdict"].as_str().unwrap_or("unsure"),
                data["reason"].as_str().unwrap_or("not judged")
            ),
        },
        ("pick", 0) => listed(&data["matches"], |m| {
            m["text"].as_str().unwrap_or_default().to_owned()
        }),
        (_, _) => abstained(&format!(
            "nothing fits: {}",
            data["reason"].as_str().unwrap_or("no_match")
        )),
    }
}

fn string(arguments: &Value, key: &str) -> Result<Option<String>, JevifyError> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(JevifyError::Usage(format!("{key} must be a string"))),
    }
}

/// Reads a whole file on the blocking pool, within the stdin size limit.
async fn read_file(path: String) -> Result<Vec<u8>, JevifyError> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let file =
            std::fs::File::open(&path).map_err(|e| JevifyError::Input(format!("{path}: {e}")))?;
        let mut bytes = Vec::new();
        file.take(crate::input::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| JevifyError::Input(format!("{path}: {e}")))?;
        if bytes.len() > crate::input::MAX_BYTES {
            return Err(JevifyError::InputTooLarge(format!(
                "{path} exceeds {} MiB",
                crate::input::MAX_BYTES / 1024 / 1024
            )));
        }
        Ok(bytes)
    })
    .await
    .map_err(|e| JevifyError::Input(e.to_string()))?
}

async fn why(ctx: &Config, arguments: &Value) -> Result<Outcome, JevifyError> {
    let bytes = match (string(arguments, "path")?, string(arguments, "text")?) {
        (Some(path), None) => read_file(path).await?,
        (None, Some(text)) => text.into_bytes(),
        _ => {
            return Err(JevifyError::Usage(
                "why takes path or text, one of the two".into(),
            ));
        }
    };
    if bytes.is_empty() {
        return Err(JevifyError::EmptyInput("the input was empty"));
    }
    crate::cmd::why::analyse(ctx, bytes, 3, 1, false).await
}

async fn is(ctx: &Config, arguments: &Value) -> Result<Outcome, JevifyError> {
    let Some(statement) = string(arguments, "statement")?.filter(|s| !s.trim().is_empty()) else {
        return Err(JevifyError::Usage("statement is required".into()));
    };
    let lines = match (
        string(arguments, "context")?,
        string(arguments, "context_path")?,
    ) {
        (Some(text), None) => {
            let lines = crate::input::split_lines(&text);
            if lines.iter().all(|line| line.trim().is_empty()) {
                return Err(JevifyError::EmptyInput("context was empty"));
            }
            lines
        }
        (None, Some(path)) => {
            let path = std::path::PathBuf::from(path);
            tokio::task::spawn_blocking(move || crate::cmd::is::read_context(&path))
                .await
                .map_err(|e| JevifyError::Input(e.to_string()))??
        }
        _ => {
            return Err(JevifyError::Usage(
                "is takes context or context_path, one of the two".into(),
            ));
        }
    };
    crate::cmd::is::judge(ctx, &[statement], lines, 0.15).await
}

async fn pick(ctx: &Config, arguments: &Value) -> Result<Outcome, JevifyError> {
    let Some(description) = string(arguments, "description")?.filter(|s| !s.trim().is_empty())
    else {
        return Err(JevifyError::Usage("description is required".into()));
    };
    let items = match arguments.get("items") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| JevifyError::Usage("items must be strings".into()))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err(JevifyError::Usage("items must be an array".into())),
    };
    let from_kind = string(arguments, "from_kind")?;
    let cwd = string(arguments, "cwd")?;
    match (items, from_kind) {
        (Some(items), None) => {
            if cwd.is_some() {
                return Err(JevifyError::Usage("cwd applies to from_kind only".into()));
            }
            // NUL-separated, so an item holding a line break stays one record.
            let bytes = items.join("\0").into_bytes();
            crate::cmd::pick::rank(
                ctx,
                &description,
                1,
                false,
                crate::records::Split::Nul,
                false,
                bytes,
            )
            .await
        }
        (None, Some(kind)) => {
            let Some((_, jevify_kind)) = FROM_KINDS.iter().find(|(name, _)| *name == kind) else {
                return Err(JevifyError::Usage(format!(
                    "unknown from_kind {kind:?}; one of {}",
                    FROM_KINDS
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            };
            // The lister runs in the process's directory; the calls run one at a time, so
            // the directory is set for this one and put back after it.
            let previous = match &cwd {
                Some(dir) => {
                    let previous =
                        std::env::current_dir().map_err(|e| JevifyError::Input(e.to_string()))?;
                    std::env::set_current_dir(dir)
                        .map_err(|e| JevifyError::Input(format!("cwd {dir}: {e}")))?;
                    Some(previous)
                }
                None => None,
            };
            let outcome = crate::cmd::pick::from_kind(ctx, &description, 1, jevify_kind).await;
            if let Some(previous) = previous {
                if let Err(e) = std::env::set_current_dir(&previous) {
                    eprintln!("jevify mcp: cannot return to {}: {e}", previous.display());
                }
            }
            outcome
        }
        _ => Err(JevifyError::Usage(
            "pick takes items or from_kind, one of the two".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_and_from_kinds_map_to_real_verbs_and_kinds() {
        let names: Vec<_> = tools()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["why", "is", "pick"]);
        let env = crate::source::Env::from_process(std::time::Duration::from_secs(1));
        for (_, kind) in FROM_KINDS {
            assert!(
                crate::source::lookup(kind, &env).unwrap().is_some(),
                "{kind}"
            );
        }
        let enumerated = &tools()[2]["inputSchema"]["properties"]["from_kind"]["enum"];
        assert_eq!(enumerated.as_array().unwrap().len(), FROM_KINDS.len());
    }

    #[test]
    fn initialize_echoes_a_known_revision_and_offers_the_latest_otherwise() {
        for (requested, expected) in [
            ("2025-11-25", "2025-11-25"),
            ("2024-11-05", "2024-11-05"),
            ("1.0", "2025-11-25"),
        ] {
            let result = initialize(&json!({"protocolVersion": requested}));
            assert_eq!(result["protocolVersion"], expected);
            assert!(result["capabilities"]["tools"].is_object());
        }
        assert_eq!(initialize(&Value::Null)["protocolVersion"], LEGACY_VERSION);
    }
}
