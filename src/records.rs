use crate::config::Config;
use crate::exit::{Exit, JevifyError};
use crate::jev::client::{Client, batch_size};
use crate::jev::{Questions, Response};
use futures::StreamExt;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::{ffi::OsString, ops::Range};

#[derive(Clone, Debug)]
pub struct Record {
    pub handle: OsString,
    pub evidence: String,
    pub raw: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    Lines,
    Nul,
    Para,
}

/// Raw ranges include their original terminators; handles exclude only terminators.
pub fn parse(input: &[u8], split: Split) -> Result<Vec<Record>, JevifyError> {
    let mut records = Vec::new();
    let mut start = 0;
    let delimiter = if split == Split::Nul { 0 } else { b'\n' };
    let mut block = None;
    for part in input.split_inclusive(|b| [delimiter].contains(b)) {
        let end = start + part.len();
        let mut content = part.strip_suffix(&[delimiter]).unwrap_or(part);
        if split != Split::Nul {
            content = content.strip_suffix(b"\r").unwrap_or(content);
        }
        let blank = content.iter().all(u8::is_ascii_whitespace);
        if split == Split::Para {
            if blank {
                if let Some(begin) = block.take() {
                    records.push(record(
                        input,
                        begin..end,
                        paragraph_handle(input, begin..start),
                    ));
                }
            } else {
                block.get_or_insert(start);
            }
        } else if !blank {
            records.push(record(input, start..end, start..start + content.len()));
        }
        start = end;
    }
    if let Some(begin) = block {
        records.push(record(
            input,
            begin..start,
            paragraph_handle(input, begin..start),
        ));
    }
    Ok(records)
}

fn paragraph_handle(input: &[u8], mut range: Range<usize>) -> Range<usize> {
    if input[range.clone()].ends_with(b"\n") {
        range.end -= 1;
        if input[range.clone()].ends_with(b"\r") {
            range.end -= 1;
        }
    }
    range
}

fn evidence(bytes: &[u8]) -> String {
    let text = crate::input::strip_ansi(&String::from_utf8_lossy(bytes));
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    crate::tournament::clip(&crate::input::redact(&text), 32_000)
}

fn record(input: &[u8], raw: Range<usize>, handle: Range<usize>) -> Record {
    Record {
        handle: OsString::from_vec(input[handle.clone()].to_vec()),
        evidence: evidence(&input[handle]),
        raw,
    }
}

/// Select a 1-based whitespace field, dropping and counting records without it.
pub fn field(records: &mut Vec<Record>, number: usize) -> Result<usize, JevifyError> {
    if number == 0 {
        return Err(JevifyError::Usage("field numbers start at 1".into()));
    }
    let before = records.len();
    records.retain_mut(|record| {
        let value = record
            .handle
            .as_bytes()
            .split(u8::is_ascii_whitespace)
            .filter(|s| !s.is_empty())
            .nth(number - 1);
        if let Some(value) = value {
            record.handle = OsString::from_vec(value.to_vec());
            true
        } else {
            false
        }
    });
    Ok(before - records.len())
}

/// Read JSON lines or one JSON array, preserving each value's source range.
pub fn key(input: &[u8], name: &str) -> Result<(Vec<Record>, usize), JevifyError> {
    let error = |e| JevifyError::Input(format!("invalid JSON: {e}"));
    let mut ranges = Vec::new();
    if matches!(input.iter().find(|b| !b.is_ascii_whitespace()), Some(b'[')) {
        let _: Vec<serde_json::Value> = serde_json::from_slice(input).map_err(error)?;
        let mut pos = input.iter().position(|b| matches!(b, b'[')).unwrap() + 1;
        loop {
            while input[pos].is_ascii_whitespace() || matches!(input[pos], b',') {
                pos += 1;
            }
            if matches!(input[pos], b']') {
                break;
            }
            let mut values = serde_json::Deserializer::from_slice(&input[pos..])
                .into_iter::<serde_json::Value>();
            values
                .next()
                .expect("validated array element")
                .map_err(error)?;
            let end = pos + values.byte_offset();
            ranges.push((pos..end, pos..end));
            pos = end;
        }
    } else {
        for r in parse(input, Split::Lines)? {
            ranges.push((r.raw.clone(), r.raw));
        }
    }
    let mut records = Vec::new();
    let mut omitted = 0;
    for (raw, content) in ranges {
        let value: serde_json::Value =
            serde_json::from_slice(&input[content.clone()]).map_err(error)?;
        if let Some(handle) = value.get(name) {
            records.push(Record {
                handle: handle
                    .as_str()
                    .map_or_else(|| handle.to_string(), str::to_owned)
                    .into(),
                evidence: evidence(&input[content]),
                raw,
            });
        } else {
            omitted += 1;
        }
    }
    Ok((records, omitted))
}

