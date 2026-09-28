use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    ops::Range,
    os::unix::ffi::{OsStrExt, OsStringExt},
};

#[derive(Debug, Clone)]
pub struct Arg {
    /// Original bytes, including marker spellings and literal escapes.
    pub literal: OsString,
    pub markers: Vec<Marker>,
}

#[derive(Debug, Clone)]
pub struct Marker {
    pub kind: String,
    pub description: String,
    pub span: Range<usize>,
    pub argv_index: usize,
    /// Adjacent literal segments, with `@@{` decoded, excluding other markers.
    pub prefix: OsString,
    pub suffix: OsString,
    pub opens_argument: bool,
    pub options: Vec<String>,
    pub flag: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MarkerError(pub String);

pub fn parse(argv: &[OsString]) -> Result<Vec<Arg>, MarkerError> {
    let mut args = Vec::with_capacity(argv.len());
    let mut count = 0;
    for (argv_index, arg) in argv.iter().enumerate() {
        let bytes = arg.as_bytes();
        let mut markers = Vec::new();
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos..].starts_with(b"@@{") {
                pos += 3;
                continue;
            }
            let Some(body_start) = marker_start(bytes, pos) else {
                pos += 1;
                continue;
            };
            let start = pos;
            let kind = std::str::from_utf8(&bytes[start + 2..body_start - 1])
                .expect("marker_start accepts ASCII kinds only");
            pos = body_start;
            while pos < bytes.len() {
                if bytes[pos..].starts_with(b"\\}") {
                    pos += 2;
                } else if bytes[pos] == b'}' {
                    break;
                } else {
                    pos += 1;
                }
            }
            if pos == bytes.len() {
                return Err(literal_error(
                    "unterminated marker; escape it as a literal",
                    arg,
                ));
            }
            let end = pos + 1;
            let body = std::str::from_utf8(&bytes[body_start..pos]).map_err(|_| {
                literal_error("marker BODY must be UTF-8; escape it as a literal", arg)
            })?;
            let body = strip_quotes(body);
            if body.is_empty() {
                return Err(literal_error(
                    "marker BODY is empty; escape it as a literal",
                    arg,
                ));
            }
            let mut options = Vec::new();
            let mut flag = None;
            let question = if matches!(kind, "one" | "flag") {
                let Some(colon) = delimiter(body.as_bytes(), b':') else {
                    return Err(literal_error(
                        "expected options or flag followed by ':question'; escape as a literal",
                        arg,
                    ));
                };
                let head = &body[..colon];
                if kind == "one" {
                    let mut rest = head;
                    while let Some(pipe) = delimiter(rest.as_bytes(), b'|') {
                        options.push(unescape_body(&rest[..pipe], true));
                        rest = &rest[pipe + 1..];
                    }
                    options.push(unescape_body(rest, true));
                    let unique: HashSet<_> = options.iter().collect();
                    if options.len() < 2 || unique.len() != options.len() {
                        return Err(literal_error(
                            "one needs at least two distinct options; escape as a literal",
                            arg,
                        ));
                    }
                } else {
                    if start != 0 || end != bytes.len() {
                        return Err(literal_error(
                            "flag must occupy a whole argument; escape as a literal",
                            arg,
                        ));
                    }
                    let text = unescape_body(head, true);
                    if !text.starts_with('-') {
                        return Err(literal_error(
                            "FLAGTEXT must start with '-'; escape as a literal",
                            arg,
                        ));
                    }
                    flag = Some(text);
                }
                &body[colon + 1..]
            } else {
                body
            };
            if question.is_empty() {
                return Err(literal_error(
                    "marker question is empty; escape as a literal",
                    arg,
                ));
            }
            markers.push(Marker {
                kind: kind.into(),
                description: unescape_body(question, matches!(kind, "one" | "flag")),
                span: start..end,
                argv_index,
                prefix: OsString::new(),
                suffix: OsString::new(),
                opens_argument: start == 0,
                options,
                flag,
            });
            pos = end;
        }
        // The command is literal: jevify never runs a model-chosen program. A marker in argv[0]
        // is refused before the space guard, since its description usually holds spaces.
        if argv_index == 0 {
            if let Some(marker) = markers.first() {
                return Err(MarkerError(format!(
                    "the command must be literal; find the tool first with jevify pick --from tool {}",
                    single_quoted(&marker.description)
                )));
            }
            if bytes.contains(&b' ') {
                return Err(MarkerError(
                    "pass the command as separate arguments, for example 'git' 'switch' '@{branch:description}'".into(),
                ));
            }
        }
        for i in 0..markers.len() {
            let before = if i == 0 { 0 } else { markers[i - 1].span.end };
            let after = markers.get(i + 1).map_or(bytes.len(), |m| m.span.start);
            markers[i].prefix = literal(&bytes[before..markers[i].span.start]);
            markers[i].suffix = literal(&bytes[markers[i].span.end..after]);
        }
        count += markers.len();
        args.push(Arg {
            literal: arg.clone(),
            markers,
        });
    }
    if count == 0 {
        return Err(MarkerError(
            "no markers; pass a command and a marker argument such as '@{file:description}'".into(),
        ));
    }
    Ok(args)
}

