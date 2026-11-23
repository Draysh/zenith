//! The index of the clip folder, in SQLite: what makes the browser fast,
//! and where a clip's own details are kept (its name, favourite, edit).

use super::probe::{Meta, ProbeError};
use crate::edit::Edit;
use crate::recorder::Track;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("the clip index could not be used: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("that clip is not in the library any more")]
    Missing,
}

type Result<T> = std::result::Result<T, LibraryError>;

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: i64,
    pub path: PathBuf,
    pub game: String,
    /// The name shown; the file's name when none was given.
    pub title: String,
    /// Seconds since 1970.
    pub saved_at: i64,
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub bytes: u64,
    pub favorite: bool,
    pub tracks: Vec<Track>,
    pub edit: Option<Edit>,
    /// The user's own words for finding it again, in lower case.
    pub tags: Vec<String>,
    pub notes: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Order {
    #[default]
    Newest,
    Oldest,
    Longest,
    Shortest,
    Largest,
    Smallest,
}

impl Order {
    /// The order a screen asks for by name; newest first when it is none
    /// of them.
    pub fn named(name: &str) -> Self {
        match name {
            "oldest" => Self::Oldest,
            "longest" => Self::Longest,
            "shortest" => Self::Shortest,
            "largest" => Self::Largest,
            "smallest" => Self::Smallest,
            _ => Self::Newest,
        }
    }

    /// Clips that are as long or as large as each other go newest first.
    fn sql(self) -> &'static str {
        match self {
            Self::Newest => " ORDER BY saved_at DESC, id DESC",
            Self::Oldest => " ORDER BY saved_at ASC, id ASC",
            Self::Longest => " ORDER BY seconds DESC, saved_at DESC, id DESC",
            Self::Shortest => " ORDER BY seconds ASC, saved_at DESC, id DESC",
            Self::Largest => " ORDER BY bytes DESC, saved_at DESC, id DESC",
            Self::Smallest => " ORDER BY bytes ASC, saved_at DESC, id DESC",
        }
    }
}

/// What to list. The default is every clip, newest first.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub game: Option<String>,
    pub favorites: bool,
    /// Only clips with this tag.
    pub tag: Option<String>,
    /// Words that must all be in the title, the game's name, a tag or the
    /// notes. A word written `#so` is only looked for among the tags.
    pub search: String,
    pub order: Order,
}

/// A tag and how many clips have it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagCount {
    pub tag: String,
    pub clips: u32,
}

/// A game and how many clips it has.
#[derive(Clone, Debug, PartialEq)]
pub struct Shelf {
    pub game: String,
    pub clips: u32,
    pub latest: i64,
}

/// What a rescan changed.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Changes {
    pub added: u32,
    pub removed: u32,
    /// Files that were renamed or moved to another folder: still the same
    /// clips, with what the user set.
    pub moved: u32,
}

/// Before an older index is moved up, a copy of it is put beside it
/// (`library.v1.db`): the index is the only place a clip's name, favourite
/// and edit are kept. A copy that cannot be made does not stop the move,
/// which only adds to the table.
fn keep_a_copy_before_moving_up(db: &Connection, file: &Path) {
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
    let clips: i64 = db.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0)).unwrap_or(0);
    if version >= SCHEMA_VERSION || clips == 0 {
        return;
    }
    let copy = file.with_extension(format!("v{}.db", version.max(1)));
    let _ = std::fs::remove_file(&copy);
    let _ = db.execute("VACUUM INTO ?1", [copy.to_string_lossy()]);
}

pub struct Library {
    db: Connection,
    /// Files a probe refused, with the size and time they had then: they
    /// are not read again until they change.
    refused: std::cell::RefCell<std::collections::HashMap<PathBuf, (u64, Option<std::time::SystemTime>)>>,
    /// Where what could not be read of a clip is put before it is written
    /// over (see `keep_what_cannot_be_read`): beside the index.
    unread: Option<PathBuf>,
}

/// The layout of the index. An older index is moved up to it when it is
/// opened, one step at a time, keeping every row.
const SCHEMA_VERSION: i64 = 2;

/// The index as version 1 had it (Zenith 0.1 to 0.5). It stays as it was:
/// a new index is made this way and then moved up like any other, so there
/// is one road to the newest layout and every index has travelled it.
const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS clips (
    id        INTEGER PRIMARY KEY,
    path      TEXT NOT NULL UNIQUE,
    game      TEXT NOT NULL,
    title     TEXT,
    saved_at  INTEGER NOT NULL,
    seconds   REAL NOT NULL,
    width     INTEGER NOT NULL,
    height    INTEGER NOT NULL,
    fps       REAL NOT NULL,
    bytes     INTEGER NOT NULL,
    favorite  INTEGER NOT NULL DEFAULT 0,
    tracks    TEXT NOT NULL,
    edit      TEXT
);
CREATE INDEX IF NOT EXISTS clips_by_time ON clips (saved_at);
CREATE INDEX IF NOT EXISTS clips_by_game ON clips (game, saved_at);
";

/// What each version added to the one before it: `STEPS[0]` moves an index
/// from version 1 to 2. Steps only ever add, and what they add has a
/// default, so an older Zenith (the recorder, not yet restarted after an
/// update) can still write clips into a newer index.
const STEPS: &[&str] = &[
    // 2 (Zenith 0.6): tags and notes, and the time a clip was put in the
    // trash while that can still be undone.
    "ALTER TABLE clips ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
     ALTER TABLE clips ADD COLUMN notes TEXT NOT NULL DEFAULT '';
     ALTER TABLE clips ADD COLUMN trashed_at INTEGER;",
];

const COLUMNS: &str = "id, path, game, title, saved_at, seconds, width, height, fps, bytes, favorite, tracks, edit, tags, notes";

/// Move an index up to the newest layout. Returns the version it had.
///
/// The steps and the new version number are written together or not at
/// all, and the version is read after the index is locked: the recorder
/// and the window may both open an old index at the same moment, and the
/// second one must find the work done.
///
/// An index that has the newest layout already is not locked at all: every
/// window and every rescan opens it, and none of them should wait for, or
/// be refused by, whoever is writing to it at that moment.
fn migrate(db: &Connection) -> Result<i64> {
    let found: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if found >= SCHEMA_VERSION {
        return Ok(found);
    }
    let all = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)?;
    all.execute_batch(SCHEMA_V1)?;
    let found: i64 = all.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    // 0 is an index from before the layouts were numbered, or a new one:
    // both are version 1's table.
    let version = found.max(1);
    for step in STEPS.iter().skip((version - 1) as usize) {
        all.execute_batch(step)?;
    }
    if version < SCHEMA_VERSION {
        all.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    all.commit()?;
    Ok(found)
}

