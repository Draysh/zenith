//! The window's own log. The recorder has had one all along; the window
//! (`zenith-clips`) said what it had to say on stderr, which on Windows
//! goes nowhere and elsewhere is gone with the session. So what it says is
//! kept in `logs/zenith-clips.log` beside the recorder's: one file across
//! every start, each start and each proper close marked by a line, so that
//! a start that never closed (a crash) is there to be seen afterwards,
//! with whatever was said just before it.
//!
//! Writing the file is the window's (it points its stderr at it, see
//! `keepLog` in the app's `cpp/helpers.h`); what is here is where the file
//! is, the lines that mark a start and a close, and reading it back for
//! the report.

use std::path::{Path, PathBuf};

/// A log past this size is put aside at the next start, so it never grows
/// without end. One older file is kept.
const MAX_BYTES: u64 = 1024 * 1024;

/// How a line that marks a start begins, and one that marks a close, and
/// one the window writes as it goes down.
const STARTED: &str = "==== zenith-clips ";
const CLOSED: &str = "==== closed";
const CRASHED: &str = "==== crashed";

/// The window's log file.
pub fn file() -> PathBuf {
    crate::paths::data_dir().join("logs").join("zenith-clips.log")
}

/// The file a log grown too long is put aside as.
pub fn old(path: &Path) -> PathBuf {
    let mut old = path.as_os_str().to_owned();
    old.push(".old");
    PathBuf::from(old)
}

/// Make the log ready to be added to: its folder is made, and a file
/// grown too long becomes `<name>.old`, in place of the one before it.
pub fn prepare(path: &Path) {
    prepare_within(path, MAX_BYTES);
}

fn prepare_within(path: &Path, max_bytes: u64) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max_bytes) {
        let old = old(path);
        // Windows does not rename over a file that is there.
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(path, &old);
    }
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// The line that marks a start of the window.
pub fn started(version: &str) -> String {
    format!("{STARTED}{version} started {}", now())
}

/// The line that marks the window closing as it should.
pub fn closed() -> String {
    format!("{CLOSED} {}", now())
}

/// What the report says of the window's log: how each of its last starts
/// ended, then its last `lines` lines. `text` is the log, oldest first.
pub fn for_report(text: &str, lines: usize) -> Vec<String> {
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    // Each start, and how it ended: closed, crashed (it said so as it
    // went down), or neither.
    let mut starts: Vec<(&str, &str)> = Vec::new();
    for line in &all {
        if let Some(rest) = line.strip_prefix(STARTED) {
            starts.push((rest, ""));
        } else if let Some(last) = starts.last_mut() {
            if line.starts_with(CLOSED) {
                last.1 = "closed";
            } else if line.starts_with(CRASHED) {
                last.1 = "CRASHED";
            }
        }
    }
    let mut out = Vec::new();
    let from = starts.len().saturating_sub(6);
    for (at, (what, ended)) in starts.iter().enumerate().skip(from) {
        let ended = match (*ended, at + 1 == starts.len()) {
            ("", true) => "open now, or it did not close",
            ("", false) => "DID NOT CLOSE (it crashed, or was ended from outside)",
            (ended, _) => ended,
        };
        out.push(format!("{what}: {ended}"));
    }
    if !out.is_empty() {
        out.push(String::new());
    }
    out.extend(all[all.len().saturating_sub(lines)..].iter().map(|l| l.to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_that_never_closed_is_told_from_one_that_did() {
        let log = "==== zenith-clips 0.8.0 started 2026-11-20 10:00:00\n\
                   qml: something\n\
                   ==== closed 2026-11-20 10:05:00\n\
                   ==== zenith-clips 0.8.0 started 2026-11-21 11:00:00\n\
                   qml: the last thing said\n\
                   ==== zenith-clips 0.8.0 started 2026-11-21 11:01:00\n\
                   ==== crashed: exception 0xc0000005 at\n\
                   \x20 Qt6Quick.dll+0x1234\n\
                   ==== zenith-clips 0.8.1 started 2026-11-22 09:00:00\n";
        let said = for_report(log, 3);
        assert_eq!(
            said,
            [
                "0.8.0 started 2026-11-20 10:00:00: closed",
                "0.8.0 started 2026-11-21 11:00:00: DID NOT CLOSE (it crashed, or was ended from outside)",
                "0.8.0 started 2026-11-21 11:01:00: CRASHED",
                "0.8.1 started 2026-11-22 09:00:00: open now, or it did not close",
                "",
                "==== crashed: exception 0xc0000005 at",
                "  Qt6Quick.dll+0x1234",
                "==== zenith-clips 0.8.1 started 2026-11-22 09:00:00",
            ]
        );
        assert!(for_report("", 10).is_empty());
    }

    #[test]
    fn a_long_log_is_put_aside_and_one_old_one_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/zenith-clips.log");
        prepare_within(&path, 10);
        std::fs::write(&path, "short\n").unwrap();
        prepare_within(&path, 10);
        assert!(path.exists() && !old(&path).exists());
        std::fs::write(&path, "longer than ten bytes\n").unwrap();
        prepare_within(&path, 10);
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(old(&path)).unwrap(), "longer than ten bytes\n");
    }

    #[test]
    fn the_marks_are_what_the_report_looks_for() {
        assert!(started("0.8.0").starts_with("==== zenith-clips 0.8.0 started 20"));
        assert!(closed().starts_with(CLOSED));
    }
}
