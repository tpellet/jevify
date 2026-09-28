use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// How long a saved input stays on disk, matching the answer cache: raw input holds more than
/// an answer does, so it does not outlive one. A save refreshes its own file's age, so an input
/// that keeps arriving keeps its file.
pub const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Names the store writes: `<blake3-16>.log`, and the `<blake3-16>.tmp-<pid>-<n>` a crash
/// between creation and rename leaves behind. Anything else in the directory is somebody
/// else's file and is never touched.
fn prunable(name: &str) -> bool {
    let Some((stem, rest)) = name.split_once('.') else {
        return false;
    };
    stem.len() == 16
        && stem
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() && b.is_ascii_hexdigit())
        && (rest == "log" || rest.starts_with("tmp-"))
}

/// Deletes saved inputs older than `RETENTION`, and nothing else. Best effort: pruning never
/// fails a save.
///
/// The bound of the deletion is the one directory handle: entries come from reading `outputs`
/// itself, no descent and no path built from their contents. A symlink is left alone, both as
/// the directory (`outputs` replaced by a link elsewhere prunes nothing) and as an entry (only
/// a regular file is removed), so pruning cannot reach a file outside the store.
fn prune(outputs: &Path) {
    if !fs::symlink_metadata(outputs).is_ok_and(|m| m.file_type().is_dir()) {
        return;
    }
    let Ok(entries) = fs::read_dir(outputs) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        if !entry.file_name().to_str().is_some_and(prunable) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > RETENTION));
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Saves raw input; `None` skips saving. Call from the blocking pool.
/// Errors are reasons for the caller's `full output: not saved (<reason>)` line.
pub fn save(input: &[u8], directory: Option<&Path>) -> Result<PathBuf, String> {
    let directory =
        directory.ok_or_else(|| "saving disabled or directory unavailable".to_owned())?;
    let write = || -> io::Result<PathBuf> {
        let outputs = directory.join("outputs");
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&outputs)?;
        fs::set_permissions(&outputs, Permissions::from_mode(0o700))?;
        let hash = blake3::hash(input).to_hex();
        let path = outputs.join(format!("{}.log", &hash[..16]));
        // Exclusive creation avoids following an existing temporary file or sharing a writer.
        let (temporary, mut file) = loop {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let temporary = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
            {
                Ok(file) => break (temporary, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        file.set_permissions(Permissions::from_mode(0o600))?;
        file.write_all(input)?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, &path)?;
        // On write, never on read: a verb that only reads a saved input deletes nothing.
        prune(&outputs);
        Ok(path)
    };
    write().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        tempfile::Builder::new()
            .prefix("jevify-save-")
            .tempdir()
            .unwrap()
            .keep()
    }

    #[test]
    fn exact_bytes_hash_and_private_modes() {
        let directory = directory();
        let input = b"secret\0\xff\r\nlast line";
        let path = save(input, Some(&directory)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), input);
        let hash = blake3::hash(input).to_hex();
        assert_eq!(
            path,
            directory
                .join("outputs")
                .join(format!("{}.log", &hash[..16]))
        );
        assert_eq!(path.file_stem().unwrap().len(), 16);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    /// Backdates a file by `RETENTION` plus an hour, so it is past retention whatever the clock.
    fn age(path: &Path) {
        let stale = std::time::SystemTime::now() - RETENTION - Duration::from_secs(3600);
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(stale))
            .unwrap();
    }

    #[test]
    fn pruning_never_leaves_the_outputs_directory() {
        let directory = directory();
        let outside = directory.join("outside.log");
        fs::write(&outside, b"not the store's file").unwrap();
        age(&outside);
        let elsewhere = directory.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        // A real saved-input name, aged past retention, but outside the store.
        let decoy = elsewhere.join("0123456789abcdef.log");
        fs::write(&decoy, b"not the store's file either").unwrap();
        age(&decoy);
        // Saving creates outputs/, then a link inside it points at the file outside.
        save(b"input", Some(&directory)).unwrap();
        let outputs = directory.join("outputs");
        std::os::unix::fs::symlink(&decoy, outputs.join("fedcba9876543210.log")).unwrap();
        let mut deep = outputs.join("nested");
        fs::create_dir(&deep).unwrap();
        deep.push("0123456789abcdef.log");
        fs::write(&deep, b"a subdirectory is not scanned").unwrap();
        age(&deep);

        save(b"another input", Some(&directory)).unwrap();

        assert_eq!(fs::read(&outside).unwrap(), b"not the store's file");
        assert_eq!(fs::read(&decoy).unwrap(), b"not the store's file either");
        assert_eq!(fs::read(&deep).unwrap(), b"a subdirectory is not scanned");
        assert!(fs::symlink_metadata(outputs.join("fedcba9876543210.log")).is_ok());
    }

    #[test]
    fn an_outputs_directory_replaced_by_a_symlink_prunes_nothing() {
        let directory = directory();
        let elsewhere = directory.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let victim = elsewhere.join("0123456789abcdef.log");
        fs::write(&victim, b"someone else's file").unwrap();
        age(&victim);
        std::os::unix::fs::symlink(&elsewhere, directory.join("outputs")).unwrap();
        // The save itself follows the link, as any path does; the pruning does not run.
        save(b"input", Some(&directory)).unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"someone else's file");
    }
}