/// Tags as the user typed them (`Ace, #clutch`), made into the ones kept:
/// lower case, no `#`, one of each, in the order given.
pub fn tags_from(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for word in text.split([',', '\n', '#']) {
        let tag = word.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if !tag.is_empty() && !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

/// Whether a clip has what a search asks for: every word, in its title,
/// its game, its tags or its notes.
fn matches(clip: &Clip, words: &[String]) -> bool {
    let (title, game, notes) = (clip.title.to_lowercase(), clip.game.to_lowercase(), clip.notes.to_lowercase());
    words.iter().all(|word| match word.strip_prefix('#') {
        Some(tag) => !tag.is_empty() && clip.tags.iter().any(|t| t.starts_with(tag)),
        None => title.contains(word) || game.contains(word) || notes.contains(word) || clip.tags.iter().any(|t| t.contains(word)),
    })
}

/// The tracks a clip most likely has when nothing recorded which they are.
pub fn guess_tracks(count: u32) -> Vec<Track> {
    match count {
        0 => vec![],
        1 => vec![Track::Game],
        2 => vec![Track::Game, Track::Microphone],
        n => [Track::Game, Track::Chat, Track::Microphone].into_iter().cycle().take(n as usize).collect(),
    }
}

fn clip(row: &rusqlite::Row) -> rusqlite::Result<Clip> {
    let path = PathBuf::from(row.get::<_, String>(1)?);
    let title: Option<String> = row.get(3)?;
    let title = title.unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let tracks: String = row.get(11)?;
    let edit: Option<String> = row.get(12)?;
    let tags: String = row.get(13)?;
    let seconds: f64 = row.get(5)?;
    Ok(Clip {
        id: row.get(0)?,
        path,
        game: row.get(2)?,
        title,
        saved_at: row.get(4)?,
        seconds,
        width: row.get(6)?,
        height: row.get(7)?,
        fps: row.get(8)?,
        bytes: row.get::<_, i64>(9)? as u64,
        favorite: row.get(10)?,
        tracks: serde_json::from_str(&tracks).unwrap_or_default(),
        // What is kept is not trusted as it is: see `Edit::fitted`.
        edit: edit.and_then(|e| serde_json::from_str::<Edit>(&e).ok()).and_then(|e| e.fitted(seconds)),
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        notes: row.get(14)?,
    })
}

impl Library {
    /// Open the index at `file`, making it if it is not there.
    pub fn open(file: &Path) -> Result<Self> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let db = Connection::open(file)?;
        keep_a_copy_before_moving_up(&db, file);
        Self::with(db, Some(file.with_extension("unread.jsonl")))
    }

    pub fn in_memory() -> Result<Self> {
        Self::with(Connection::open_in_memory()?, None)
    }

    fn with(db: Connection, unread: Option<PathBuf>) -> Result<Self> {
        // The daemon writes while the app reads.
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&db)?;
        Ok(Self { db, refused: Default::default(), unread })
    }

    /// Called before a clip's edit or tags are written over. What is kept
    /// there now may be something this Zenith cannot read (written by a
    /// newer one, or damaged): the clip then shows as having none, and the
    /// next change would replace it for good. It is put in a file beside
    /// the index first (`library.unread.jsonl`, a line each), so it can be
    /// looked at and put back. The write itself goes ahead either way:
    /// a clip that could never be edited again would be the worse loss.
    fn keep_what_cannot_be_read(&self, id: i64, column: &str, reads: impl Fn(&str) -> bool) {
        let Some(file) = &self.unread else { return };
        let kept = self.db.query_row(&format!("SELECT path, {column} FROM clips WHERE id = ?1"), [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)));
        let Ok((path, Some(text))) = kept else { return };
        if reads(&text) {
            return;
        }
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let line = serde_json::json!({ "clip": id, "path": path, "what": column, "was": text, "at": at });
        if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
            use std::io::Write;
            let _ = writeln!(out, "{line}");
        }
    }

    /// Add a clip file, or bring its details up to date. What the user set
    /// (title, favourite, edit) is kept. `tracks` is what the caller knows
    /// the file's audio tracks to be, and replaces an earlier guess.
    pub fn add(&self, path: &Path, game: &str, saved_at: i64, meta: &Meta, tracks: &[Track]) -> Result<i64> {
        self.put(path, game, saved_at, meta, tracks, true)
    }

    /// Add a file found in the folder, whose tracks can only be guessed
    /// from their number. A clip already known keeps the tracks it has.
    fn add_found(&self, path: &Path, game: &str, saved_at: i64, meta: &Meta) -> Result<i64> {
        self.put(path, game, saved_at, meta, &guess_tracks(meta.audio_tracks), false)
    }

    fn put(&self, path: &Path, game: &str, saved_at: i64, meta: &Meta, tracks: &[Track], known: bool) -> Result<i64> {
        let tracks = serde_json::to_string(tracks).expect("tracks serialize");
        let also = if known { ", tracks = ?9" } else { "" };
        self.db.execute(
            &format!(
                "INSERT INTO clips (path, game, saved_at, seconds, width, height, fps, bytes, tracks)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (path) DO UPDATE SET game = ?2, seconds = ?4, width = ?5, height = ?6, fps = ?7, bytes = ?8{also}"
            ),
            params![path.to_string_lossy(), game, saved_at, meta.seconds, meta.width, meta.height, meta.fps, meta.bytes as i64, tracks],
        )?;
        Ok(self.db.query_row("SELECT id FROM clips WHERE path = ?1", [path.to_string_lossy()], |r| r.get(0))?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Clip>> {
        Ok(self.db.query_row(&format!("SELECT {COLUMNS} FROM clips WHERE id = ?1"), [id], clip).optional()?)
    }

    /// The clips a filter asks for. One in the trash, waiting for the
    /// moment an undo is still offered, is not listed.
    pub fn list(&self, filter: &Filter) -> Result<Vec<Clip>> {
        let mut sql = format!("SELECT {COLUMNS} FROM clips WHERE trashed_at IS NULL");
        let mut args: Vec<String> = Vec::new();
        if let Some(game) = &filter.game {
            args.push(game.clone());
            sql += &format!(" AND game = ?{}", args.len());
        }
        if filter.favorites {
            sql += " AND favorite";
        }
        sql += filter.order.sql();
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), clip)?;
        let mut clips: Vec<Clip> = rows.collect::<rusqlite::Result<_>>()?;
        if let Some(tag) = &filter.tag {
            clips.retain(|c| c.tags.contains(tag));
        }
        // Matched against what the user sees and wrote, never the folders
        // the file is in.
        let words: Vec<String> = filter.search.split_whitespace().map(str::to_lowercase).collect();
        if !words.is_empty() {
            clips.retain(|c| matches(c, &words));
        }
        Ok(clips)
    }

    /// The games that have clips, the one clipped last first.
    pub fn shelves(&self) -> Result<Vec<Shelf>> {
        let mut stmt = self.db.prepare("SELECT game, COUNT(*), MAX(saved_at) FROM clips WHERE trashed_at IS NULL GROUP BY game ORDER BY 3 DESC")?;
        let rows = stmt.query_map([], |r| Ok(Shelf { game: r.get(0)?, clips: r.get(1)?, latest: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_favorite(&self, id: i64, favorite: bool) -> Result<()> {
        self.db.execute("UPDATE clips SET favorite = ?2 WHERE id = ?1", params![id, favorite])?;
        Ok(())
    }

    /// Give a clip a name of its own; an empty one goes back to the file's.
    pub fn set_title(&self, id: i64, title: &str) -> Result<()> {
        let title = Some(title.trim()).filter(|t| !t.is_empty());
        self.db.execute("UPDATE clips SET title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    /// Keep a clip's edit; `None` clears it, and the clip is one not edited yet.
    pub fn set_edit(&self, id: i64, edit: Option<&Edit>) -> Result<()> {
        let json = edit.map(|e| serde_json::to_string(e).expect("an edit serializes"));
        self.keep_what_cannot_be_read(id, "edit", |kept| serde_json::from_str::<Edit>(kept).is_ok());
        self.db.execute("UPDATE clips SET edit = ?2 WHERE id = ?1", params![id, json])?;
        Ok(())
    }

    /// Every tag in use and how many clips have it, the most used first.
    pub fn tags(&self) -> Result<Vec<TagCount>> {
        let mut stmt = self.db.prepare("SELECT tags FROM clips WHERE trashed_at IS NULL AND tags != '[]'")?;
        let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
        for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
            for tag in serde_json::from_str::<Vec<String>>(&row?).unwrap_or_default() {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut tags: Vec<TagCount> = counts.into_iter().map(|(tag, clips)| TagCount { tag, clips }).collect();
        // Already by name; the sort keeps that among tags used as often.
        tags.sort_by_key(|t| std::cmp::Reverse(t.clips));
        Ok(tags)
    }

    /// The tags a clip has, replacing the ones it had.
    pub fn set_tags(&self, id: i64, tags: &[String]) -> Result<()> {
        let tags = serde_json::to_string(&tags_from(&tags.join(","))).unwrap_or_else(|_| "[]".into());
        self.keep_what_cannot_be_read(id, "tags", |kept| serde_json::from_str::<Vec<String>>(kept).is_ok());
        self.db.execute("UPDATE clips SET tags = ?2 WHERE id = ?1", params![id, tags])?;
        Ok(())
    }

    /// Give every one of `ids` the tags in `tags`, beside the ones it has.
    pub fn add_tags(&self, ids: &[i64], tags: &[String]) -> Result<()> {
        let all = self.db.unchecked_transaction()?;
        for id in ids {
            let Some(clip) = self.get(*id)? else { continue };
            let both: Vec<String> = clip.tags.into_iter().chain(tags.iter().cloned()).collect();
            self.set_tags(*id, &both)?;
        }
        Ok(all.commit()?)
    }

    pub fn set_notes(&self, id: i64, notes: &str) -> Result<()> {
        self.db.execute("UPDATE clips SET notes = ?2 WHERE id = ?1", params![id, notes.trim()])?;
        Ok(())
    }

    /// Mark or unmark every one of `ids` as a favourite, in one write.
    pub fn set_favorites(&self, ids: &[i64], favorite: bool) -> Result<()> {
        let all = self.db.unchecked_transaction()?;
        for id in ids {
            self.set_favorite(*id, favorite)?;
        }
        Ok(all.commit()?)
    }

    /// Put clips in the trash as far as the library goes: they are not
    /// listed any more, and their files stay where they are until
    /// `in_trash` is acted on. `restore` before then is the undo.
    pub fn trash(&self, ids: &[i64], now: i64) -> Result<()> {
        let all = self.db.unchecked_transaction()?;
        for id in ids {
            self.db.execute("UPDATE clips SET trashed_at = ?2 WHERE id = ?1", params![id, now])?;
        }
        Ok(all.commit()?)
    }

    /// Bring back every clip `trash` took out that still has its row.
    /// Returns how many came back.
    pub fn restore(&self) -> Result<u32> {
        Ok(self.db.execute("UPDATE clips SET trashed_at = NULL WHERE trashed_at IS NOT NULL", [])? as u32)
    }

    /// Bring back only `ids`: the ones whose files could not be moved to
    /// the trash after all.
    pub fn restore_some(&self, ids: &[i64]) -> Result<()> {
        let all = self.db.unchecked_transaction()?;
        for id in ids {
            self.db.execute("UPDATE clips SET trashed_at = NULL WHERE id = ?1", [id])?;
        }
        Ok(all.commit()?)
    }

    /// The clips `trash` took out, whose files are still to be moved.
    pub fn in_trash(&self) -> Result<Vec<Clip>> {
        let mut stmt = self.db.prepare(&format!("SELECT {COLUMNS} FROM clips WHERE trashed_at IS NOT NULL ORDER BY id"))?;
        let rows = stmt.query_map([], clip)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Give a clip a name and its file the same one, so the folder reads
    /// as the library does. The clip stays the clip it was: its favourite,
    /// tags, notes and edit go with it. Returns where the file is now.
    ///
    /// A name with nothing in it goes back to the file's, and the file is
    /// left alone. When another file has the name already, a number is
    /// added to this one.
    pub fn rename_file(&self, id: i64, title: &str) -> Result<PathBuf> {
        let clip = self.get(id)?.ok_or(LibraryError::Missing)?;
        let Some(stem) = super::file_stem_for(title) else {
            self.set_title(id, "")?;
            return Ok(clip.path);
        };
        let ext = clip.path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let wanted = clip.path.with_file_name(format!("{stem}{ext}"));
        let target = if wanted == clip.path { wanted } else { self.move_file(id, &clip.path, &wanted)? };
        // The file's name is the clip's name when no other is kept: one is
        // only kept when the file could not be called exactly that
        // (a `:` in it, or a number added).
        let shown = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.set_title(id, if shown == title.trim() { "" } else { title })?;
        Ok(target)
    }

    /// Move a clip to another game: its file goes to that game's folder in
    /// the clip folder `dir`. Returns where the file is now.
    pub fn move_to_game(&self, id: i64, dir: &Path, game: &str) -> Result<PathBuf> {
        let clip = self.get(id)?.ok_or(LibraryError::Missing)?;
        let game = crate::games::folder_name(game);
        let Some(name) = clip.path.file_name() else { return Err(LibraryError::Missing) };
        let wanted = dir.join(&game).join(name);
        let target = if wanted == clip.path { wanted } else { self.move_file(id, &clip.path, &wanted)? };
        self.db.execute("UPDATE clips SET game = ?2 WHERE id = ?1", params![id, game])?;
        Ok(target)
    }

    /// Move a clip's file to `wanted` (or that name with a number, when it
    /// is taken) and point the clip at it.
    fn move_file(&self, id: i64, from: &Path, wanted: &Path) -> Result<PathBuf> {
        if let Some(folder) = wanted.parent() {
            std::fs::create_dir_all(folder)?;
        }
        // The same file under a name that differs only in its capitals is
        // not a name that is taken, where capitals do not count.
        // Where capitals do count, that name may be another clip's file,
        // and a rename onto it would replace it.
        let same = wanted.parent() == from.parent()
            && wanted.file_name().zip(from.file_name()).is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
            && (!wanted.exists() || same_file(from, wanted));
        let target = if same { wanted.to_path_buf() } else { super::free_path(wanted) };
        std::fs::rename(from, &target)?;
        if let Err(e) = self.db.execute("UPDATE clips SET path = ?2 WHERE id = ?1", params![id, target.to_string_lossy()]) {
            // Better back where it was than a file the index does not know.
            let _ = std::fs::rename(&target, from);
            return Err(e.into());
        }
        Ok(target)
    }

    /// Take a clip out of the index. Its file is the caller's to deal with.
    pub fn forget(&self, id: i64) -> Result<()> {
        self.db.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Bring the index in line with the clip folder: files that are gone
    /// leave it, files that are new join it, and a file that was renamed
    /// or moved stays the clip it was. `probe` reads a file; one it
    /// refuses (still being written, not a video) is left for next time.
    ///
    /// A folder that cannot be read (a drive that is not plugged in) says
    /// nothing about its clips, so nothing is changed then: the index is
    /// the only place a clip's name, favourite and edit are kept. For the
    /// same reason a clip is only forgotten when the place its file was in
    /// can be looked into and the file is not there (see `says_gone`).
    ///
    /// The index is read, then the files are looked at with nothing held,
    /// then what was found is written in one short go. Reading a new file
    /// takes a moment each (`probe` runs ffprobe), and the first look at a
    /// large folder takes minutes of them: the recorder must be able to add
    /// the clip it has just saved all through that.
    pub fn rescan(&self, dir: &Path, probe: impl Fn(&Path) -> std::result::Result<Meta, ProbeError>) -> Result<Changes> {
        let Ok(mut entries) = std::fs::read_dir(dir) else {
            return Ok(Changes::default());
        };
        // A clip folder with nothing in it at all is what is left where a
        // drive was mounted: someone who deleted their clips has the
        // games' folders still.
        let bare = entries.next().is_none();
        let on_disk = super::files(dir);
        let known: Vec<(i64, PathBuf, u64)> = {
            let mut stmt = self.db.prepare("SELECT id, path, bytes FROM clips")?;
            stmt.query_map([], |r| Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?), r.get::<_, i64>(2)? as u64)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let indexed: std::collections::HashSet<&Path> = known.iter().map(|(_, path, _)| path.as_path()).collect();
        // Clips whose file is not where it was, until a new file claims one.
        let mut gone: Vec<(i64, &Path, u64)> = known.iter().filter(|(_, path, _)| !path.is_file()).map(|(id, path, bytes)| (*id, path.as_path(), *bytes)).collect();
        // What there is to write: clips whose file has a new place (from,
        // to), and files that are new.
        let mut moved: Vec<(i64, &Path, &Path)> = Vec::new();
        let mut found: Vec<(&Path, Meta)> = Vec::new();
        for file in on_disk.iter().filter(|file| !indexed.contains(file.as_path())) {
            // The same number of bytes under a new name is the file that went missing.
            let stamp = std::fs::metadata(file).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
            let size = stamp.0;
            if let Some(at) = gone.iter().position(|(_, _, bytes)| size > 0 && *bytes == size) {
                let (id, from, _) = gone.remove(at);
                moved.push((id, from, file));
                continue;
            }
            // Refused before and not changed since: it would be refused again.
            if self.refused.borrow().get(file) == Some(&stamp) {
                continue;
            }
            let Ok(meta) = probe(file) else {
                self.refused.borrow_mut().insert(file.clone(), stamp);
                continue;
            };
            self.refused.borrow_mut().remove(file);
            found.push((file, meta));
        }
        let forgotten: Vec<(i64, &Path)> = gone.into_iter().filter(|(_, path, _)| says_gone(dir, bare, path)).map(|(id, path, _)| (id, path)).collect();
        let mut changes = Changes::default();
        if moved.is_empty() && found.is_empty() && forgotten.is_empty() {
            // Nothing to write: the index is not locked at all.
            return Ok(changes);
        }
        // One write to the index, however many clips changed. It is taken
        // as a write from its start, so it waits its turn behind another
        // writer instead of failing halfway. Every row is looked at again
        // as it is written: the recorder or another window may have changed
        // it since it was read above.
        let all = rusqlite::Transaction::new_unchecked(&self.db, rusqlite::TransactionBehavior::Immediate)?;
        for (id, from, to) in moved {
            // Only a clip that is still where it was read to be, and only
            // to a name no clip has taken meanwhile.
            let done = self.db.execute(
                "UPDATE OR IGNORE clips SET path = ?3, game = ?4 WHERE id = ?1 AND path = ?2",
                params![id, from.to_string_lossy(), to.to_string_lossy(), super::game_of(dir, to)],
            )?;
            changes.moved += done as u32;
        }
        for (file, meta) in found {
            // Added meanwhile by the recorder, which knows more about it
            // (its tracks, when it was saved) than can be read from the file.
            let there: bool = self.db.query_row("SELECT EXISTS (SELECT 1 FROM clips WHERE path = ?1)", [file.to_string_lossy()], |r| r.get(0))?;
            if there {
                continue;
            }
            self.add_found(file, &super::game_of(dir, file), super::saved_at(file), &meta)?;
            changes.added += 1;
        }
        for (id, path) in forgotten {
            // A file that is back, or a clip that points somewhere else
            // now, is not forgotten.
            if path.is_file() {
                continue;
            }
            changes.removed += self.db.execute("DELETE FROM clips WHERE id = ?1 AND path = ?2", params![id, path.to_string_lossy()])? as u32;
        }
        all.commit()?;
        Ok(changes)
    }
}

/// Whether a clip's file not being at `path` means the clip is gone, and
/// not only out of reach. `dir` is the clip folder, `bare` whether it has
/// nothing in it at all.
fn says_gone(dir: &Path, bare: bool, path: &Path) -> bool {
    let Some(folder) = path.parent() else { return false };
    if path.starts_with(dir) {
        // An empty clip folder is a drive that is not there. A folder that
        // is there but cannot be looked into (not allowed) says nothing; one
        // that is not there any more was deleted with its clips.
        !bare && (!folder.exists() || std::fs::read_dir(folder).is_ok())
    } else {
        // A clip from another clip folder, kept from before: gone when the
        // folder it was in is still there without it. That folder missing
        // too is a drive that is not plugged in.
        std::fs::read_dir(folder).is_ok()
    }
}

/// Whether two names are the same file (where capitals do not count, a
/// file answers to its name in any capitals).
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(seconds: f64) -> Meta {
        Meta { seconds, width: 2560, height: 1440, fps: 60.0, codec: "h264".into(), audio_tracks: 3, bytes: 1000 }
    }

    fn library() -> Library {
        let lib = Library::in_memory().unwrap();
        let all = [Track::Game, Track::Chat, Track::Microphone];
        lib.add(Path::new("/c/CS2/CS2 2026-11-01 10-00-00.mp4"), "CS2", 100, &meta(30.0), &all).unwrap();
        lib.add(Path::new("/c/CS2/CS2 2026-11-02 10-00-00.mp4"), "CS2", 200, &meta(60.0), &all).unwrap();
        lib.add(Path::new("/c/Dota 2/Dota 2 2026-11-03 10-00-00.mp4"), "Dota 2", 300, &meta(45.0), &[Track::Game]).unwrap();
        lib
    }

    fn titles(clips: &[Clip]) -> Vec<&str> {
        clips.iter().map(|c| c.title.as_str()).collect()
    }

    #[test]
    fn clips_list_by_time_game_and_words() {
        let lib = library();
        let all = lib.list(&Filter::default()).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].game, "Dota 2", "newest first");
        assert_eq!(all[0].tracks, [Track::Game]);
        let oldest = lib.list(&Filter { order: Order::Oldest, ..Default::default() }).unwrap();
        assert_eq!(oldest[0].saved_at, 100);
        let cs = lib.list(&Filter { game: Some("CS2".into()), ..Default::default() }).unwrap();
        assert_eq!(cs.len(), 2);
        let found = lib.list(&Filter { search: "dota 11-03".into(), ..Default::default() }).unwrap();
        assert_eq!(titles(&found), ["Dota 2 2026-11-03 10-00-00"]);
        assert!(lib.list(&Filter { search: "100%".into(), ..Default::default() }).unwrap().is_empty());
        // The folders a clip is in, and its file type, are not its name.
        for path_only in ["/c", "mp4"] {
            assert!(lib.list(&Filter { search: path_only.into(), ..Default::default() }).unwrap().is_empty(), "{path_only}");
        }
        let id = all[0].id;
        lib.set_title(id, "Rampage").unwrap();
        assert_eq!(lib.list(&Filter { search: "RAMP".into(), ..Default::default() }).unwrap().len(), 1);
    }

    #[test]
    fn known_tracks_replace_a_guess_and_a_guess_never_replaces_them() {
        let lib = Library::in_memory().unwrap();
        let path = Path::new("/c/CS2/a.mp4");
        let two = Meta { audio_tracks: 2, ..meta(30.0) };
        let id = lib.add_found(path, "CS2", 100, &two).unwrap();
        assert_eq!(lib.get(id).unwrap().unwrap().tracks, [Track::Game, Track::Microphone]);
        // The recorder says what they really are.
        lib.add(path, "CS2", 100, &two, &[Track::Game, Track::Chat]).unwrap();
        assert_eq!(lib.get(id).unwrap().unwrap().tracks, [Track::Game, Track::Chat]);
        lib.add_found(path, "CS2", 100, &two).unwrap();
        assert_eq!(lib.get(id).unwrap().unwrap().tracks, [Track::Game, Track::Chat]);
    }

    #[test]
    fn a_refused_file_is_not_read_again_until_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let broken = d.join("broken.mp4");
        std::fs::write(&broken, b"").unwrap();
        let reads = std::cell::Cell::new(0);
        let probe = |p: &Path| {
            reads.set(reads.get() + 1);
            if std::fs::metadata(p).unwrap().len() == 0 { Err(ProbeError::Invalid("empty".into())) } else { Ok(meta(12.0)) }
        };
        let lib = Library::open(&d.join("index/library.db")).unwrap();
        for _ in 0..3 {
            assert_eq!(lib.rescan(d, probe).unwrap(), Changes::default());
        }
        assert_eq!(reads.get(), 1);
        std::fs::write(&broken, b"now a video").unwrap();
        assert_eq!(lib.rescan(d, probe).unwrap(), Changes { added: 1, ..Default::default() });
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn the_index_says_which_layout_it_has() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("library.db");
        drop(Library::open(&file).unwrap());
        let lib = Library::open(&file).unwrap();
        let version: i64 = lib.db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    /// An index as Zenith 0.5 left it: version 1's table, with what a user
    /// had set on some of its clips.
    fn version_1_index(file: &Path) -> Edit {
        let db = Connection::open(file).unwrap();
        db.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.execute_batch(SCHEMA_V1).unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        let mut edit = Edit::new(30.5, 2);
        edit.split(10.0);
        let tracks = serde_json::to_string(&[Track::Game, Track::Chat]).unwrap();
        for n in 1..=40i64 {
            let named = (n % 4 == 0).then(|| format!("Named {n}"));
            let edited = (n % 5 == 0).then(|| serde_json::to_string(&edit).unwrap());
            db.execute(
                "INSERT INTO clips (path, game, title, saved_at, seconds, width, height, fps, bytes, favorite, tracks, edit)
                 VALUES (?1, 'CS2', ?2, ?3, 30.5, 2560, 1440, 60.0, ?4, ?5, ?6, ?7)",
                params![format!("/c/CS2/{n}.mp4"), named, n * 100, n * 1000, n % 3 == 0, tracks, edited],
            )
            .unwrap();
        }
        edit
    }

    #[test]
    fn an_index_from_version_1_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("library.db");
        let edit = version_1_index(&file);
        let lib = Library::open(&file).unwrap();
        let version: i64 = lib.db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let clips = lib.list(&Filter { order: Order::Oldest, ..Default::default() }).unwrap();
        assert_eq!(clips.len(), 40, "every row");
        for (clip, n) in clips.iter().zip(1..=40i64) {
            assert_eq!(clip.path, PathBuf::from(format!("/c/CS2/{n}.mp4")));
            assert_eq!(clip.title, if n % 4 == 0 { format!("Named {n}") } else { n.to_string() }, "titles");
            assert_eq!(clip.favorite, n % 3 == 0, "favourites");
            assert_eq!(clip.edit, (n % 5 == 0).then(|| edit.clone()), "edits");
            assert_eq!((clip.saved_at, clip.seconds, clip.bytes, clip.width, clip.fps), (n * 100, 30.5, n as u64 * 1000, 2560, 60.0));
            assert_eq!(clip.tracks, [Track::Game, Track::Chat]);
            assert_eq!((clip.tags.len(), clip.notes.as_str()), (0, ""), "what is new starts empty");
        }
        // What version 2 added can be used on a row that came from version 1.
        lib.set_tags(clips[0].id, &["ace".into()]).unwrap();
        lib.set_notes(clips[0].id, "the first one").unwrap();
        lib.trash(&[clips[1].id], 5).unwrap();
        drop(lib);
        // Opened again, nothing is moved a second time and nothing is lost.
        let lib = Library::open(&file).unwrap();
        let again = lib.list(&Filter { order: Order::Oldest, ..Default::default() }).unwrap();
        assert_eq!(again.len(), 39);
        assert_eq!((again[0].tags.as_slice(), again[0].notes.as_str()), (&["ace".to_string()][..], "the first one"));
        assert_eq!(lib.restore().unwrap(), 1);
    }

    #[test]
    fn a_copy_of_the_older_index_is_kept_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("library.db");
        version_1_index(&file);
        drop(Library::open(&file).unwrap());
        let copy = Connection::open(dir.path().join("library.v1.db")).unwrap();
        let (rows, version): (i64, i64) =
            (copy.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0)).unwrap(), copy.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap());
        assert_eq!((rows, version), (40, 1));
        assert!(copy.prepare("SELECT tags FROM clips").is_err(), "the copy is the index as it was");
        // A new index has nothing to keep a copy of.
        let fresh = tempfile::tempdir().unwrap();
        drop(Library::open(&fresh.path().join("library.db")).unwrap());
        drop(Library::open(&fresh.path().join("library.db")).unwrap());
        assert_eq!(std::fs::read_dir(fresh.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains(".v")).count(), 0);
    }

    #[test]
    fn an_index_from_before_the_layouts_were_numbered_is_version_1() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("library.db");
        version_1_index(&file);
        Connection::open(&file).unwrap().pragma_update(None, "user_version", 0).unwrap();
        let lib = Library::open(&file).unwrap();
        assert_eq!(lib.list(&Filter::default()).unwrap().len(), 40);
        assert!(lib.list(&Filter::default()).unwrap().iter().all(|c| c.tags.is_empty()));
    }

    #[test]
    fn an_older_zenith_can_still_add_to_a_newer_index() {
        // The recorder of the version before, still running after an update:
        // it writes the columns it knows.
        let lib = Library::in_memory().unwrap();
        lib.db
            .execute(
                "INSERT INTO clips (path, game, saved_at, seconds, width, height, fps, bytes, tracks) VALUES ('/c/a.mp4', 'CS2', 1, 2.0, 3, 4, 5.0, 6, '[]')",
                [],
            )
            .unwrap();
        let clip = &lib.list(&Filter::default()).unwrap()[0];
        assert_eq!((clip.tags.len(), clip.notes.as_str()), (0, ""));
    }

    #[test]
    fn tags_are_tidied_and_found_by_search() {
        assert_eq!(tags_from(" Ace, #Clutch ,ace,, 1v5   retake #x"), ["ace", "clutch", "1v5 retake", "x"]);
        assert!(tags_from(" , # ").is_empty());
        let lib = library();
        let all = lib.list(&Filter { order: Order::Oldest, ..Default::default() }).unwrap();
        let (first, second, dota) = (all[0].id, all[1].id, all[2].id);
        lib.set_tags(first, &["Ace".into(), "clutch".into()]).unwrap();
        lib.add_tags(&[first, second, dota], &["Funny".into(), "ace".into()]).unwrap();
        lib.set_notes(dota, "  Roshan steal with the whole team watching ").unwrap();
        assert_eq!(lib.get(first).unwrap().unwrap().tags, ["ace", "clutch", "funny"]);
        assert_eq!(lib.get(dota).unwrap().unwrap().notes, "Roshan steal with the whole team watching");

        let found = |search: &str| lib.list(&Filter { search: search.into(), order: Order::Oldest, ..Default::default() }).unwrap();
        let ids = |clips: Vec<Clip>| clips.iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(found("clutch")), [first]);
        assert_eq!(ids(found("FUNNY cs2")), [first, second], "a tag and the game together");
        assert_eq!(ids(found("roshan")), [dota], "words in the notes");
        assert_eq!(ids(found("#clu")), [first], "a # looks among the tags only");
        assert!(found("#roshan").is_empty());
        assert!(found("#").is_empty());
        assert_eq!(ids(lib.list(&Filter { tag: Some("clutch".into()), ..Default::default() }).unwrap()), [first]);
        assert_eq!(lib.list(&Filter { tag: Some("clu".into()), ..Default::default() }).unwrap().len(), 0, "a whole tag, not a part");

        assert_eq!(
            lib.tags().unwrap(),
            [TagCount { tag: "ace".into(), clips: 3 }, TagCount { tag: "funny".into(), clips: 3 }, TagCount { tag: "clutch".into(), clips: 1 }]
        );
        lib.set_tags(first, &[]).unwrap();
        assert_eq!(lib.tags().unwrap().len(), 2);
    }

    #[test]
    fn clips_sort_by_date_length_and_size() {
        let lib = Library::in_memory().unwrap();
        // (name, saved at, seconds, bytes)
        for (name, at, seconds, bytes) in [("a", 100, 30.0, 500u64), ("b", 200, 10.0, 900), ("c", 300, 30.0, 100), ("d", 400, 20.0, 900)] {
            lib.add(Path::new(&format!("/c/{name}.mp4")), "CS2", at, &Meta { bytes, ..meta(seconds) }, &[]).unwrap();
        }
        let order = |order: Order| lib.list(&Filter { order, ..Default::default() }).unwrap().iter().map(|c| c.title.clone()).collect::<Vec<_>>().join("");
        assert_eq!(order(Order::Newest), "dcba");
        assert_eq!(order(Order::Oldest), "abcd");
        assert_eq!(order(Order::Longest), "cadb", "as long as each other: the newer first");
        assert_eq!(order(Order::Shortest), "bdca");
        assert_eq!(order(Order::Largest), "dbac");
        assert_eq!(order(Order::Smallest), "cadb");
        for (name, order) in [("oldest", Order::Oldest), ("longest", Order::Longest), ("shortest", Order::Shortest), ("largest", Order::Largest), ("smallest", Order::Smallest), ("newest", Order::Newest), ("", Order::Newest)] {
            assert_eq!(Order::named(name), order);
        }
    }

    #[test]
    fn a_trashed_clip_is_out_of_sight_until_it_is_restored() {
        let lib = library();
        let all = lib.list(&Filter::default()).unwrap();
        lib.set_tags(all[0].id, &["only".into()]).unwrap();
        lib.trash(&[all[0].id, all[1].id], 1234).unwrap();
        assert_eq!(lib.list(&Filter::default()).unwrap().len(), 1);
        assert_eq!(lib.shelves().unwrap(), [Shelf { game: "CS2".into(), clips: 1, latest: 100 }]);
        assert!(lib.tags().unwrap().is_empty());
        assert_eq!(lib.in_trash().unwrap().len(), 2);
        // One could not be moved after all: it alone comes back.
        lib.restore_some(&[all[1].id]).unwrap();
        assert_eq!(lib.list(&Filter::default()).unwrap().len(), 2);
        assert_eq!(lib.restore().unwrap(), 1);
        assert_eq!(lib.restore().unwrap(), 0);
        let back = lib.get(all[0].id).unwrap().unwrap();
        assert_eq!(back.tags, ["only"], "with what it had");
        assert!(lib.in_trash().unwrap().is_empty());
    }

    /// A clip folder with one Hades clip that has a favourite, a tag, notes
    /// and an edit.
    fn one_clip(d: &Path) -> (Library, i64, Edit) {
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        std::fs::write(d.join("Hades/Hades 2026-11-20 21-04-33.mp4"), b"the clip").unwrap();
        let lib = Library::open(&d.join("index/library.db")).unwrap();
        lib.rescan(d, real_size).unwrap();
        let id = lib.list(&Filter::default()).unwrap()[0].id;
        let mut edit = Edit::new(12.0, 3);
        edit.split(4.0);
        lib.set_favorite(id, true).unwrap();
        lib.set_tags(id, &["boss".into()]).unwrap();
        lib.set_notes(id, "no hits taken").unwrap();
        lib.set_edit(id, Some(&edit)).unwrap();
        (lib, id, edit)
    }

    #[test]
    fn renaming_a_clip_can_rename_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let (lib, id, edit) = one_clip(d);
        let saved = lib.get(id).unwrap().unwrap().saved_at;

        let now = lib.rename_file(id, "  Flawless Hydra ").unwrap();
        assert_eq!(now, d.join("Hades/Flawless Hydra.mp4"));
        assert!(now.is_file() && !d.join("Hades/Hades 2026-11-20 21-04-33.mp4").exists());
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!((clip.title.as_str(), clip.path.as_path(), clip.game.as_str()), ("Flawless Hydra", now.as_path(), "Hades"));
        assert_eq!((clip.favorite, clip.tags.as_slice(), clip.notes.as_str(), clip.edit.as_ref()), (true, &["boss".to_string()][..], "no hits taken", Some(&edit)));
        assert_eq!(clip.saved_at, saved, "it keeps its day though the name no longer says it");
        // The folder agrees: a rescan has nothing to do, and it is still one clip.
        assert_eq!(lib.rescan(d, real_size).unwrap(), Changes::default());
        assert_eq!(lib.list(&Filter::default()).unwrap().len(), 1);

        // What a file cannot be called is kept as the name shown.
        let now = lib.rename_file(id, "Ace: 1v5?").unwrap();
        assert_eq!(now, d.join("Hades/Ace 1v5.mp4"));
        assert_eq!(lib.get(id).unwrap().unwrap().title, "Ace: 1v5?");
        // The same name again moves nothing.
        assert_eq!(lib.rename_file(id, "Ace: 1v5?").unwrap(), now);
        // A name another file has: this one gets a number, and keeps the name.
        std::fs::write(d.join("Hades/Taken.mp4"), b"another").unwrap();
        let now = lib.rename_file(id, "Taken").unwrap();
        assert_eq!(now, d.join("Hades/Taken (2).mp4"));
        assert_eq!(lib.get(id).unwrap().unwrap().title, "Taken");
        assert_eq!(std::fs::read(d.join("Hades/Taken.mp4")).unwrap(), b"another");
        // Only the capitals changed: that is this file, not a taken name.
        assert_eq!(lib.rename_file(id, "taken (2)").unwrap(), d.join("Hades/taken (2).mp4"));
        // No name: back to the file's, which stays.
        assert_eq!(lib.rename_file(id, "  ").unwrap(), d.join("Hades/taken (2).mp4"));
        assert_eq!(lib.get(id).unwrap().unwrap().title, "taken (2)");
        assert!(matches!(lib.rename_file(9999, "x"), Err(LibraryError::Missing)));
    }

    #[test]
    fn a_file_that_cannot_be_renamed_leaves_the_clip_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let (lib, id, _) = one_clip(d);
        std::fs::remove_file(d.join("Hades/Hades 2026-11-20 21-04-33.mp4")).unwrap();
        assert!(matches!(lib.rename_file(id, "Gone"), Err(LibraryError::Io(_))));
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!((clip.title.as_str(), clip.path), ("Hades 2026-11-20 21-04-33", d.join("Hades/Hades 2026-11-20 21-04-33.mp4")));
    }

    #[test]
    fn a_clip_moves_to_another_game_with_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let (lib, id, edit) = one_clip(d);
        let now = lib.move_to_game(id, d, "Hades: II™").unwrap();
        assert_eq!(now, d.join("Hades II/Hades 2026-11-20 21-04-33.mp4"));
        assert!(now.is_file());
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!((clip.game.as_str(), clip.favorite, clip.edit.as_ref()), ("Hades II", true, Some(&edit)));
        assert_eq!(lib.rescan(d, real_size).unwrap(), Changes::default(), "the folder says the same");
        assert_eq!(lib.shelves().unwrap()[0].game, "Hades II");
        // To the game it is in already: nothing moves.
        assert_eq!(lib.move_to_game(id, d, "Hades II").unwrap(), now);
        // From a folder within a folder, it lands directly under the game.
        std::fs::create_dir_all(d.join("Hades II/2026/best")).unwrap();
        std::fs::rename(&now, d.join("Hades II/2026/best/run.mp4")).unwrap();
        lib.rescan(d, real_size).unwrap();
        assert_eq!(lib.get(id).unwrap().unwrap().game, "Hades II");
        assert_eq!(lib.move_to_game(id, d, "Best of").unwrap(), d.join("Best of/run.mp4"));
    }

    #[test]
    fn clips_in_folders_within_folders_are_indexed_under_their_game() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Hades/2026/November")).unwrap();
        std::fs::write(d.join("Hades/2026/November/run.mp4"), b"deep").unwrap();
        std::fs::write(d.join("Hades/top.mp4"), b"shallow one").unwrap();
        let lib = Library::open(&d.join("index/library.db")).unwrap();
        assert_eq!(lib.rescan(d, real_size).unwrap(), Changes { added: 2, ..Default::default() });
        assert_eq!(lib.shelves().unwrap(), [Shelf { game: "Hades".into(), clips: 2, latest: lib.shelves().unwrap()[0].latest }]);
        // Sorted into a deeper folder by hand: still the same clip.
        let id = lib.list(&Filter { search: "top".into(), ..Default::default() }).unwrap()[0].id;
        lib.set_title(id, "Kept").unwrap();
        std::fs::rename(d.join("Hades/top.mp4"), d.join("Hades/2026/top.mp4")).unwrap();
        assert_eq!(lib.rescan(d, real_size).unwrap(), Changes { moved: 1, ..Default::default() });
        assert_eq!(lib.get(id).unwrap().unwrap().title, "Kept");
    }

    #[test]
    fn shelves_count_clips_by_game() {
        let shelves = library().shelves().unwrap();
        assert_eq!(shelves[0], Shelf { game: "Dota 2".into(), clips: 1, latest: 300 });
        assert_eq!(shelves[1], Shelf { game: "CS2".into(), clips: 2, latest: 200 });
    }

    #[test]
    fn what_the_user_set_survives_a_second_add() {
        let lib = library();
        let id = lib.list(&Filter::default()).unwrap()[0].id;
        lib.set_favorite(id, true).unwrap();
        lib.set_title(id, "  Rampage  ").unwrap();
        let mut edit = Edit::new(45.0, 1);
        edit.split(10.0);
        lib.set_edit(id, Some(&edit)).unwrap();
        let again = lib.add(Path::new("/c/Dota 2/Dota 2 2026-11-03 10-00-00.mp4"), "Dota 2", 999, &meta(45.25), &[]).unwrap();
        assert_eq!(again, id);
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!((clip.title.as_str(), clip.favorite, clip.seconds, clip.saved_at), ("Rampage", true, 45.25, 300));
        assert_eq!(clip.edit, Some(edit));
        assert_eq!(lib.list(&Filter { favorites: true, ..Default::default() }).unwrap().len(), 1);
        lib.set_title(id, " ").unwrap();
        lib.set_edit(id, Some(&Edit::new(45.0, 1))).unwrap();
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!(clip.title, "Dota 2 2026-11-03 10-00-00");
        assert_eq!(clip.edit, Some(Edit::new(45.0, 1)), "a clip put back as recorded keeps that, whatever the settings start one at");
        lib.set_edit(id, None).unwrap();
        assert_eq!(lib.get(id).unwrap().unwrap().edit, None);
        lib.forget(id).unwrap();
        assert_eq!(lib.get(id).unwrap(), None);
    }

    #[test]
    fn a_rescan_follows_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        let a = d.join("Hades/Hades 2026-11-20 21-04-33.mp4");
        let b = d.join("Hades/broken.mp4");
        std::fs::write(&a, b"video").unwrap();
        std::fs::write(&b, b"").unwrap();
        let probe = |p: &Path| {
            if p.ends_with("broken.mp4") { Err(ProbeError::Invalid("empty".into())) } else { Ok(meta(12.0)) }
        };
        let lib = Library::open(&d.join("index/library.db")).unwrap();
        assert_eq!(lib.rescan(d, probe).unwrap(), Changes { added: 1, ..Default::default() });
        assert_eq!(lib.rescan(d, probe).unwrap(), Changes::default(), "nothing new the second time");
        let clip = &lib.list(&Filter::default()).unwrap()[0];
        assert_eq!((clip.game.as_str(), clip.seconds), ("Hades", 12.0));
        assert_eq!(clip.tracks, [Track::Game, Track::Chat, Track::Microphone]);
        std::fs::remove_file(&a).unwrap();
        assert_eq!(lib.rescan(d, probe).unwrap(), Changes { removed: 1, ..Default::default() });
        assert!(lib.list(&Filter::default()).unwrap().is_empty());
    }

    fn real_size(p: &Path) -> std::result::Result<Meta, ProbeError> {
        Ok(Meta { bytes: std::fs::metadata(p).unwrap().len(), ..meta(12.0) })
    }

    #[test]
    fn a_folder_that_cannot_be_read_forgets_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("Clips");
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        std::fs::write(d.join("Hades/a.mp4"), b"video").unwrap();
        let lib = Library::open(&dir.path().join("library.db")).unwrap();
        lib.rescan(&d, real_size).unwrap();
        let id = lib.list(&Filter::default()).unwrap()[0].id;
        lib.set_title(id, "Kept").unwrap();
        // The drive is unplugged: the folder is not there at all.
        let away = dir.path().join("away");
        std::fs::rename(&d, &away).unwrap();
        assert_eq!(lib.rescan(&d, real_size).unwrap(), Changes::default());
        std::fs::rename(&away, &d).unwrap();
        assert_eq!(lib.rescan(&d, real_size).unwrap(), Changes::default());
        assert_eq!(lib.get(id).unwrap().unwrap().title, "Kept");
    }

    #[test]
    fn an_empty_clip_folder_is_a_drive_that_is_not_there() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("Clips");
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        std::fs::write(d.join("Hades/a.mp4"), b"video").unwrap();
        std::fs::write(d.join("Hades/b.mp4"), b"another").unwrap();
        let lib = Library::open(&dir.path().join("library.db")).unwrap();
        lib.rescan(&d, real_size).unwrap();
        let id = lib.list(&Filter::default()).unwrap()[0].id;
        lib.set_title(id, "Kept").unwrap();
        // Unmounted: the folder it was mounted on is there, with nothing in it.
        let away = dir.path().join("away");
        std::fs::rename(&d, &away).unwrap();
        std::fs::create_dir(&d).unwrap();
        assert_eq!(lib.rescan(&d, real_size).unwrap(), Changes::default());
        assert_eq!(lib.list(&Filter::default()).unwrap().len(), 2);
        std::fs::remove_dir(&d).unwrap();
        std::fs::rename(&away, &d).unwrap();
        assert_eq!(lib.rescan(&d, real_size).unwrap(), Changes::default());
        assert_eq!(lib.get(id).unwrap().unwrap().title, "Kept");
        // Deleted by hand, the game's folder still there: those are gone.
        std::fs::remove_file(d.join("Hades/a.mp4")).unwrap();
        std::fs::remove_file(d.join("Hades/b.mp4")).unwrap();
        assert_eq!(lib.rescan(&d, real_size).unwrap().removed, 2);
    }

    #[test]
    fn a_clip_from_another_clip_folder_is_kept_while_that_folder_is_away() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("Old"), dir.path().join("New"));
        std::fs::create_dir_all(old.join("Hades")).unwrap();
        std::fs::create_dir_all(new.join("Celeste")).unwrap();
        std::fs::write(old.join("Hades/a.mp4"), b"from before").unwrap();
        std::fs::write(new.join("Celeste/b.mp4"), b"new").unwrap();
        let lib = Library::open(&dir.path().join("library.db")).unwrap();
        lib.rescan(&old, real_size).unwrap();
        lib.rescan(&new, real_size).unwrap();
        assert_eq!(lib.list(&Filter::default()).unwrap().len(), 2);
        // The old folder's drive is unplugged.
        let away = dir.path().join("away");
        std::fs::rename(&old, &away).unwrap();
        assert_eq!(lib.rescan(&new, real_size).unwrap(), Changes::default());
        // Back, and the clip deleted from it: now it is gone.
        std::fs::rename(&away, &old).unwrap();
        std::fs::remove_file(old.join("Hades/a.mp4")).unwrap();
        assert_eq!(lib.rescan(&new, real_size).unwrap().removed, 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_may_not_be_looked_into_forgets_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        std::fs::create_dir_all(d.join("Celeste")).unwrap();
        std::fs::write(d.join("Hades/a.mp4"), b"video").unwrap();
        std::fs::write(d.join("Celeste/b.mp4"), b"another").unwrap();
        let lib = Library::open(&d.join("library.db")).unwrap();
        lib.rescan(d, real_size).unwrap();
        let shut = |mode| std::fs::set_permissions(d.join("Hades"), std::fs::Permissions::from_mode(mode)).unwrap();
        shut(0o000);
        // Whoever may read everything (root) is not shut out by this.
        if std::fs::read_dir(d.join("Hades")).is_err() {
            assert_eq!(lib.rescan(d, real_size).unwrap(), Changes::default());
        }
        shut(0o755);
        assert_eq!(lib.list(&Filter::default()).unwrap().len(), 2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_new_name_in_other_capitals_does_not_replace_another_clip() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        std::fs::write(d.join("Hades/Ace.mp4"), b"the first").unwrap();
        std::fs::write(d.join("Hades/ace.mp4"), b"the second one").unwrap();
        let lib = Library::open(&d.join("index/library.db")).unwrap();
        lib.rescan(d, real_size).unwrap();
        let first = lib.list(&Filter::default()).unwrap().into_iter().find(|c| c.path.ends_with("Ace.mp4")).unwrap().id;
        let now = lib.rename_file(first, "ace").unwrap();
        assert_eq!(now, d.join("Hades/ace (2).mp4"));
        assert_eq!(std::fs::read(&now).unwrap(), b"the first");
        assert_eq!(std::fs::read(d.join("Hades/ace.mp4")).unwrap(), b"the second one");
        // Its own name in other capitals is still just a rename.
        assert_eq!(lib.rename_file(first, "ACE (2)").unwrap(), d.join("Hades/ACE (2).mp4"));
    }

    #[test]
    fn a_moved_file_is_still_the_same_clip() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        std::fs::create_dir_all(d.join("Best of")).unwrap();
        std::fs::write(d.join("Hades/a.mp4"), b"the one that moves").unwrap();
        std::fs::write(d.join("Hades/b.mp4"), b"stays").unwrap();
        let lib = Library::open(&d.join("index/library.db")).unwrap();
        lib.rescan(d, real_size).unwrap();
        let id = lib.list(&Filter::default()).unwrap().iter().find(|c| c.path.ends_with("a.mp4")).unwrap().id;
        lib.set_favorite(id, true).unwrap();
        lib.set_title(id, "Ace").unwrap();
        std::fs::rename(d.join("Hades/a.mp4"), d.join("Best of/renamed.mp4")).unwrap();
        assert_eq!(lib.rescan(d, real_size).unwrap(), Changes { moved: 1, ..Default::default() });
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!((clip.title.as_str(), clip.favorite, clip.game.as_str()), ("Ace", true, "Best of"));
        assert_eq!(clip.path, d.join("Best of/renamed.mp4"));
        assert_eq!(lib.list(&Filter::default()).unwrap().len(), 2);
    }

    #[test]
    fn the_recorder_can_add_a_clip_while_a_rescan_reads_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        std::fs::write(d.join("Hades/a.mp4"), b"one").unwrap();
        std::fs::write(d.join("Hades/b.mp4"), b"another").unwrap();
        std::fs::write(d.join("Hades/saved.mp4"), b"just saved").unwrap();
        let index = dir.path().join("index/library.db");
        let lib = Library::open(&index).unwrap();
        // The recorder has the index open too. It does not wait long here,
        // so that being kept out shows as a failure and not as a slow test.
        let recorder = Library::open(&index).unwrap();
        recorder.db.busy_timeout(std::time::Duration::from_millis(100)).unwrap();
        let adds = std::cell::Cell::new(0);
        let probe = |p: &Path| {
            // While each file is read, the recorder files the clip it saved,
            // spelled as the folder's listing spells it (on Windows, with
            // backward slashes throughout).
            recorder.add(&d.join("Hades").join("saved.mp4"), "Hades", 500, &meta(20.0), &[Track::Game]).unwrap();
            adds.set(adds.get() + 1);
            real_size(p)
        };
        assert_eq!(lib.rescan(d, probe).unwrap(), Changes { added: 2, ..Default::default() }, "the one the recorder added is its own");
        assert_eq!(adds.get(), 3);
        let clips = lib.list(&Filter::default()).unwrap();
        assert_eq!(clips.len(), 3);
        let saved = clips.iter().find(|c| c.path.ends_with("saved.mp4")).unwrap();
        assert_eq!((saved.saved_at, saved.seconds, saved.tracks.as_slice()), (500, 20.0, &[Track::Game][..]), "as the recorder knows it");
        assert_eq!(lib.rescan(d, probe).unwrap(), Changes::default());
    }

    #[test]
    fn a_rescan_with_nothing_to_write_and_an_open_wait_for_no_writer() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("Hades")).unwrap();
        std::fs::write(d.join("Hades/a.mp4"), b"one").unwrap();
        let index = dir.path().join("index/library.db");
        let lib = Library::open(&index).unwrap();
        lib.rescan(d, real_size).unwrap();
        // Someone is in the middle of a write to the index.
        let writer = Connection::open(&index).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE; UPDATE clips SET notes = 'being written'").unwrap();
        let started = std::time::Instant::now();
        assert_eq!(lib.rescan(d, real_size).unwrap(), Changes::default());
        let window = Library::open(&index).unwrap();
        assert_eq!(window.list(&Filter::default()).unwrap().len(), 1);
        assert!(started.elapsed() < std::time::Duration::from_secs(3), "neither waited for the lock");
        writer.execute_batch("COMMIT").unwrap();
        assert_eq!(window.list(&Filter::default()).unwrap()[0].notes, "being written");
    }

    #[test]
    fn a_kept_edit_that_does_not_fit_the_file_is_not_handed_out() {
        let lib = library();
        let id = lib.list(&Filter::default()).unwrap()[0].id;
        let kept = |json: &str| {
            lib.db.execute("UPDATE clips SET edit = ?2 WHERE id = ?1", params![id, json]).unwrap();
            lib.get(id).unwrap().unwrap().edit
        };
        // Stretches that would make a trim panic are put right as it is read.
        let mut edit = kept(r#"{"duration":45.0,"segments":[{"start":44.99,"end":45.0},{"start":80.0,"end":20.0}],"gains":[{"volume":1.0,"muted":false}]}"#).unwrap();
        assert_eq!(edit, Edit::new(45.0, 1));
        edit.trim_end(0.0);
        edit.trim_start(45.0);
        // The file was replaced by a shorter one: the edit was of the other.
        let mut long = Edit::new(45.0, 1);
        long.trim_start(40.0);
        lib.set_edit(id, Some(&long)).unwrap();
        assert_eq!(lib.get(id).unwrap().unwrap().edit, Some(long));
        lib.add(Path::new("/c/Dota 2/Dota 2 2026-11-03 10-00-00.mp4"), "Dota 2", 300, &meta(12.0), &[Track::Game]).unwrap();
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!((clip.seconds, clip.edit), (12.0, None));
    }

    #[test]
    fn what_cannot_be_read_of_a_clip_is_kept_aside_before_it_is_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let (lib, id, edit) = one_clip(d);
        let aside = d.join("index/library.unread.jsonl");
        // What reads is simply replaced.
        lib.set_edit(id, Some(&edit)).unwrap();
        lib.set_tags(id, &["again".into()]).unwrap();
        assert!(!aside.exists());
        // An edit and tags as a newer Zenith might keep them.
        let newer = r#"{"version":9,"cuts":[[0,4]]}"#;
        lib.db.execute("UPDATE clips SET edit = ?2, tags = 'boss, flawless' WHERE id = ?1", params![id, newer]).unwrap();
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!((clip.edit, clip.tags.len()), (None, 0), "shown as a clip with neither");
        lib.set_edit(id, Some(&edit)).unwrap();
        lib.add_tags(&[id], &["new".into()]).unwrap();
        let clip = lib.get(id).unwrap().unwrap();
        assert_eq!((clip.edit, clip.tags.as_slice()), (Some(edit), &["new".to_string()][..]), "the clip can be worked on again");
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(&aside).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0]["clip"].as_i64(), lines[0]["what"].as_str(), lines[0]["was"].as_str()), (Some(id), Some("edit"), Some(newer)));
        assert_eq!((lines[1]["what"].as_str(), lines[1]["was"].as_str()), (Some("tags"), Some("boss, flawless")));
        assert!(lines[0]["path"].as_str().unwrap().ends_with("Hades 2026-11-20 21-04-33.mp4"));
    }

    #[test]
    fn tracks_are_guessed_from_their_number() {
        assert_eq!(guess_tracks(0), []);
        assert_eq!(guess_tracks(2), [Track::Game, Track::Microphone]);
        assert_eq!(guess_tracks(3), [Track::Game, Track::Chat, Track::Microphone]);
    }
}
