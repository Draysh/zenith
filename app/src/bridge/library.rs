//! The clip library for the screens: listing, the games' shelves, and what
//! can be done to a clip or to many at once (favourite, tag, rename, move
//! to another game, copy, trash and take that back). Where several clips
//! are meant, their ids come as a JSON array.

use zenith_core::config::Config;
use zenith_core::games::steam;
use zenith_core::library::{self as lib, Clip, Filter, Order};
use zenith_core::paths;
use cxx_qt::Threading;
use cxx_qt_lib::QString;
use serde_json::{Value, json};
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("zenith/cpp/helpers.h");
        #[namespace = "cct"]
        #[cxx_name = "useOpenGl"]
        fn use_opengl();
        #[namespace = "cct"]
        #[cxx_name = "keepLog"]
        fn keep_log(path: &QString) -> bool;
        #[namespace = "cct"]
        #[cxx_name = "ownFrame"]
        fn own_frame();
        #[namespace = "cct"]
        #[cxx_name = "watchSizing"]
        fn watch_sizing();
        #[namespace = "cct"]
        #[cxx_name = "copyFiles"]
        fn copy_files(paths: &QString);
        #[namespace = "cct"]
        #[cxx_name = "moveToTrash"]
        fn move_to_trash(path: &QString) -> bool;
        #[namespace = "cct"]
        #[cxx_name = "fileUrl"]
        fn file_url_of(path: &QString) -> QString;
        #[namespace = "cct"]
        #[cxx_name = "localPath"]
        fn local_path_of(url: &QString) -> QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Library = super::LibraryRust;

        /// The clips matching `{game?, favorites?, tag?, search?, sort?}`,
        /// as a JSON array. `sort` is `newest`, `oldest`, `longest`,
        /// `shortest`, `largest` or `smallest`.
        #[qinvokable]
        fn clips(self: &Library, filter: &QString) -> QString;
        /// The games that have clips, as a JSON array.
        #[qinvokable]
        fn shelves(self: &Library) -> QString;
        /// The tags in use and how many clips have each, as a JSON array.
        #[qinvokable]
        fn tags(self: &Library) -> QString;
        /// Bring the index in line with the clip folder. Returns how many
        /// clips were added or removed. This waits for the folder to be
        /// read; `rescanStart` does the same without holding the window up.
        #[qinvokable]
        fn rescan(self: &Library) -> i32;
        /// As `rescan`, on another thread: `rescanFinished` says how many
        /// clips changed. One at a time; a second ask while one runs is dropped.
        #[qinvokable]
        fn rescan_start(self: Pin<&mut Library>);
        #[qsignal]
        fn rescan_finished(self: Pin<&mut Library>, changed: i32);
        #[qinvokable]
        fn set_favorite(self: &Library, id: i64, favorite: bool);
        #[qinvokable]
        fn set_favorites(self: &Library, ids: &QString, favorite: bool);
        #[qinvokable]
        fn rename(self: &Library, id: i64, title: &QString);
        /// Name a clip and give its file the same name. An error message,
        /// empty when it worked.
        #[qinvokable]
        fn rename_with_file(self: &Library, id: i64, title: &QString) -> QString;
        /// A clip's tags, as typed: `ace, clutch`.
        #[qinvokable]
        fn set_tags(self: &Library, id: i64, tags: &QString);
        /// Give several clips the tags typed, beside the ones they have.
        #[qinvokable]
        fn add_tags(self: &Library, ids: &QString, tags: &QString);
        #[qinvokable]
        fn set_notes(self: &Library, id: i64, notes: &QString);
        /// Move clips, with their files, to another game. An error
        /// message, empty when every one of them moved.
        #[qinvokable]
        fn move_to_game(self: &Library, ids: &QString, game: &QString) -> QString;
        /// Take clips out of the library. Their files stay where they are
        /// until `finishTrash`, so `undoTrash` can bring the clips back.
        /// The clips of an earlier `trash` are finished first. Returns how
        /// many were taken out.
        #[qinvokable]
        fn trash(self: Pin<&mut Library>, ids: &QString) -> i32;
        /// Bring back the clips of the last `trash`. Returns how many.
        #[qinvokable]
        fn undo_trash(self: &Library) -> i32;
        /// Move the files of the last `trash` to the system's trash, on
        /// another thread. `trashed` follows only when some could not be
        /// moved: those clips are back in the library.
        #[qinvokable]
        fn finish_trash(self: Pin<&mut Library>);
        /// As `finishTrash`, waiting for it: for when the window closes.
        #[qinvokable]
        fn finish_trash_now(self: &Library);
        /// As `finishTrash`, for clips a window that was closed in a hurry
        /// left behind.
        #[qinvokable]
        fn finish_old_trash(self: Pin<&mut Library>);
        #[qsignal]
        fn trashed(self: Pin<&mut Library>, kept: i32, problem: QString);
        /// Put clips' files on the clipboard, to paste into a chat. One
        /// path to a line.
        #[qinvokable]
        fn copy_to_clipboard(self: &Library, paths: &QString);
        /// Add videos made elsewhere to the library, under `game`: copies
        /// of them, in the clip folder. `files` is one path or `file://`
        /// address to a line; a folder brings every video in it.
        /// `imported` follows.
        #[qinvokable]
        fn import_files(self: Pin<&mut Library>, files: &QString, game: &QString);
        #[qsignal]
        fn imported(self: Pin<&mut Library>, added: i32, problem: QString);
        /// The clip folder.
        #[qinvokable]
        fn folder(self: &Library) -> QString;
        /// A file's or folder's path as a `file://` address, and back.
        #[qinvokable]
        fn file_url(self: &Library, path: &QString) -> QString;
        #[qinvokable]
        fn local_path(self: &Library, url: &QString) -> QString;
    }

    impl cxx_qt::Threading for Library {}
}