/// `ordinal` is the occurrence's 1-based position before selection or deduplication.
pub fn envelope(input: &[u8], record: &Record, ordinal: usize) -> serde_json::Value {
    let bytes = &input[record.raw.clone()];
    let mut value = serde_json::json!({"text": String::from_utf8_lossy(bytes), "ordinal": ordinal});
    if std::str::from_utf8(bytes).is_err() {
        value["lossy"] = true.into();
    }
    value
}

/// Representative record indices and, for every occurrence, its distinct-set index.
pub fn distinct(input: &[u8], records: &[Record]) -> (Vec<usize>, Vec<usize>) {
    let mut seen = HashMap::new();
    let mut representatives = Vec::new();
    let mut occurrences = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let next = representatives.len();
        let distinct = *seen.entry(&input[record.raw.clone()]).or_insert_with(|| {
            representatives.push(index);
            next
        });
        occurrences.push(distinct);
    }
    (representatives, occurrences)
}

/// Judge caller-written components only. `.` and `..` navigation are not hidden components.
pub(crate) fn withheld(path: &Path) -> bool {
    path.components().any(|component| {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        let bytes = name.as_bytes();
        bytes.starts_with(b".")
            || bytes.starts_with(b"id_")
            || bytes.ends_with(b".pem")
            || bytes.ends_with(b".key")
            || bytes.windows(11).any(|w| matches!(w, b"credentials"))
            || bytes.windows(6).any(|w| matches!(w, b"secret"))
    })
}

