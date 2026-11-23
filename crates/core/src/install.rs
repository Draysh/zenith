//! Putting a new version in place of this one, and going back when it does
//! not start.
//!
//! A release for Linux is a `.tar.gz` of the programs. Each is written
//! beside the one it replaces, the new app is started once to see that it
//! runs here, and only then are the files switched, with the old ones kept.
//! The first start of the new version is a trial: if the app is started a
//! second time without the first having run well, the old files come back
//! and that version is not installed by itself again.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The programs a release may replace.
pub const APP: &str = if cfg!(windows) { "zenith-clips.exe" } else { "zenith-clips" };
pub const DAEMON: &str = if cfg!(windows) { "zenithd.exe" } else { "zenithd" };
const PROGRAMS: [&str; 2] = [APP, DAEMON];

const TRIAL: &str = ".zenith-update.json";
const ROLLED_BACK: &str = ".zenith-rolled-back";

/// How this copy came to be here, which decides whether it updates itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Here {
    /// Under the user's own folders: it replaces itself.
    Portable,
    /// Put there by a package (`/usr`, `/opt`): the package manager updates it.
    Packaged,
    /// Built from the source and run from there: it is left alone.
    Development,
}

impl Here {
    pub fn word(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::Packaged => "packaged",
            Self::Development => "development",
        }
    }
}

/// Where the running program is, without the mark Linux adds to a file
/// that was replaced while it ran.
pub fn current_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let text = exe.to_string_lossy();
    Some(text.strip_suffix(" (deleted)").map(PathBuf::from).unwrap_or(exe))
}

/// What kind of copy the program at `exe` is.
pub fn here_at(exe: &Path, debug: bool) -> Here {
    let dir = exe.parent().unwrap_or(Path::new("/"));
    if debug || dir.components().any(|c| c.as_os_str() == "target") {
        Here::Development
    } else if dir.starts_with("/usr") || dir.starts_with("/opt") {
        Here::Packaged
    } else {
        Here::Portable
    }
}

pub fn here() -> Here {
    current_exe().map(|exe| here_at(&exe, cfg!(debug_assertions))).unwrap_or(Here::Development)
}

/// The programs in a release's `.tar.gz`, by name. Anything else in it
/// (the launcher entry, the service) is for a first install and is skipped.
pub fn unpack(archive: &[u8]) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut found = Vec::new();
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if entry.header().entry_type().is_file() && PROGRAMS.contains(&name.as_str()) {
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            found.push((name, bytes));
        }
    }
    if !found.iter().any(|(name, _)| name == APP) {
        return Err(std::io::Error::other("the download does not hold the app"));
    }
    Ok(found)
}

fn staged(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!(".{name}.new"))
}

fn kept(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!(".{name}.old"))
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Trial {
    from: String,
    to: String,
    /// How many times the new version has been started.
    starts: u32,
    /// The programs that were switched.
    programs: Vec<String>,
}