pub use ffi::{keep_log, own_frame, use_opengl, watch_sizing};

#[derive(Default)]
pub struct LibraryRust {
    db: OnceCell<Option<lib::Library>>,
    steam_roots: OnceCell<Vec<PathBuf>>,
    /// Each game's cover, looked up once: finding it reads Steam's files.
    covers: RefCell<HashMap<String, Option<String>>>,
    scanning: Arc<AtomicBool>,
    /// The library as the threads that read the clip folder have it: one
    /// connection for all of them, opened by the first. It remembers which
    /// files could not be read as videos, so a broken one in the folder is
    /// looked into once and not at every look at the folder.
    scanner: Arc<Mutex<Option<lib::Library>>>,
    /// The clips of the last `trash`, while taking it back is still offered.
    undoable: RefCell<Vec<i64>>,
}

fn open_db() -> Option<lib::Library> {
    lib::Library::open(&paths::data_dir().join("library.db")).inspect_err(|e| eprintln!("zenith-clips: {e}")).ok()
}

fn rescan_now(db: &lib::Library) -> i32 {
    match db.rescan(&clip_dir(), lib::probe) {
        Ok(changes) => (changes.added + changes.removed + changes.moved) as i32,
        Err(e) => {
            eprintln!("zenith-clips: {e}");
            0
        }
    }
}

/// Read the clip folder with the threads' shared library (see `scanner`),
/// opening it when this is the first to want it.
fn rescan_shared(scanner: &Mutex<Option<lib::Library>>) -> i32 {
    // Whoever held it last only read the folder with it.
    let mut held = scanner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if held.is_none() {
        *held = open_db();
    }
    held.as_ref().map_or(0, rescan_now)
}

fn clip_dir() -> PathBuf {
    Config::load_usable(&paths::config_file()).library.dir
}

impl LibraryRust {
    fn db(&self) -> Option<&lib::Library> {
        self.db.get_or_init(open_db).as_ref()
    }