/// The lines of an excerpt that carry meaning: blank lines, imports (`use`, `import`, `from`,
/// `#include`, `mod`, `extern crate`, `package`, with their `{ ... }` continuations), shebang
/// and inner-attribute lines, and license or copyright comment lines are dropped. Doc comments
/// and every other line stay, in order, within the bound the excerpt already has.
fn meaningful(text: &str) -> String {
    const IMPORTS: &[&str] = &[
        "use ",
        "pub use ",
        "pub(crate) use ",
        "import ",
        "from ",
        "#include",
        "mod ",
        "pub mod ",
        "pub(crate) mod ",
        "extern crate ",
        "package ",
    ];
    let mut kept = Vec::new();
    let mut open_import = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if open_import {
            open_import = !trimmed.ends_with(';');
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("#!") {
            continue;
        }
        let import = IMPORTS.iter().any(|prefix| trimmed.starts_with(prefix))
            && (!trimmed.starts_with("from ") || trimmed.contains(" import "));
        if import {
            open_import = trimmed.ends_with('{') || trimmed.ends_with(',');
            continue;
        }
        let comment = trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*');
        let lower = trimmed.to_ascii_lowercase();
        if comment
            && (lower.contains("copyright") || lower.contains("license") || lower.contains("spdx"))
        {
            continue;
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// The records of a `--files` read that carry their name and no excerpt.
///
/// Two reasons keep an excerpt out, and they are told apart: the policy of `withheld`
/// (a hidden or secret-looking path, a symlink file) is a choice jevify makes, and the caller
/// can predict it from the name; an unreadable file (missing, a directory in its place, a
/// permission or sandbox denial on the file or on a directory on the way to it) is a failure
/// the caller cannot predict, and a verdict on its name alone would pass for a verdict on its
/// content. `count` is the status line's `excerpts withheld: N`, both reasons together, so a
/// caller who reads one number learns that N records were judged without their content.
/// `unreadable` names the second kind, by record index, with the operating system's reason;
/// a verb prints them on stderr and leaves those records unsure instead of asking.
#[derive(Debug, Default)]
pub struct Unread {
    pub count: usize,
    pub unreadable: std::collections::BTreeMap<usize, String>,
}

impl Unread {
    /// Name every unreadable record on stderr under `verb`, the first ten in full.
    pub fn report(&self, verb: &str, records: &[Record]) {
        const NAMED: usize = 10;
        for (&index, reason) in self.unreadable.iter().take(NAMED) {
            let name = records[index].handle.to_string_lossy();
            let name: String = name
                .chars()
                .map(|c| if c.is_control() { '\u{fffd}' } else { c })
                .collect();
            eprintln!("jevify {verb}: excerpt unreadable: {name}: {reason}");
        }
        if self.unreadable.len() > NAMED {
            eprintln!(
                "jevify {verb}: excerpt unreadable: {} more",
                self.unreadable.len() - NAMED
            );
        }
    }
}

/// Enrich only the supplied records (all records or pick's finalists). A record whose excerpt
/// is withheld or unreadable keeps its name as evidence and is counted in the result.
pub async fn excerpts(records: &mut [Record], cwd: &Path) -> Result<Unread, JevifyError> {
    let paths: Vec<_> = records.iter().map(|r| r.handle.clone()).collect();
    let cwd = cwd.to_path_buf();
    let (values, unread) = tokio::task::spawn_blocking(move || {
        let mut unread = Unread::default();
        let values: Vec<_> = paths
            .into_iter()
            .enumerate()
            .map(|(index, handle)| {
                let fallback = evidence(handle.as_bytes());
                match excerpt_of(&cwd, Path::new(&handle)) {
                    Ok(Some(text)) => evidence(text.as_bytes()),
                    Ok(None) => {
                        unread.count += 1;
                        fallback
                    }
                    Err(reason) => {
                        unread.count += 1;
                        unread.unreadable.insert(index, reason);
                        fallback
                    }
                }
            })
            .collect();
        (values, unread)
    })
    .await
    .map_err(|e| JevifyError::Input(format!("excerpt worker failed: {e}")))?;
    for (record, value) in records.iter_mut().zip(values) {
        record.evidence = value;
    }
    Ok(unread)
}

/// `Ok(Some)` is the excerpt, `Ok(None)` a path the policy withholds, `Err` the reason a
/// file's bytes could not be read.
fn excerpt_of(cwd: &Path, path: &Path) -> Result<Option<String>, String> {
    if withheld(path) {
        return Ok(None);
    }
    let joined = cwd.join(path);
    let normalized: std::path::PathBuf = joined.components().collect();
    let Some(name) = normalized.file_name() else {
        return Err("not a file name".into());
    };
    let parent = normalized
        .parent()
        .ok_or_else(|| "not a file name".to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let resolved = parent.join(name);
    let metadata = resolved.symlink_metadata().map_err(|e| e.to_string())?;
    if metadata.is_symlink() {
        return Ok(None);
    }
    if metadata.is_dir() {
        return Err("is a directory".into());
    }
    if !metadata.is_file() {
        return Err("not a regular file".into());
    }
    let excerpt = read_excerpt(&resolved).map_err(|e| e.to_string())?;
    let name = name.to_string_lossy();
    Ok(Some(match excerpt.strip_prefix(&format!("{name}: ")) {
        Some(body) => format!("{name}: {}", meaningful(body)),
        None => excerpt,
    }))
}

/// The stdin of `filter`: its records, deduplicated and capped, with excerpts
/// read for `--files`; judged one Choice per distinct record and written out in input order.
pub struct Input {
    verb: &'static str,
    machine: bool,
    bytes: Vec<u8>,
    pub records: Vec<Record>,
    /// The representative record of each distinct record, and each record's distinct index.
    unique: Vec<usize>,
    occurrences: Vec<usize>,
    pub unread: Unread,
    /// The saved copy of the bytes for `saved_input`, or why there is none.
    pub saved: Result<PathBuf, String>,
}

/// Reads and splits stdin for `verb`, off the runtime; `no_save` is true for a verb that keeps
/// no copy of its input; `example` is the hint past 20,000 distinct records.
pub async fn read(
    verb: &'static str,
    split: Split,
    machine: bool,
    no_save: bool,
    example: &'static str,
) -> Result<Input, JevifyError> {
    tokio::task::spawn_blocking(move || {
        let bytes = crate::input::read_stdin_bytes()?;
        let records = parse(&bytes, split)?;
        let (unique, occurrences) = distinct(&bytes, &records);
        if unique.len() > 20_000 {
            return Err(JevifyError::Kinded {
                kind: "too_many",
                exit: Exit::Input,
                message: "more than 20,000 distinct records; narrow with grep or head".into(),
                hint: "narrow with grep or head",
                example,
            });
        }
        let directory = crate::config::saved_input_dir(no_save);
        let saved = crate::save::save(&bytes, directory.as_deref());
        Ok(Input {
            verb,
            machine,
            bytes,
            records,
            unique,
            occurrences,
            unread: Unread::default(),
            saved,
        })
    })
    .await
    .map_err(|e| JevifyError::Input(e.to_string()))?
}

impl Input {
    /// With `files`, reads each record's excerpt and says how many were withheld.
    pub async fn excerpts(&mut self, files: bool) -> Result<(), JevifyError> {
        if files {
            let cwd = std::env::current_dir().map_err(|e| JevifyError::Input(e.to_string()))?;
            self.unread = excerpts(&mut self.records, &cwd).await?;
        }
        let withheld = self.unread.count;
        if !self.machine && withheld > 0 {
            eprintln!("jevify {}: excerpts withheld: {withheld}", self.verb);
        }
        self.unread.report(self.verb, &self.records);
        Ok(())
    }

    /// The bytes of record `index` as they came in.
    pub fn raw(&self, index: usize) -> &[u8] {
        &self.bytes[self.records[index].raw.clone()]
    }

    /// Record `index`'s envelope entry with its verdict `fields` and, if unreadable, the reason.
    pub fn entry(
        &self,
        index: usize,
        fields: impl FnOnce(&mut serde_json::Value),
    ) -> serde_json::Value {
        let mut entry = envelope(&self.bytes, &self.records[index], index + 1);
        fields(&mut entry);
        if let Some(reason) = self.unread.unreadable.get(&index) {
            entry["unreadable"] = reason.as_str().into();
        }
        entry
    }

    /// One Choice per distinct record, batched, delivered in input order as each batch lands.
    /// `answer` reads a record's verdict off its response; `emit` writes record `index` with
    /// its verdict, false to stop, which cancels the pending requests. A record never asked
    /// (an unreadable file: its name alone is not the evidence the caller asked for) or never
    /// answered is `unsure`. `Ok(true)` when every record was written.
    pub async fn judge<A: Clone>(
        &self,
        ctx: &Config,
        questions: &Questions,
        unsure: A,
        mut answer: impl FnMut(&Response) -> Result<A, JevifyError>,
        mut emit: impl FnMut(usize, &A) -> Result<bool, JevifyError>,
    ) -> Result<bool, JevifyError> {
        let client = Client::new(ctx)?;
        let asked: Vec<usize> = (0..self.unique.len())
            .filter(|&u| !self.unread.unreadable.contains_key(&self.unique[u]))
            .collect();
        let evidence: Vec<_> = asked
            .iter()
            .map(|&u| self.records[self.unique[u]].evidence.clone())
            .collect();
        let size = batch_size(client.backend(), questions.len());
        eprintln!(
            "jevify {}: {} records, {} distinct, {} requests",
            self.verb,
            self.records.len(),
            self.unique.len(),
            asked.len().div_ceil(size)
        );
        let mut answers: Vec<A> = Vec::with_capacity(self.unique.len());
        let mut cursor = 0;
        let mut flush = |answers: &[A]| -> Result<bool, JevifyError> {
            while cursor < self.records.len() && self.occurrences[cursor] < answers.len() {
                if !emit(cursor, &answers[self.occurrences[cursor]])? {
                    return Ok(false);
                }
                cursor += 1;
            }
            Ok(true)
        };
        let streamed = async {
            let mut stream = client.ask_each(&evidence, questions);
            let mut ready = BTreeMap::new();
            let mut next = 0;
            let mut asked_done = 0;
            while let Some(batch) = stream.next().await {
                let (index, responses) = batch?;
                ready.insert(index, responses);
                while let Some(responses) = ready.remove(&next) {
                    for response in &responses {
                        let verdict = answer(response)?;
                        answers.resize(asked[asked_done], unsure.clone());
                        answers.push(verdict);
                        asked_done += 1;
                    }
                    if !flush(&answers)? {
                        return Ok(false);
                    }
                    next += 1;
                }
            }
            answers.resize(self.unique.len(), unsure.clone());
            flush(&answers)
        }
        .await;
        streamed.map_err(|error| {
            let answered = format!("answered {} of {}", answers.len(), self.unique.len());
            if self.machine {
                return JevifyError::Kinded {
                    kind: error.kind(),
                    exit: error.exit(),
                    message: format!("{error}; {answered}"),
                    hint: error.hint(),
                    example: error.example(),
                };
            }
            eprintln!("jevify {}: {answered}", self.verb);
            error
        })
    }

    /// The verb's closing stderr line: `head`, the withheld count, and any model that answered
    /// instead of Jev, on one line.
    pub fn status(&self, ctx: &Config, head: &str) {
        let mut status = format!("jevify {}: {head}", self.verb);
        if self.unread.count > 0 {
            status.push_str(&format!(", excerpts withheld: {}", self.unread.count));
        }
        if let Some(model) = ctx.stats.model.lock().unwrap().as_ref() {
            let others: Vec<_> = model
                .split(", ")
                .filter(|m| !m.starts_with("jev"))
                .collect();
            if !others.is_empty() {
                status.push_str(&format!(", answered by {}, not Jev", others.join(", ")));
            }
        }
        eprintln!("{}", status.replace(['\r', '\n'], " "));
    }
}

/// Flush each record so even a short first batch reaches the downstream reader.
pub fn write_record(
    stdout: &mut std::io::StdoutLock<'_>,
    bytes: &[u8],
) -> Result<bool, JevifyError> {
    match stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(JevifyError::Input(error.to_string())),
    }
}

/// The excerpt of a regular file, or the error that kept its bytes from being read (a denied
/// directory on the way, a missing file, a directory in its place). Binary content is not an
/// error: the name alone describes it. `p` is absolute and normalized.
fn read_excerpt(p: &Path) -> std::io::Result<String> {
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
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        && let Some(text) = pdf_text(p)
    {
        return format!("{name}: {}", crate::input::redact(&text));
    }
    name
}

/// One `pdftotext` render of the first two pages; a hung or slow converter yields no text and
/// the file is described by its name alone.
const PDF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Two pages of text are a few kilobytes; the excerpt keeps 2,000 characters of it.
const PDF_OUTPUT_CAP: usize = 64 * 1024;

/// The text of the PDF at `path` from the `pdftotext` on PATH, killed at the timeout (no text
/// then), output bounded. Only a regular file that opens without following a link is handed to
/// the converter.
fn pdf_text(path: &Path) -> Option<String> {
    open_regular(path).ok()?;
    let mut command = std::process::Command::new("pdftotext");
    command
        .args(["-l", "2"])
        .arg(path)
        .arg("-")
        .env("PATH", std::env::var_os("PATH").unwrap_or_default());
    let deadline = std::time::Instant::now() + PDF_TIMEOUT;
    let output = crate::inventory::output_within(command, deadline, PDF_OUTPUT_CAP)?;
    let text: String = String::from_utf8_lossy(&output)
        .chars()
        .take(2000)
        .collect();
    (!text.trim().is_empty()).then_some(text)
}

/// Opens a regular file by walking every component without following a link.
fn open_regular(path: &Path) -> std::io::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags, openat};
    use std::io::Error;
    let mut dir = std::fs::File::open("/")?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::other("missing parent"))?;
    for component in parent.components() {
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
            _ => return Err(Error::other("expected absolute normalized path")),
        }
    }
    let file: std::fs::File = openat(
        &dir,
        path.file_name()
            .ok_or_else(|| Error::other("missing name"))?,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into();
    if !file.metadata()?.is_file() {
        return Err(Error::other("not a regular file"));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn withholding_judges_the_caller_path_not_the_working_directory_ancestors() {
        for path in ["", ".", "./source.rs", "a/./source.rs", "../source.rs"] {
            assert!(!withheld(Path::new(path)), "{path}");
        }
        for path in [
            ".npmrc",
            "a/.hidden/b.txt",
            "conf/.env.local",
            "/project/.hidden/source.rs",
            "a/.hidden/../source.rs",
        ] {
            assert!(withheld(Path::new(path)), "{path}");
        }
        let root = tempfile::Builder::new()
            .prefix(".jevify-records-")
            .tempdir()
            .unwrap()
            .keep();
        eprintln!("retained records fixture: {}", root.display());
        std::fs::write(root.join("source.rs"), "VISIBLE_EXCERPT_MARKER").unwrap();
        let mut records = parse(b"./source.rs\n", Split::Lines).unwrap();
        assert_eq!(excerpts(&mut records, &root).await.unwrap().count, 0);
        assert!(records[0].evidence.contains("VISIBLE_EXCERPT_MARKER"));
        let absolute = root.join("source.rs");
        let mut records = parse(absolute.as_os_str().as_bytes(), Split::Nul).unwrap();
        let unread = excerpts(&mut records, &root).await.unwrap();
        assert_eq!((unread.count, unread.unreadable.len()), (1, 0));
        assert_eq!(
            records[0].evidence,
            evidence(absolute.as_os_str().as_bytes())
        );
    }

    #[test]
    fn split_boundaries_and_blank_records() {
        for split in [Split::Lines, Split::Nul, Split::Para] {
            assert!(parse(b"", split).unwrap().is_empty());
            let records = parse(b"one", split).unwrap();
            assert_eq!(records[0].raw, 0..3);
            assert_eq!(records[0].handle, "one");
        }
        for (input, split, expected) in [
            (b"a\r\nb\n".as_slice(), Split::Lines, vec![0..3, 3..5]),
            (b"a\0b\0".as_slice(), Split::Nul, vec![0..2, 2..4]),
            (b"a\nb\n\nc\n".as_slice(), Split::Para, vec![0..5, 5..7]),
            (b"a\r\nb\r\n\r\nc".as_slice(), Split::Para, vec![0..8, 8..9]),
            (b"\n \t\r\n".as_slice(), Split::Lines, vec![]),
            (b"\0 \t\0".as_slice(), Split::Nul, vec![]),
            (b"\n \t\r\n".as_slice(), Split::Para, vec![]),
        ] {
            let records = parse(input, split).unwrap();
            assert_eq!(
                records.iter().map(|r| r.raw.clone()).collect::<Vec<_>>(),
                expected
            );
        }
        let input = b"a\nb\n\nc\n";
        let records = parse(input, Split::Para).unwrap();
        assert_eq!(records[0].handle, "a\nb");
        assert_eq!(
            records
                .iter()
                .flat_map(|r| input[r.raw.clone()].iter().copied())
                .collect::<Vec<_>>(),
            input
        );
    }

    #[test]
    fn raw_bytes_and_handles_survive_clean_evidence() {
        let input = b"\x1b[31mred\x1b[0m\0\xff\r\n";
        let records = parse(input, Split::Lines).unwrap();
        assert_eq!(&input[records[0].raw.clone()], input);
        assert_eq!(records[0].handle.as_bytes(), &input[..input.len() - 2]);
        assert_eq!(records[0].evidence, "red\u{fffd}");
        let paths = parse(b"dir/\xff\0", Split::Nul).unwrap();
        assert_eq!(paths[0].handle.as_bytes(), b"dir/\xff");
        let mut fields = parse(b"one \xff\n", Split::Lines).unwrap();
        assert_eq!(field(&mut fields, 2).unwrap(), 0);
        assert_eq!(fields[0].handle.as_bytes(), b"\xff");
        assert_eq!(fields[0].evidence, "one \u{fffd}");
        assert_eq!(evidence(b"token=abcdefghijk"), "token=[REDACTED]");
    }

    #[test]
    fn envelope_and_distinct_preserve_occurrences() {
        let input = b"same\n\xff\nsame\n\xff\n";
        let records = parse(input, Split::Lines).unwrap();
        let (unique, map) = distinct(input, &records);
        assert_eq!(unique, [0, 1]);
        assert_eq!(map, [0, 1, 0, 1]);
        for (i, r) in records.iter().enumerate() {
            assert_eq!(
                &input[r.raw.clone()],
                &input[records[unique[map[i]]].raw.clone()]
            );
            let value = envelope(input, r, i + 1);
            assert_eq!(value["ordinal"], i + 1);
            assert_eq!(
                value["text"],
                String::from_utf8_lossy(&input[r.raw.clone()]).as_ref()
            );
            assert_eq!(
                value.get("lossy"),
                if i % 2 == 1 {
                    Some(&serde_json::Value::Bool(true))
                } else {
                    None
                }
            );
        }
    }

    #[test]
    fn fields_and_json_handles() {
        for (number, expected, omitted) in
            [(1, Some("one"), 0), (3, Some("three"), 0), (4, None, 1)]
        {
            let mut records = parse(b" one\t  two   three\n", Split::Lines).unwrap();
            assert_eq!(field(&mut records, number).unwrap(), omitted);
            assert_eq!(
                records.first().map(|r| r.handle.to_str().unwrap()),
                expected
            );
        }
        assert!(field(&mut vec![], 0).is_err());
        for input in [
            b"{\"id\":\"a\"}\n{\"id\":12}\n{}\n".as_slice(),
            b"[ {\"id\":\"a\"}, {\"id\":12}, {} ]",
        ] {
            let (records, omitted) = key(input, "id").unwrap();
            assert_eq!(omitted, 1);
            assert_eq!(
                records
                    .iter()
                    .map(|r| r.handle.as_bytes())
                    .collect::<Vec<_>>(),
                [b"a".as_slice(), b"12"]
            );
            for record in records {
                let value: serde_json::Value = serde_json::from_slice(&input[record.raw]).unwrap();
                assert!(value.get("id").is_some());
            }
        }
        for input in [
            b"[".as_slice(),
            b"[{},]",
            b"no json",
            b"{} {}",
            b"{\"id\":\"\xff\"}",
        ] {
            assert_eq!(key(input, "id").unwrap_err().exit().code(), 6);
        }
        assert!(key(b"[]", "id").unwrap().0.is_empty());
        let (nested, _) = key(br#"[{"id":{"a":[1,2]}},{"id":"a,b]"}]"#, "id").unwrap();
        assert_eq!(nested[1].handle, "a,b]");
    }

    #[tokio::test]
    async fn excerpts_resolve_paths_and_withhold_before_reading() {
        // Keep fixtures: the worker contract forbids deleting even scratch files.
        let root = tempfile::Builder::new()
            .prefix("jevify-records-")
            .tempdir()
            .unwrap()
            .keep();
        std::fs::create_dir(root.join("a")).unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("a/.hidden")).unwrap();
        let marker = "PRIVATE_EXCERPT_MARKER";
        for path in [
            ".npmrc",
            ".env.local",
            "id_rsa",
            "x.pem",
            "a/.hidden/b.txt",
            "my-credentials.json",
            "x.key",
            "my-secret.txt",
        ] {
            std::fs::write(root.join(path), marker).unwrap();
        }
        for path in ["a.txt", "b.txt", "src/main.rs"] {
            std::fs::write(root.join(path), "VISIBLE_EXCERPT_MARKER").unwrap();
        }
        std::os::unix::fs::symlink(root.join("a.txt"), root.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("ancestor")).unwrap();
        let input = b"./a.txt\0a/../b.txt\0ancestor/a.txt\0src/main.rs\0missing\0a\0.npmrc\0.env.local\0id_rsa\0x.pem\0a/.hidden/b.txt\0my-credentials.json\0link.txt\0x.key\0my-secret.txt\0";
        let mut records = parse(input, Split::Nul).unwrap();
        // Nine withheld by policy; the missing path and the directory are unreadable.
        let unread = excerpts(&mut records, &root).await.unwrap();
        assert_eq!(unread.count, 11);
        assert_eq!(
            unread.unreadable.keys().copied().collect::<Vec<_>>(),
            [4, 5]
        );
        assert!(unread.unreadable[&4].contains("No such file"), "{unread:?}");
        assert_eq!(unread.unreadable[&5], "is a directory");
        for r in &records[..4] {
            assert!(
                r.evidence.contains("VISIBLE_EXCERPT_MARKER"),
                "{}",
                r.evidence
            );
        }
        for r in &records[4..] {
            assert_eq!(r.evidence, evidence(r.handle.as_bytes()));
        }
        assert!(records.iter().all(|r| !r.evidence.contains(marker)));
        assert_eq!(records.len(), 15);
        let mut subset = parse(b"a.txt\nmissing\n", Split::Lines).unwrap();
        assert_eq!(excerpts(&mut subset[..1], &root).await.unwrap().count, 0);
        assert_eq!(subset[1].evidence, "missing");
        eprintln!("retained records fixture: {}", root.display());
    }

    #[tokio::test]
    async fn non_utf8_file_excerpt() {
        let root = tempfile::Builder::new()
            .prefix("jevify-records-")
            .tempdir()
            .unwrap()
            .keep();
        eprintln!("retained records fixture: {}", root.display());
        let name = OsString::from_vec(b"file-\xff".to_vec());
        let file_created = match std::fs::write(root.join(&name), "VISIBLE_EXCERPT_MARKER") {
            Ok(()) => true,
            Err(error) => {
                // EILSEQ differs between the two supported platforms.
                assert!(
                    matches!(
                        (std::env::consts::OS, error.raw_os_error()),
                        ("macos", Some(92)) | ("linux", Some(84))
                    ),
                    "non-UTF-8 fixture creation failed: {error}"
                );
                eprintln!(
                    "NOT RUN: non-UTF-8 file-system excerpt assertion: file system refused the name with EILSEQ: {error}"
                );
                false
            }
        };
        let mut records = parse(b"file-\xff\0", Split::Nul).unwrap();
        let unread = excerpts(&mut records, &root).await.unwrap();
        assert_eq!(unread.count, usize::from(!file_created));
        assert_eq!(unread.unreadable.len(), usize::from(!file_created));
        assert_eq!(records[0].handle.as_bytes(), b"file-\xff");
        if file_created {
            assert!(records[0].evidence.contains("VISIBLE_EXCERPT_MARKER"));
        } else {
            assert_eq!(records[0].evidence, "file-\u{fffd}");
        }
    }
}