/// Put `programs` in `dir` in place of the ones there. `runs` starts the
/// new app (`--version`) and gives what it printed: it must name the
/// version, or nothing is changed.
pub fn put_in_place(
    dir: &Path,
    programs: &[(String, Vec<u8>)],
    from: &str,
    to: &str,
    runs: impl Fn(&Path) -> std::io::Result<String>,
) -> std::io::Result<()> {
    let clean = |dir: &Path| {
        for (name, _) in programs {
            let _ = std::fs::remove_file(staged(dir, name));
        }
    };
    for (name, bytes) in programs {
        let path = staged(dir, name);
        std::fs::write(&path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    let said = runs(&staged(dir, APP)).inspect_err(|_| clean(dir))?;
    if !said.split_whitespace().any(|word| word == to) {
        clean(dir);
        return Err(std::io::Error::other(format!("the new version does not start here (it said: {})", said.trim())));
    }
    for (name, _) in programs {
        let now = dir.join(name);
        let old = kept(dir, name);
        let _ = std::fs::remove_file(&old);
        if now.exists() {
            // A second name for the old file, so it is still there after
            // the new one takes its place.
            std::fs::hard_link(&now, &old).or_else(|_| std::fs::copy(&now, &old).map(|_| ()))?;
        }
        std::fs::rename(staged(dir, name), &now)?;
    }
    let trial = Trial { from: from.into(), to: to.into(), starts: 0, programs: programs.iter().map(|(n, _)| n.clone()).collect() };
    std::fs::write(dir.join(TRIAL), serde_json::to_vec(&trial).expect("a trial serializes"))?;
    let _ = std::fs::remove_file(dir.join(ROLLED_BACK));
    Ok(())
}

/// What the start of the app found.
#[derive(Debug, PartialEq, Eq)]
pub enum Start {
    /// Nothing is being tried.
    Usual,
    /// The first start of a new version.
    Trying,
    /// The new version did not run well: the old one is back in place, and
    /// is the one to start.
    WentBack { to: String },
}

/// Called as the app starts, running version `current` from `dir`.
pub fn on_start(dir: &Path, current: &str) -> Start {
    let path = dir.join(TRIAL);
    let Some(mut trial) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Trial>(&b).ok()) else {
        return Start::Usual;
    };
    if trial.to != current {
        // Not the version that was being tried: the trial is over.
        let _ = std::fs::remove_file(&path);
        return Start::Usual;
    }
    trial.starts += 1;
    if trial.starts == 1 {
        let _ = std::fs::write(&path, serde_json::to_vec(&trial).expect("a trial serializes"));
        return Start::Trying;
    }
    go_back(dir, &trial);
    Start::WentBack { to: trial.from }
}

fn go_back(dir: &Path, trial: &Trial) {
    for name in &trial.programs {
        let old = kept(dir, name);
        if old.exists() {
            let _ = std::fs::rename(&old, dir.join(name));
        }
    }
    let _ = std::fs::remove_file(dir.join(TRIAL));
    let _ = std::fs::write(dir.join(ROLLED_BACK), &trial.to);
}

/// The new version has run well (for a while, or to a clean end): the
/// trial is over and the old files go.
pub fn started_well(dir: &Path) {
    let path = dir.join(TRIAL);
    if let Some(trial) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Trial>(&b).ok()) {
        for name in &trial.programs {
            let _ = std::fs::remove_file(kept(dir, name));
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// The version that did not start here and was taken out again, if any.
pub fn rolled_back(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(ROLLED_BACK)).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, name, *bytes).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[cfg(unix)]
    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn a_copy_updates_itself_only_from_the_users_own_folders() {
        assert_eq!(here_at(Path::new("/home/a/.local/bin/zenith-clips"), false), Here::Portable);
        assert_eq!(here_at(Path::new("/usr/bin/zenith-clips"), false), Here::Packaged);
        assert_eq!(here_at(Path::new("/opt/zenith/zenith-clips"), false), Here::Packaged);
        assert_eq!(here_at(Path::new("/home/a/src/zenith/target/release/zenith-clips"), false), Here::Development);
        assert_eq!(here_at(Path::new("/home/a/.local/bin/zenith-clips"), true), Here::Development);
    }

    // Switching the programs in place is how Linux updates; Windows runs a setup program.
    #[cfg(unix)]
    #[test]
    fn only_the_programs_are_taken_from_a_release() {
        let tar = archive(&[("zenith-clips", b"app"), ("zenithd", b"daemon"), ("zenithd.service", b"unit"), ("evil", b"x")]);
        let found = unpack(&tar).unwrap();
        assert_eq!(found, [("zenith-clips".to_string(), b"app".to_vec()), ("zenithd".to_string(), b"daemon".to_vec())]);
        assert!(unpack(&archive(&[("zenithd", b"daemon")])).is_err(), "a release without the app is no release");
        assert!(unpack(b"not an archive").is_err());
    }

    // Switching the programs in place is how Linux updates; Windows runs a setup program.
    #[cfg(unix)]
    #[test]
    fn a_new_version_takes_the_old_ones_place_and_the_old_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join(APP), "old app").unwrap();
        std::fs::write(d.join(DAEMON), "old daemon").unwrap();
        let new = [(APP.to_string(), b"new app".to_vec()), (DAEMON.to_string(), b"new daemon".to_vec())];
        put_in_place(d, &new, "0.1.0", "0.2.0", |_| Ok("zenith-clips 0.2.0".into())).unwrap();
        assert_eq!((read(d, APP), read(d, DAEMON)), ("new app".into(), "new daemon".into()));
        assert_eq!(read(d, ".zenith-clips.old"), "old app");

        // Its first start is a trial; running well ends it and drops the old files.
        assert_eq!(on_start(d, "0.2.0"), Start::Trying);
        started_well(d);
        assert_eq!(on_start(d, "0.2.0"), Start::Usual);
        assert!(!d.join(".zenith-clips.old").exists() && !d.join(".zenithd.old").exists());
        assert_eq!(rolled_back(d), None);
    }

    // Switching the programs in place is how Linux updates; Windows runs a setup program.
    #[cfg(unix)]
    #[test]
    fn a_version_that_does_not_start_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join(APP), "old app").unwrap();
        let new = [(APP.to_string(), b"new app".to_vec())];
        let err = put_in_place(d, &new, "0.1.0", "0.2.0", |_| Ok("zenith-clips 0.1.5".into())).unwrap_err();
        assert!(err.to_string().contains("does not start here"), "{err}");
        assert!(put_in_place(d, &new, "0.1.0", "0.2.0", |_| Err(std::io::Error::other("no such library"))).is_err());
        assert_eq!(read(d, APP), "old app");
        assert_eq!(std::fs::read_dir(d).unwrap().count(), 1, "nothing is left lying about");
        assert_eq!(on_start(d, "0.1.0"), Start::Usual);
    }

    // Switching the programs in place is how Linux updates; Windows runs a setup program.
    #[cfg(unix)]
    #[test]
    fn a_second_start_without_a_good_first_one_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join(APP), "old app").unwrap();
        std::fs::write(d.join(DAEMON), "old daemon").unwrap();
        let new = [(APP.to_string(), b"new app".to_vec()), (DAEMON.to_string(), b"new daemon".to_vec())];
        put_in_place(d, &new, "0.1.0", "0.2.0", |_| Ok("zenith-clips 0.2.0".into())).unwrap();
        assert_eq!(on_start(d, "0.2.0"), Start::Trying);
        // It fell over before it ran well, and is started again.
        assert_eq!(on_start(d, "0.2.0"), Start::WentBack { to: "0.1.0".into() });
        assert_eq!((read(d, APP), read(d, DAEMON)), ("old app".into(), "old daemon".into()));
        assert_eq!(rolled_back(d), Some("0.2.0".into()));
        assert_eq!(on_start(d, "0.1.0"), Start::Usual);
        // Installing a later version forgets the one that failed.
        put_in_place(d, &new, "0.1.0", "0.2.1", |_| Ok("zenith-clips 0.2.1".into())).unwrap();
        assert_eq!(rolled_back(d), None);
    }
}
