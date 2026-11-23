//! The log: every line goes to stderr, where the daemon reads it, and to
//! `zenith-rec.log`, which is what is left to look at after a first run on
//! someone else's machine went wrong.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// A log this big is put aside as `.old` when the next run starts.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

/// Start writing to `zenith-rec.log` in `dir` as well as to stderr. A log
/// that cannot be opened is said once and done without.
pub fn init(dir: &Path) {
    let path = dir.join("zenith-rec.log");
    let _ = std::fs::create_dir_all(dir);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    let file = OpenOptions::new().create(true).append(true).open(&path);
    if let Err(err) = &file {
        stderr(&format!("zenith-rec: no log file at {}: {err}", path.display()));
    }
    let _ = FILE.set(Mutex::new(file.ok()));
}

/// Write a line to stderr. Unlike `eprintln!` this does not panic when
/// stderr is a pipe whose reader has gone, which is how a daemon dies.
pub fn stderr(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

/// Write one line, with the time in front.
pub fn line(text: &str) {
    let stamped = format!("{} {text}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"));
    stderr(&stamped);
    if let Some(file) = FILE.get()
        && let Ok(mut file) = file.lock()
        && let Some(file) = file.as_mut()
    {
        let _ = writeln!(file, "{stamped}");
    }
}

/// `note!("...", ..)`: a line in the log.
#[macro_export]
macro_rules! note {
    ($($arg:tt)*) => { $crate::log::line(&format!($($arg)*)) };
}