    /// A game's cover, from Steam's files, when it is a Steam game.
    fn cover(&self, game: &str) -> Option<String> {
        if let Some(known) = self.covers.borrow().get(game) {
            return known.clone();
        }
        let found = self.find_cover(game);
        self.covers.borrow_mut().insert(game.to_string(), found.clone());
        found
    }

    fn find_cover(&self, game: &str) -> Option<String> {
        let roots = self.steam_roots.get_or_init(|| {
            steam::roots(&paths::home())
        });
        let appid = steam::appid_by_name(game, roots)?;
        steam::cover(appid, roots).map(|p| p.to_string_lossy().into_owned())
    }
}

/// The ids in a JSON array; none when it is not one.
fn ids(text: &QString) -> Vec<i64> {
    serde_json::from_str(&text.to_string()).unwrap_or_default()
}

/// Move the files of trashed clips to the system's trash and forget the
/// clips. A clip whose file could not be moved comes back to the library.
/// Returns how many came back, and why.
fn settle_trash(db: &lib::Library, ids: Option<&[i64]>) -> (i32, String) {
    let mut clips = db.in_trash().unwrap_or_default();
    if let Some(ids) = ids {
        clips.retain(|c| ids.contains(&c.id));
    }
    let paths: Vec<&std::path::Path> = clips.iter().map(|c| c.path.as_path()).collect();
    let problem = to_trash(&paths).err().unwrap_or_default();
    // Whatever the trash said, a file that is not there any more is gone.
    let (gone, kept): (Vec<&Clip>, Vec<&Clip>) = clips.iter().partition(|c| !c.path.exists());
    for clip in gone {
        let _ = db.forget(clip.id);
    }
    let _ = db.restore_some(&kept.iter().map(|c| c.id).collect::<Vec<_>>());
    (kept.len() as i32, if kept.is_empty() { String::new() } else { problem })
}

/// `gio trash` puts files where the file manager's trash is, so they can
/// be restored. A hundred at a time: one program started, not one each.
#[cfg(not(windows))]
fn to_trash(paths: &[&std::path::Path]) -> Result<(), String> {
    let mut problem = None;
    for some in paths.chunks(100) {
        match std::process::Command::new("gio").arg("trash").args(some).output() {
            Ok(out) if out.status.success() => {}
            Ok(out) => problem = String::from_utf8_lossy(&out.stderr).lines().next().map(|l| l.trim().to_string()),
            // No `gio` here: Qt knows where the trash is too.
            Err(e) => {
                if !some.iter().all(|path| ffi::move_to_trash(&QString::from(&*path.to_string_lossy()))) {
                    problem = Some(format!("The trash could not be used: {e}"));
                }
            }
        }
    }
    problem.map_or(Ok(()), Err)
}

/// On Windows the trash is the Recycle Bin, which Qt moves files to.
#[cfg(windows)]
fn to_trash(paths: &[&std::path::Path]) -> Result<(), String> {
    let all = paths.iter().fold(true, |ok, path| ffi::move_to_trash(&QString::from(&*path.to_string_lossy())) && ok);
    if all { Ok(()) } else { Err("The file could not be moved to the Recycle Bin.".into()) }
}

/// Copy dropped videos (and the videos in dropped folders) into the clip
/// folder under `game`. Returns how many came in, and what went wrong with
/// the first that did not.
fn import_now(files: &[PathBuf], game: &str) -> (i32, String) {
    let dir = clip_dir();
    let videos = files.iter().flat_map(|f| if f.is_dir() { lib::files(f) } else { vec![f.clone()] });
    let (mut added, mut problem) = (0, String::new());
    for video in videos {
        match lib::import(&dir, game, &video) {
            Ok(_) => added += 1,
            Err(e) if problem.is_empty() => problem = e.to_string(),
            Err(_) => {}
        }
    }
    (added, problem)
}