/// Check stdin ownership after parsing and before reading either input.
pub fn check_stdin_roles(
    args: &[Arg],
    has_candidates: bool,
    has_context: bool,
) -> Result<(), MarkerError> {
    let markers = || args.iter().flat_map(|arg| &arg.markers);
    if !has_candidates
        && !has_context
        && markers().any(|m| m.kind == "-")
        && markers().any(|m| matches!(m.kind.as_str(), "one" | "flag"))
    {
        return Err(MarkerError(
            "stdin has two roles; add '--candidates' 'FILE' or '--context' 'FILE' before '--'"
                .into(),
        ));
    }
    Ok(())
}

/// Handles follow marker order. An empty handle for a flag removes its argument.
pub fn substitute(args: &[Arg], handles: &[OsString]) -> Result<Vec<OsString>, MarkerError> {
    if args.iter().map(|a| a.markers.len()).sum::<usize>() != handles.len() {
        return Err(MarkerError(
            "supply one resolved handle per marker, such as '@{file:description}'".into(),
        ));
    }
    let mut handles = handles.iter();
    let mut result = Vec::with_capacity(args.len());
    for arg in args {
        let bytes = arg.literal.as_bytes();
        let mut output = OsString::new();
        let mut pos = 0;
        let mut remove = false;
        for marker in &arg.markers {
            let handle = handles.next().expect("handle count checked above");
            if marker.kind == "flag" && handle.is_empty() {
                remove = true;
                break;
            }
            output.push(literal(&bytes[pos..marker.span.start]));
            if marker.opens_argument
                && matches!(marker.kind.as_str(), "file" | "dir")
                && handle.as_bytes().starts_with(b"-")
            {
                output.push("./");
            }
            output.push(handle);
            pos = marker.span.end;
        }
        if !remove {
            output.push(literal(&bytes[pos..]));
            result.push(output);
        }
    }
    Ok(result)
}

fn marker_start(bytes: &[u8], pos: usize) -> Option<usize> {
    if !bytes[pos..].starts_with(b"@{") {
        return None;
    }
    let mut end = pos + 2;
    if bytes.get(end) == Some(&b'-') {
        end += 1;
    } else {
        if !bytes.get(end).is_some_and(u8::is_ascii_lowercase) {
            return None;
        }
        end += 1;
        while bytes
            .get(end)
            .is_some_and(|b| b.is_ascii_lowercase() || *b == b'-')
        {
            end += 1;
        }
    }
    (bytes.get(end) == Some(&b':')).then_some(end + 1)
}

fn strip_quotes(body: &str) -> &str {
    let bytes = body.as_bytes();
    if bytes.len() >= 2 && matches!(bytes[0], b'\'' | b'"') && bytes.last() == Some(&bytes[0]) {
        &body[1..body.len() - 1]
    } else {
        body
    }
}

fn delimiter(bytes: &[u8], target: u8) -> Option<usize> {
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'\\'
            && bytes
                .get(pos + 1)
                .is_some_and(|b| matches!(b, b':' | b'|' | b'}'))
        {
            pos += 2;
        } else if bytes[pos] == target {
            return Some(pos);
        } else {
            pos += 1;
        }
    }
    None
}

fn unescape_body(body: &str, choices: bool) -> String {
    let mut chars = body.chars().peekable();
    let mut output = String::with_capacity(body.len());
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && chars
                .peek()
                .is_some_and(|next| *next == '}' || choices && matches!(next, ':' | '|'))
        {
            output.push(chars.next().expect("peeked escape"));
        } else {
            output.push(if ch == '\n' { ' ' } else { ch });
        }
    }
    output
}

fn literal(bytes: &[u8]) -> OsString {
    let mut output = Vec::with_capacity(bytes.len());
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos..].starts_with(b"@@{") {
            output.extend_from_slice(b"@{");
            pos += 3;
        } else {
            output.push(bytes[pos]);
            pos += 1;
        }
    }
    OsString::from_vec(output)
}

fn literal_error(reason: &str, arg: &OsStr) -> MarkerError {
    let corrected = arg.to_string_lossy().replace("@{", "@@{");
    MarkerError(format!("{reason}: {}", single_quoted(&corrected)))
}

