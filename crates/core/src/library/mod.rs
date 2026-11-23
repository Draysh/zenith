//! The clip library: the files in the clip folder, and an index of them.
//! The files are what is true; the index can always be made again from them.

pub mod db;
pub mod probe;

pub use db::{Changes, Clip, Filter, Library, LibraryError, Order, Shelf, TagCount, tags_from};
pub use probe::{Meta, probe};

use crate::games::{DESKTOP, folder_name};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::path::{Path, PathBuf};

const STAMP: &str = "%Y-%m-%d %H-%M-%S";

/// Where a clip of `game` saved at `when` goes:
/// `<dir>/<Game>/<Game> 2026-11-20 21-04-33.mp4`. A number is added when
/// that name is taken.
pub fn clip_path(dir: &Path, game: &str, when: DateTime<Local>) -> PathBuf {
    let game = folder_name(game);
    let base = format!("{game} {}", when.format(STAMP));
    let folder = dir.join(&game);
    let mut path = folder.join(format!("{base}.mp4"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{base} ({n}).mp4"));
        n += 1;
    }
    path
}

/// `path`, or the same name with a number when a file is already there.
pub fn free_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut candidate = path.to_path_buf();
    let mut n = 2;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    candidate
}

/// The game a clip file belongs to: the first folder it is in under the
/// clip folder, however many more there are below that one
/// (`Hades/2026/run.mp4` is a clip of Hades).
pub fn game_of(dir: &Path, file: &Path) -> String {
    let below = file.strip_prefix(dir).ok().and_then(|rest| {
        let mut parts = rest.components();
        let first = parts.next()?;
        // A file directly in the clip folder has no game.
        parts.next().map(|_| first.as_os_str().to_string_lossy().into_owned())
    });
    below
        .or_else(|| {
            // Not under the clip folder at all: the folder it is in.
            let folder = file.parent().filter(|p| *p != dir && file.strip_prefix(dir).is_err())?;
            Some(folder.file_name()?.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| DESKTOP.into())
}

/// When a clip was saved, in seconds since 1970: the time in its name,
/// else the file's own.
pub fn saved_at(file: &Path) -> i64 {
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // The stamp is 19 characters, at the end or before " (2)".
    let stem = stem.rsplit_once(" (").filter(|(_, n)| n.ends_with(')')).map_or(stem.as_str(), |(s, _)| s);
    let stamp = stem.len().checked_sub(19).and_then(|i| stem.get(i..));
    stamp
        .and_then(|s| NaiveDateTime::parse_from_str(s, STAMP).ok())
        .and_then(|naive| Local.from_local_datetime(&naive).earliest())
        .map(|t| t.timestamp())
        .or_else(|| {
            let modified = std::fs::metadata(file).ok()?.modified().ok()?;
            Some(modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
        })
        .unwrap_or(0)
}

/// How many folders down from the clip folder clips are looked for. Deep
/// enough for any way of sorting clips by hand, and an end to a folder
/// that links back to one above it.
const DEEPEST: usize = 8;

/// Whether a file's name is a finished video's: one of the kinds Zenith
/// plays, and not one still being written (`.part.mp4`).
pub fn is_clip_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["mp4", "mkv", "mov", "webm"].iter().any(|e| lower.ends_with(&format!(".{e}"))) && !name.contains(".part.")
}

/// Every clip file under the clip folder, in whatever folders the user
/// sorted them into. Hidden files and folders are left alone.
pub fn files(dir: &Path) -> Vec<PathBuf> {
    fn videos(dir: &Path, deeper: usize, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if deeper > 0 {
                    videos(&path, deeper - 1, out);
                }
            } else if is_clip_name(&name) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    videos(dir, DEEPEST, &mut out);
    out.sort();
    out
}

/// A clip's name as a file name: nothing a file system refuses, and none
/// when nothing is left of it.
pub fn file_stem_for(title: &str) -> Option<String> {
    let trimmed = title.trim();
    if trimmed.trim_matches(|c: char| c == '.' || c == ' ').is_empty() {
        return None;
    }
    Some(folder_name(trimmed))
}

/// Bring a video made elsewhere into the clip folder, under `game`: a copy,
/// so the file the user dropped stays where it was. A file already in the
/// clip folder is left where it is. The copy keeps the file's own time, so
/// the clip sits in the library at the day it was made.
pub fn import(dir: &Path, game: &str, file: &Path) -> std::io::Result<PathBuf> {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !is_clip_name(&name) || !file.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{name} is not a video Zenith plays")));
    }
    if file.starts_with(dir) {
        return Ok(file.to_path_buf());
    }
    let folder = dir.join(folder_name(game));
    std::fs::create_dir_all(&folder)?;
    let target = free_path(&folder.join(&name));
    // Under an unfinished file's name until every byte is there, so a
    // rescan in the middle does not take half a video for a clip.
    let part = target.with_extension(format!("part.{}", target.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
    let copied = std::fs::copy(file, &part).and_then(|_| {
        if let Ok(modified) = std::fs::metadata(file).and_then(|m| m.modified()) {
            // Without it the clip is dated today: not worth refusing the file for.
            let _ = std::fs::File::options().write(true).open(&part).and_then(|f| f.set_modified(modified));
        }
        std::fs::rename(&part, &target)
    });
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn clips_are_named_by_game_and_time() {
        let dir = tempfile::tempdir().unwrap();
        let when = at(2026, 11, 20, 21, 4, 33);
        let path = clip_path(dir.path(), "NieR:Automata™", when);
        assert_eq!(path, dir.path().join("NieR Automata/NieR Automata 2026-11-20 21-04-33.mp4"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();
        let second = clip_path(dir.path(), "NieR:Automata™", when);
        assert!(second.ends_with("NieR Automata 2026-11-20 21-04-33 (2).mp4"));
        assert_eq!(game_of(dir.path(), &path), "NieR Automata");
        assert_eq!(game_of(dir.path(), &dir.path().join("loose.mp4")), DESKTOP);
        assert_eq!(saved_at(&path), when.timestamp());
        assert_eq!(saved_at(&second), when.timestamp());
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Ace (Discord).mp4");
        assert_eq!(free_path(&path), path);
        std::fs::write(&path, b"").unwrap();
        let second = free_path(&path);
        assert_eq!(second, dir.path().join("Ace (Discord) (2).mp4"));
        std::fs::write(&second, b"").unwrap();
        assert_eq!(free_path(&path), dir.path().join("Ace (Discord) (3).mp4"));
    }

    #[test]
    fn a_file_without_a_stamp_uses_its_own_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("renamed by hand.mp4");
        std::fs::write(&path, b"").unwrap();
        let now = Local::now().timestamp();
        assert!((saved_at(&path) - now).abs() < 5);
        assert_eq!(saved_at(&dir.path().join("gone.mp4")), 0);
    }

    #[test]
    fn only_finished_videos_are_clips() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Game/deeper")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        for f in ["a.mp4", "Game/b.MKV", "Game/c.part.mp4", "Game/notes.txt", "Game/deeper/d.mp4", ".hidden/e.mp4"] {
            std::fs::write(d.join(f), b"").unwrap();
        }
        assert_eq!(files(d), [d.join("Game/b.MKV"), d.join("Game/deeper/d.mp4"), d.join("a.mp4")]);
    }

    #[test]
    fn clips_in_folders_within_folders_belong_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Hades/2026/November/best")).unwrap();
        let deep = d.join("Hades/2026/November/best/run.mp4");
        std::fs::write(&deep, b"").unwrap();
        std::fs::write(d.join("Hades/2026/first.mkv"), b"").unwrap();
        assert_eq!(files(d), [d.join("Hades/2026/November/best/run.mp4"), d.join("Hades/2026/first.mkv")]);
        assert_eq!(game_of(d, &deep), "Hades");
        assert_eq!(game_of(d, &d.join("Hades/a.mp4")), "Hades");
        assert_eq!(game_of(d, &d.join("a.mp4")), DESKTOP);
        assert_eq!(game_of(d, Path::new("/elsewhere/Videos/a.mp4")), "Videos");
    }

    #[test]
    fn folders_too_deep_to_be_sorting_are_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut folder = dir.path().to_path_buf();
        for level in 0..DEEPEST + 2 {
            std::fs::write(folder.join(format!("{level}.mp4")), b"").unwrap();
            folder = folder.join("in");
            std::fs::create_dir(&folder).unwrap();
        }
        assert_eq!(files(dir.path()).len(), DEEPEST + 1);
    }

    #[test]
    fn a_name_becomes_a_file_name() {
        assert_eq!(file_stem_for("  Ace: 1v5?  ").as_deref(), Some("Ace 1v5"));
        assert_eq!(file_stem_for("Rampage").as_deref(), Some("Rampage"));
        assert_eq!(file_stem_for(" .. "), None);
        assert_eq!(file_stem_for(""), None);
    }

    #[test]
    fn a_dropped_video_is_copied_under_its_game() {
        let dir = tempfile::tempdir().unwrap();
        let clips = dir.path().join("Clips");
        let elsewhere = dir.path().join("Downloads");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let source = elsewhere.join("ace.mp4");
        std::fs::write(&source, b"a video").unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options().write(true).open(&source).unwrap().set_modified(old).unwrap();

        let first = import(&clips, "Hades: II", &source).unwrap();
        assert_eq!(first, clips.join("Hades II/ace.mp4"));
        assert_eq!(std::fs::read(&first).unwrap(), b"a video");
        assert!(source.is_file(), "the dropped file stays where it was");
        assert_eq!(saved_at(&first), 1_700_000_000, "and the copy keeps its day");
        // The same name again sits beside the first; nothing unfinished is left.
        assert_eq!(import(&clips, "Hades: II", &source).unwrap(), clips.join("Hades II/ace (2).mp4"));
        assert_eq!(files(&clips).len(), 2);
        assert_eq!(std::fs::read_dir(clips.join("Hades II")).unwrap().count(), 2);
        // Already in the library: left where it is.
        assert_eq!(import(&clips, "Other", &first).unwrap(), first);
        assert!(import(&clips, "Hades", &elsewhere.join("notes.txt")).is_err());
        assert!(import(&clips, "Hades", &elsewhere.join("gone.mp4")).is_err());
    }
}