/// How many of `count` clips something went wrong with, and the first reason.
fn some_failed(failed: usize, count: usize, what: &str, why: &str) -> String {
    match (failed, count) {
        (0, _) => String::new(),
        (1, 1) => format!("The clip could not be {what}: {why}"),
        _ => format!("{failed} of {count} clips could not be {what}: {why}"),
    }
}

fn clip_json(c: &Clip) -> Value {
    json!({
        "id": c.id,
        "path": c.path,
        "game": c.game,
        "title": c.title,
        "savedAt": c.saved_at,
        "seconds": c.seconds,
        "width": c.width,
        "height": c.height,
        "fps": c.fps,
        "bytes": c.bytes,
        "favorite": c.favorite,
        "tracks": c.tracks.iter().map(|t| t.label()).collect::<Vec<_>>(),
        "trimmed": c.edit.as_ref().is_some_and(|e| e.is_trimmed()),
        "tags": c.tags,
        "notes": c.notes,
    })
}

impl ffi::Library {
    fn clips(&self, filter: &QString) -> QString {
        let asked: Value = serde_json::from_str(&filter.to_string()).unwrap_or_default();
        let filter = Filter {
            game: asked["game"].as_str().filter(|g| !g.is_empty()).map(String::from),
            favorites: asked["favorites"].as_bool().unwrap_or(false),
            tag: asked["tag"].as_str().filter(|t| !t.is_empty()).map(String::from),
            search: asked["search"].as_str().unwrap_or_default().to_string(),
            order: Order::named(asked["sort"].as_str().unwrap_or_default()),
        };
        let clips = self.db().and_then(|db| db.list(&filter).ok()).unwrap_or_default();
        QString::from(&Value::Array(clips.iter().map(clip_json).collect()).to_string())
    }

    fn shelves(&self) -> QString {
        let shelves = self.db().and_then(|db| db.shelves().ok()).unwrap_or_default();
        let out: Vec<Value> = shelves
            .iter()
            .map(|s| json!({ "game": s.game, "clips": s.clips, "latest": s.latest, "cover": self.cover(&s.game) }))
            .collect();
        QString::from(&Value::Array(out).to_string())
    }

    fn tags(&self) -> QString {
        let tags = self.db().and_then(|db| db.tags().ok()).unwrap_or_default();
        let out: Vec<Value> = tags.iter().map(|t| json!({ "tag": t.tag, "clips": t.clips })).collect();
        QString::from(&Value::Array(out).to_string())
    }

    fn rescan(&self) -> i32 {
        self.db().map_or(0, rescan_now)
    }

    fn rescan_start(self: Pin<&mut Self>) {
        let scanning = self.scanning.clone();
        if scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let qt = self.qt_thread();
        let scanner = self.scanner.clone();
        std::thread::spawn(move || {
            // Not the window's connection, which stays on the window's thread.
            let changed = rescan_shared(&scanner);
            scanning.store(false, Ordering::SeqCst);
            let _ = qt.queue(move |library| library.rescan_finished(changed));
        });
    }

    fn set_favorite(&self, id: i64, favorite: bool) {
        if let Some(db) = self.db() {
            let _ = db.set_favorite(id, favorite);
        }
    }

    fn rename(&self, id: i64, title: &QString) {
        if let Some(db) = self.db() {
            let _ = db.set_title(id, &title.to_string());
        }
    }

    fn set_favorites(&self, ids: &QString, favorite: bool) {
        if let Some(db) = self.db() {
            let _ = db.set_favorites(&self::ids(ids), favorite);
        }
    }

    fn rename_with_file(&self, id: i64, title: &QString) -> QString {
        let Some(db) = self.db() else { return QString::from("The library could not be opened.") };
        let problem = db.rename_file(id, &title.to_string()).err().map(|e| format!("The file could not be renamed: {e}")).unwrap_or_default();
        QString::from(&problem)
    }

    fn set_tags(&self, id: i64, tags: &QString) {
        if let Some(db) = self.db() {
            let _ = db.set_tags(id, &lib::tags_from(&tags.to_string()));
        }
    }