/// One single-quoted shell word on one line.
fn single_quoted(text: &str) -> String {
    let text = text
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\'', "'\\''");
    format!("'{text}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    fn argv(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn literal_table_preserves_bytes_and_decodes_only_the_escape() {
        for (input, expected) in [
            ("@{u}", "@{u}"),
            ("@{-1}", "@{-1}"),
            ("HEAD@{2}", "HEAD@{2}"),
            ("stash@{0}", "stash@{0}"),
            ("@{1 day ago}", "@{1 day ago}"),
            ("@types/node", "@types/node"),
            ("user@host:path", "user@host:path"),
            ("@{k='v'}", "@{k='v'}"),
            ("{user}@@{host:>8}", "{user}@{host:>8}"),
            ("@@{file:unterminated", "@{file:unterminated"),
        ] {
            let args = parse(&argv(&["cmd", input, "@{file:x}"])).unwrap();
            assert!(args[1].markers.is_empty(), "{input}");
            let result = substitute(&args, &argv(&["chosen"])).unwrap();
            assert_eq!(result[1], expected, "{input}");
        }
        let mut input = argv(&["cmd", "", "@{-:x}"]);
        input[1] = OsString::from_vec(vec![0xff, b'\n', b'@', b'{', b'2', b'}']);
        assert_eq!(
            substitute(&parse(&input).unwrap(), &argv(&["x"])).unwrap()[1],
            input[1]
        );
    }

    #[test]
    fn descriptions_are_decoded_once() {
        for (input, expected) in [
            ("@{file:a\\}b}", "a}b"),
            ("@{file:'hello'}", "hello"),
            ("@{file:\"hello\"}", "hello"),
            ("@{file:\"'hello'\"}", "'hello'"),
            ("@{file:a\nb}", "a b"),
            (r"@{file:a\:b\|c}", r"a\:b\|c"),
            (r"@{file:a\\}b}", r"a\}b"),
        ] {
            let args = parse(&argv(&["cmd", input])).unwrap();
            assert_eq!(args[1].markers[0].description, expected);
        }
    }

    #[test]
    fn context_markers_decode_options_flags_and_questions() {
        for (input, options, question) in [
            ("@{one:a|b:q}", vec!["a", "b"], "q"),
            (
                r"@{one:a\|b|c\:d|e\}f:q\:x\|y\}z}",
                vec!["a|b", "c:d", "e}f"],
                "q:x|y}z",
            ),
            ("@{one:\"a|b:q\"}", vec!["a", "b"], "q"),
            ("@{one:|b:q}", vec!["", "b"], "q"),
        ] {
            let args = parse(&argv(&["cmd", input])).unwrap();
            assert_eq!(args[1].markers[0].options, options);
            assert_eq!(args[1].markers[0].description, question);
        }
        let args = parse(&argv(&["cmd", r"@{flag:--name=a\:b:q}"])).unwrap();
        assert_eq!(args[1].markers[0].flag.as_deref(), Some("--name=a:b"));
        for input in [r"@{one:a\|b|a\|b:q}", "@{one:a|b:}", "@{flag:--draft:}"] {
            assert!(parse(&argv(&["cmd", input])).is_err());
        }
    }

    #[test]
    fn unterminated_empty_misplaced_markers_and_marker_commands_are_errors() {
        for values in [
            vec!["cmd", "@{file:unfinished"],
            vec!["cmd", "@{file:}"],
            vec!["cmd", "@{file:''}"],
            vec!["@{tool:compiler}", "x"],
            vec!["cmd", "@{one:a:q}"],
            vec!["cmd", "@{one:a|b}"],
            vec!["cmd", "prefix@{flag:--draft:q}"],
            vec!["cmd", "@{flag:draft:q}"],
            vec!["cmd", "@{flag:--draft}"],
            vec!["cmd", "literal"],
            vec!["cmd --arg", "@{file:x}"],
            vec![],
        ] {
            assert!(parse(&argv(&values)).is_err(), "{values:?}");
        }
        let input = vec![
            OsString::from("cmd"),
            OsString::from_vec(b"@{file:\xff}".to_vec()),
        ];
        assert!(parse(&input).is_err());
    }

    #[test]
    fn substitution_table() {
        for (input, handles, expected) in [
            ("src/@{file:x}", vec!["cmd/add.rs"], "src/cmd/add.rs"),
            ("@{dir:x}/archive", vec!["logs"], "logs/archive"),
            (
                "--config=conf/@{file:x}",
                vec!["app.toml"],
                "--config=conf/app.toml",
            ),
            ("@{file:x}", vec!["-odd"], "./-odd"),
            ("src/@{file:x}", vec!["-odd"], "src/-odd"),
            ("@{file:x}:@{branch:y}", vec!["a", "b"], "a:b"),
            ("@@{@{file:x}@@{", vec!["@{literal:x}"], "@{@{literal:x}@{"),
        ] {
            let args = parse(&argv(&["cmd", input])).unwrap();
            assert_eq!(substitute(&args, &argv(&handles)).unwrap()[1], expected);
        }
        let args = parse(&argv(&["cmd", "@{flag:--draft:q}", "end"])).unwrap();
        assert_eq!(
            substitute(&args, &argv(&[""])).unwrap(),
            argv(&["cmd", "end"])
        );
        assert_eq!(
            substitute(&args, &argv(&["--draft"])).unwrap(),
            argv(&["cmd", "--draft", "end"])
        );
        let args = parse(&argv(&["cmd", "src/@{file:x}"])).unwrap();
        let result = substitute(&args, &[OsString::from_vec(vec![0xff])]).unwrap();
        assert_eq!(result[1].as_bytes(), b"src/\xff");
        assert!(substitute(&args, &[]).is_err());
        assert!(substitute(&args, &argv(&["a", "b"])).is_err());
    }
}
