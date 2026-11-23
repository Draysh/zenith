//! The daemon's log as a file. On Linux the journal keeps what a service
//! says; Windows has nothing of the kind, so there each line also goes to
//! `%LOCALAPPDATA%\Zenith\logs\zenithd.log`, where a user who is asked
//! "what does the log say" can find it.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// A log past this size is put aside, at the start and while it is being
/// written, so it never grows without end. One older file is kept.
const MAX_BYTES: u64 = 1024 * 1024;

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

/// The file being written, and how long it has become.
struct Log {
    path: PathBuf,
    max_bytes: u64,
    /// `None` only for the moment it is put aside, or when it could not
    /// be opened again after that.
    file: Option<File>,
    bytes: u64,
}

impl Log {
    fn start(path: &Path, max_bytes: u64) -> std::io::Result<Self> {
        let file = open(path, max_bytes)?;
        let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self { path: path.to_path_buf(), max_bytes, file: Some(file), bytes })
    }

    /// Add `text`. When that makes the file too long it is put aside and
    /// a new one begun, as at a start: the daemon runs for weeks.
    fn add(&mut self, text: &[u8]) {
        if self.bytes > self.max_bytes {
            // Closed first: Windows does not rename a file that is open.
            self.file = None;
        }
        if self.file.is_none() {
            self.file = open(&self.path, self.max_bytes).ok();
            // If it could not be put aside (someone is reading it), it is
            // written on and tried again when it has grown as much again.
            self.bytes = 0;
        }
        if let Some(file) = &mut self.file
            && file.write_all(text).is_ok()
        {
            self.bytes += text.len() as u64;
        }
    }
}

/// Open `path` to add to, making its folder. A file grown too long becomes
/// `<name>.old` first, in place of the one before it.
fn open(path: &Path, max_bytes: u64) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max_bytes) {
        let mut old = path.as_os_str().to_owned();
        old.push(".old");
        // Windows does not rename over a file that is there.
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(path, &old);
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path)
}

/// One line as the file has it: when, then what.
fn stamped(when: chrono::DateTime<chrono::Local>, text: &dyn std::fmt::Display) -> String {
    format!("{} {text}\n", when.format("%Y-%m-%d %H:%M:%S"))
}

/// From now on, also write every line to `path`.
pub fn start(path: &Path) -> std::io::Result<()> {
    let log = Log::start(path, MAX_BYTES)?;
    let _ = LOG.set(Mutex::new(log));
    Ok(())
}

/// Add a line to the file, when there is one. A disk that is full is not a
/// reason to stop recording, so a line that cannot be written is let go.
pub fn line(text: &dyn std::fmt::Display) {
    if let Some(log) = LOG.get() {
        let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
        log.add(stamped(chrono::Local::now(), text).as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_line_says_when() {
        let when = chrono::Local.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap();
        assert_eq!(stamped(when, &"zenithd: ready"), "2026-03-04 05:06:07 zenithd: ready\n");
    }

    #[test]
    fn lines_are_added_and_a_long_log_is_put_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/zenithd.log");
        let old = dir.path().join("logs/zenithd.log.old");
        open(&path, 10).unwrap().write_all(b"first\n").unwrap();
        open(&path, 10).unwrap().write_all(b"second\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        assert!(!old.exists());
        // Longer than allowed now: the next start begins a new file.
        open(&path, 10).unwrap().write_all(b"third\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "third\n");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "first\nsecond\n");
        std::fs::write(&path, "x".repeat(11)).unwrap();
        drop(open(&path, 10).unwrap());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "x".repeat(11), "only one old log is kept");
    }

    #[test]
    fn a_log_that_grows_too_long_while_running_is_put_aside_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zenithd.log");
        let old = dir.path().join("zenithd.log.old");
        std::fs::write(&path, "before\n").unwrap();
        let mut log = Log::start(&path, 20).unwrap();
        log.add(b"one two three\n");
        assert!(!old.exists(), "21 bytes: too long only now");
        log.add(b"four\n");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "before\none two three\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "four\n");
        for _ in 0..50 {
            log.add(b"a line of the log\n");
        }
        let (now, aside) = (std::fs::metadata(&path).unwrap().len(), std::fs::metadata(&old).unwrap().len());
        assert!(now <= 20 + 18 && aside <= 20 + 18, "neither file grows past the limit by more than a line: {now}, {aside}");
    }
}