    fn add_tags(&self, ids: &QString, tags: &QString) {
        if let Some(db) = self.db() {
            let _ = db.add_tags(&self::ids(ids), &lib::tags_from(&tags.to_string()));
        }
    }

    fn set_notes(&self, id: i64, notes: &QString) {
        if let Some(db) = self.db() {
            let _ = db.set_notes(id, &notes.to_string());
        }
    }

    fn move_to_game(&self, ids: &QString, game: &QString) -> QString {
        let Some(db) = self.db() else { return QString::from("The library could not be opened.") };
        let (ids, dir, game) = (self::ids(ids), clip_dir(), game.to_string());
        let problems: Vec<String> = ids.iter().filter_map(|id| db.move_to_game(*id, &dir, &game).err()).map(|e| e.to_string()).collect();
        QString::from(&some_failed(problems.len(), ids.len(), "moved", problems.first().map_or("", String::as_str)))
    }

    fn trash(mut self: Pin<&mut Self>, ids: &QString) -> i32 {
        // Only the newest trash can be taken back.
        self.as_mut().finish_trash();
        let ids = self::ids(ids);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        if self.db().is_none_or(|db| db.trash(&ids, now).is_err()) {
            return 0;
        }
        let count = ids.len() as i32;
        *self.undoable.borrow_mut() = ids;
        count
    }

    fn undo_trash(&self) -> i32 {
        let ids = std::mem::take(&mut *self.undoable.borrow_mut());
        match self.db() {
            Some(db) if db.restore_some(&ids).is_ok() => ids.len() as i32,
            _ => 0,
        }
    }

    fn finish_trash(self: Pin<&mut Self>) {
        let ids = std::mem::take(&mut *self.undoable.borrow_mut());
        if !ids.is_empty() {
            self.settle_on_a_thread(Some(ids));
        }
    }

    fn finish_trash_now(&self) {
        let ids = std::mem::take(&mut *self.undoable.borrow_mut());
        if let Some(db) = self.db().filter(|_| !ids.is_empty()) {
            settle_trash(db, Some(&ids));
        }
    }

    fn finish_old_trash(self: Pin<&mut Self>) {
        if self.db().is_some_and(|db| db.in_trash().is_ok_and(|clips| !clips.is_empty())) {
            self.settle_on_a_thread(None);
        }
    }

    /// `gio` can take a moment (a slow drive, a network share, hundreds of
    /// files), so the window does not wait for it.
    fn settle_on_a_thread(self: Pin<&mut Self>, ids: Option<Vec<i64>>) {
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            // Its own connection: the window's stays on the window's thread.
            let (kept, problem) = match open_db() {
                Some(db) => settle_trash(&db, ids.as_deref()),
                None => (0, String::new()),
            };
            if kept > 0 {
                let _ = qt.queue(move |library| library.trashed(kept, QString::from(&problem)));
            }
        });
    }

    fn copy_to_clipboard(&self, paths: &QString) {
        ffi::copy_files(paths);
    }

    fn import_files(self: Pin<&mut Self>, files: &QString, game: &QString) {
        let files: Vec<PathBuf> =
            files.to_string().lines().filter(|l| !l.is_empty()).map(|l| PathBuf::from(ffi::local_path_of(&QString::from(l)).to_string())).collect();
        let game = game.to_string();
        let qt = self.qt_thread();
        let scanner = self.scanner.clone();
        // A long video takes a while to copy.
        std::thread::spawn(move || {
            let (added, problem) = import_now(&files, &game);
            rescan_shared(&scanner);
            let _ = qt.queue(move |library| library.imported(added, QString::from(&problem)));
        });
    }

    fn folder(&self) -> QString {
        QString::from(clip_dir().to_string_lossy().as_ref())
    }

    fn file_url(&self, path: &QString) -> QString {
        ffi::file_url_of(path)
    }

    fn local_path(&self, url: &QString) -> QString {
        ffi::local_path_of(url)
    }
}
