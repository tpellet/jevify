use regex::Regex;
use std::ffi::OsStr;
use std::process::Command;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

static OVERSTRIKE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r".\x08").unwrap());
const SYNOPSIS_MAX_CHARS: usize = 1000;
/// One `man` render: a page is ready in well under a second; a hung formatter yields no text.
const MAN_TIMEOUT: Duration = Duration::from_secs(5);
/// The largest page kept; the sections read are near the top.
const MAN_OUTPUT_CAP: usize = 4 * 1024 * 1024;

/// Renders the man page without running the tool itself, even with `--help`.
fn man_page(cmd: &str) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    man_page_within(cmd, &path, Instant::now() + MAN_TIMEOUT)
}

/// The page of `cmd` from the `man` on `path`, killed at `deadline` (no text then), stdin at
/// `/dev/null`, output bounded.
fn man_page_within(cmd: &str, path: &OsStr, deadline: Instant) -> Option<String> {
    let mut man = Command::new("man");
    man.arg(cmd)
        .env("PATH", path)
        .env("MANPAGER", "cat")
        .env("PAGER", "cat")
        .env("MANWIDTH", "200");
    let out = crate::inventory::output_within(man, deadline, MAN_OUTPUT_CAP)?;
    let text = OVERSTRIKE
        .replace_all(&String::from_utf8_lossy(&out), "")
        .into_owned();
    (!text.trim().is_empty()).then_some(text)
}

pub fn section(page: &str, name: &str) -> Option<String> {
    let mut lines = page.lines().skip_while(|l| l.trim() != name).skip(1);
    let mut body = Vec::new();
    for l in lines.by_ref() {
        let is_heading = !l.is_empty()
            && !l.starts_with(char::is_whitespace)
            && l.chars().all(|c| c.is_ascii_uppercase() || c == ' ');
        if is_heading {
            break;
        }
        body.push(l.trim());
    }
    let s = body
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!s.is_empty()).then_some(s)
}

pub fn description(cmd: &str, max_chars: usize) -> Option<String> {
    section(&man_page(cmd)?, "DESCRIPTION").map(|d| d.chars().take(max_chars).collect())
}

/// The tool's synopsis, bounded to 1000 characters. Call from a blocking worker.
pub fn synopsis(cmd: &str) -> Option<String> {
    synopsis_from_page(man_page(cmd).as_deref())
}

fn synopsis_from_page(page: Option<&str>) -> Option<String> {
    section(page?, "SYNOPSIS").map(|s| s.chars().take(SYNOPSIS_MAX_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_synopsis_is_its_section_joined_or_nothing() {
        let page = "NAME\n    tool - inspect files\nSYNOPSIS\n    tool [-v]\n        file ...\n\nDESCRIPTION\n    Inspect files.\n";
        assert_eq!(
            synopsis_from_page(Some(page)).as_deref(),
            Some("tool [-v] file ...")
        );
        for page in [
            "",
            "NAME\n    tool\nDESCRIPTION\n    inspect files\n",
            "SYNOPSIS\n\nDESCRIPTION\n    inspect files\n",
        ] {
            assert_eq!(synopsis_from_page(Some(page)), None);
        }
        assert_eq!(synopsis_from_page(None), None);
    }

    fn fake_man(body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .prefix("jevify-man-")
            .tempdir()
            .unwrap()
            .keep();
        let file = dir.join("man");
        std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn a_sleeping_man_is_killed_at_the_deadline_and_a_quick_one_is_read() {
        let slow = fake_man("exec /bin/sleep 5");
        let start = Instant::now();
        let budget = Duration::from_millis(300);
        assert_eq!(
            man_page_within("tool", slow.as_os_str(), start + budget),
            None
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        let quick = fake_man("printf 'SYNOPSIS\\n    tool [-v]\\nDESCRIPTION\\n    x\\n'");
        let page = man_page_within(
            "tool",
            quick.as_os_str(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            synopsis_from_page(Some(&page)).as_deref(),
            Some("tool [-v]")
        );
        // A failing `man` yields nothing, and an expired deadline runs none.
        let failing = fake_man("exit 1");
        assert_eq!(
            man_page_within(
                "tool",
                failing.as_os_str(),
                Instant::now() + Duration::from_secs(5)
            ),
            None
        );
        assert_eq!(
            man_page_within("tool", quick.as_os_str(), Instant::now()),
            None
        );
    }
}
