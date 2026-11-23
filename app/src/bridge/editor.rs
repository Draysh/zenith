//! The editor's state for the screens: the clip being edited, its edit
//! with undo and redo, and exporting the result. The edit is kept in the
//! library as it changes, so closing the window loses nothing.

use zenith_core::config::Config;
use zenith_core::edit::{Edit, History};
use zenith_core::export::ffmpeg::{ExportError, Job, Picture, Sound, run};
use zenith_core::export::{Preset, Source, plan};
use zenith_core::library::{self as lib, Clip};
use zenith_core::paths;
use zenith_core::volumes::Volumes;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Editor = super::EditorRust;

        /// Start editing a clip. Returns the state (see `state`), or `{}`
        /// when the clip is gone.
        #[qinvokable]
        fn open(self: Pin<&mut Editor>, id: i64) -> QString;
        /// Whether a handle or slider is held (between `grab` and `release`).
        #[qinvokable]
        fn held(self: &Editor) -> bool;
        /// `{duration, segments: [{start, end}], gains: [{volume, muted}],
        /// audioOffsetMs, kept, changed, canUndo, canRedo}` as JSON.
        #[qinvokable]
        fn state(self: &Editor) -> QString;

        /// Cut the kept stretch at `t` in two.
        #[qinvokable]
        fn split(self: Pin<&mut Editor>, t: f64) -> QString;
        /// Drop a kept stretch.
        #[qinvokable]
        fn remove(self: Pin<&mut Editor>, index: i32) -> QString;
        /// Move a stretch's start or end (while a handle is dragged: call
        /// `grab` first and `release` after, so the drag is one undo step).
        #[qinvokable]
        fn set_start(self: Pin<&mut Editor>, index: i32, t: f64) -> QString;
        #[qinvokable]
        fn set_end(self: Pin<&mut Editor>, index: i32, t: f64) -> QString;
        #[qinvokable]
        fn set_volume(self: Pin<&mut Editor>, track: i32, volume: f64) -> QString;
        #[qinvokable]
        fn set_muted(self: Pin<&mut Editor>, track: i32, muted: bool) -> QString;
        /// Put this clip's sound later (or, below zero, earlier) against
        /// its picture, in milliseconds.
        #[qinvokable]
        fn set_audio_offset(self: Pin<&mut Editor>, ms: i32) -> QString;
        #[qinvokable]
        fn grab(self: Pin<&mut Editor>);
        #[qinvokable]
        fn release(self: Pin<&mut Editor>) -> QString;
        #[qinvokable]
        fn undo(self: Pin<&mut Editor>) -> QString;
        #[qinvokable]
        fn redo(self: Pin<&mut Editor>) -> QString;

        /// How loud the editor played last time, 0 to 1, and keeping it
        /// for the next.
        #[qinvokable]
        fn listening(self: &Editor) -> f64;
        #[qinvokable]
        fn set_listening(self: &Editor, volume: f64);

        /// Where playback at clip time `t` should be: `t`, the start of the
        /// next kept stretch, or -1 past the end of the last.
        #[qinvokable]
        fn playable(self: &Editor, t: f64) -> f64;

        /// What an export to a size would look like: `{width, height, fps,
        /// videoKbps, audioKbps, bytes}`, or `{error}` when it cannot fit.
        /// `megabytes` 0 means full quality.
        #[qinvokable]
        fn plan(self: &Editor, megabytes: i32) -> QString;
        /// Export the edit. `megabytes` 0 keeps full quality and the audio
        /// tracks apart, as a new clip in the library; otherwise the result
        /// fits that size, with one mixed track. `exportProgress` and
        /// `exportFinished` follow, each with the id of the clip it is
        /// about (-1 when there was no clip to export). One export runs at
        /// a time: a second is refused through `exportFinished`.
        #[qinvokable]
        fn export_start(self: Pin<&mut Editor>, megabytes: i32);
        #[qinvokable]
        fn export_cancel(self: &Editor);
        /// Whether an export is running (or still stopping after a cancel).
        #[qinvokable]
        fn export_busy(self: &Editor) -> bool;

        #[qsignal]
        fn export_progress(self: Pin<&mut Editor>, clip: i64, fraction: f64);
        /// `error` is empty when it worked.
        #[qsignal]
        fn export_finished(self: Pin<&mut Editor>, clip: i64, path: QString, bytes: f64, error: QString);
    }

    impl cxx_qt::Threading for Editor {}
}

#[derive(Default)]
pub struct EditorRust {
    clip: Option<Clip>,
    history: Option<History<Edit>>,
    /// The edit as it was when a handle was grabbed.
    grabbed: Option<Edit>,
    cancel: Arc<AtomicBool>,
    /// An export is running; it clears this itself when it ends.
    exporting: Arc<AtomicBool>,
}

fn library() -> Option<lib::Library> {
    lib::Library::open(&paths::data_dir().join("library.db")).inspect_err(|e| eprintln!("zenith-clips: {e}")).ok()
}

/// A name that is safe as a file name.
fn file_name(title: &str) -> String {
    zenith_core::games::folder_name(title)
}

impl EditorRust {
    fn edit(&self) -> Option<&Edit> {
        self.history.as_ref().map(History::now)
    }

    fn state(&self) -> String {
        let (Some(history), Some(edit)) = (&self.history, self.edit()) else { return "{}".into() };
        json!({
            "duration": edit.duration,
            "segments": edit.segments,
            "gains": edit.gains,
            "audioOffsetMs": edit.audio_offset_ms,
            "kept": edit.kept(),
            "changed": edit.is_changed(),
            "canUndo": history.can_undo(),
            "canRedo": history.can_redo(),
        })
        .to_string()
    }

    /// Keep the edit in the library. Also one that is back at the clip as
    /// recorded: the clip then stays so, instead of starting again from the
    /// volumes the settings give a clip not edited yet.
    fn persist(&self) {
        if let (Some(clip), Some(edit), Some(db)) = (&self.clip, self.edit(), library()) {
            let _ = db.set_edit(clip.id, Some(edit));
        }
    }

    /// Change the edit. While a handle is grabbed the change replaces the
    /// current state; otherwise it is a step of its own.
    fn change(&mut self, f: impl FnOnce(&mut Edit)) -> String {
        if let Some(history) = &mut self.history {
            let mut next = history.now().clone();
            f(&mut next);
            if self.grabbed.is_some() {
                history.amend(next);
            } else {
                history.push(next);
                self.persist();
            }
        }
        self.state()
    }

    fn source(&self) -> Option<Source> {
        let (clip, edit) = (self.clip.as_ref()?, self.edit()?);
        Some(Source { width: clip.width, height: clip.height, fps: clip.fps, seconds: edit.kept(), has_audio: edit.audible() })
    }
}

impl ffi::Editor {
    fn open(mut self: Pin<&mut Self>, id: i64) -> QString {
        let clip = library().and_then(|db| db.get(id).ok().flatten());
        let mut this = self.as_mut().rust_mut();
        this.grabbed = None;
        this.history = clip.as_ref().map(|c| {
            // An edit kept for a file that has since changed length is dropped.
            let fits = |e: &Edit| (e.duration - c.seconds).abs() < 0.5 && e.gains.len() == c.tracks.len();
            History::new(c.edit.clone().filter(fits).unwrap_or_else(|| {
                // A clip not edited before starts at the volumes the
                // settings give each kind of track. From its first change
                // on, its volumes are its own.
                let mut edit = Edit::new(c.seconds, c.tracks.len());
                let start = Config::load(&paths::config_file()).unwrap_or_default().editor;
                for (i, track) in c.tracks.iter().enumerate() {
                    edit.set_volume(i, start.initial_volume(*track));
                }
                edit
            }))
        });
        this.clip = clip;
        QString::from(&this.state())
    }

    fn state(&self) -> QString {
        QString::from(&self.rust().state())
    }

    fn held(&self) -> bool {
        self.rust().grabbed.is_some()
    }

    fn split(mut self: Pin<&mut Self>, t: f64) -> QString {
        QString::from(&self.as_mut().rust_mut().change(|e| {
            e.split(t);
        }))
    }

    fn remove(mut self: Pin<&mut Self>, index: i32) -> QString {
        QString::from(&self.as_mut().rust_mut().change(|e| {
            e.remove(index.max(0) as usize);
        }))
    }

    fn set_start(mut self: Pin<&mut Self>, index: i32, t: f64) -> QString {
        QString::from(&self.as_mut().rust_mut().change(|e| {
            e.set_start(index.max(0) as usize, t);
        }))
    }

    fn set_end(mut self: Pin<&mut Self>, index: i32, t: f64) -> QString {
        QString::from(&self.as_mut().rust_mut().change(|e| {
            e.set_end(index.max(0) as usize, t);
        }))
    }

    fn set_volume(mut self: Pin<&mut Self>, track: i32, volume: f64) -> QString {
        QString::from(&self.as_mut().rust_mut().change(|e| e.set_volume(track.max(0) as usize, volume)))
    }

    fn set_muted(mut self: Pin<&mut Self>, track: i32, muted: bool) -> QString {
        QString::from(&self.as_mut().rust_mut().change(|e| e.set_muted(track.max(0) as usize, muted)))
    }

    fn set_audio_offset(mut self: Pin<&mut Self>, ms: i32) -> QString {
        QString::from(&self.as_mut().rust_mut().change(|e| e.set_audio_offset(ms)))
    }

    fn grab(mut self: Pin<&mut Self>) {
        let mut this = self.as_mut().rust_mut();
        this.grabbed = this.edit().cloned();
    }

    fn release(mut self: Pin<&mut Self>) -> QString {
        let mut this = self.as_mut().rust_mut();
        if let (Some(before), Some(history)) = (this.grabbed.take(), this.history.as_mut()) {
            // The whole drag becomes one step from where it began.
            let after = history.now().clone();
            history.amend(before);
            history.push(after);
        }
        this.persist();
        QString::from(&this.state())
    }

    fn undo(mut self: Pin<&mut Self>) -> QString {
        let mut this = self.as_mut().rust_mut();
        if let Some(history) = this.history.as_mut() {
            history.undo();
        }
        this.persist();
        QString::from(&this.state())
    }

    fn redo(mut self: Pin<&mut Self>) -> QString {
        let mut this = self.as_mut().rust_mut();
        if let Some(history) = this.history.as_mut() {
            history.redo();
        }
        this.persist();
        QString::from(&this.state())
    }

    fn listening(&self) -> f64 {
        Volumes::load(&Volumes::file()).listening
    }

    fn set_listening(&self, volume: f64) {
        let mut kept = Volumes::load(&Volumes::file());
        kept.listening = volume.clamp(0.0, 1.0);
        let _ = kept.save(&Volumes::file());
    }

    fn playable(&self, t: f64) -> f64 {
        self.rust().edit().and_then(|e| e.playable(t)).unwrap_or(-1.0)
    }

    fn plan(&self, megabytes: i32) -> QString {
        let Some(source) = self.rust().source() else { return QString::from("{}") };
        let value: Value = if megabytes <= 0 {
            json!({ "width": source.width, "height": source.height, "fps": source.fps, "full": true })
        } else {
            match plan(source, Preset::Custom { megabytes: megabytes as u32 }) {
                Ok(p) => json!({
                    "width": p.width, "height": p.height, "fps": p.fps,
                    "videoKbps": p.video_kbps, "audioKbps": p.audio_kbps,
                    "bytes": p.expected_bytes(source.seconds),
                }),
                Err(e) => json!({ "error": e.to_string() }),
            }
        };
        QString::from(&value.to_string())
    }

    fn export_start(self: Pin<&mut Self>, megabytes: i32) {
        let this = self.rust();
        // The clip as the library has it now: renamed since it was opened,
        // the export carries the new name.
        let clip = this.clip.as_ref().map(|c| library().and_then(|db| db.get(c.id).ok().flatten()).unwrap_or_else(|| c.clone()));
        let (Some(clip), Some(edit), Some(source)) = (clip, this.edit().cloned(), this.source()) else {
            let message = QString::from("This clip is no longer in the library.");
            return self.export_finished(-1, QString::default(), 0.0, message);
        };
        let id = clip.id;
        if this.exporting.load(Ordering::SeqCst) {
            let message = QString::from("Another export is still running. Try again when it has finished.");
            return self.export_finished(id, QString::default(), 0.0, message);
        }
        let clip_dir = Config::load(&paths::config_file()).unwrap_or_default().library.dir;
        let full = megabytes <= 0;
        let preset = Preset::Custom { megabytes: megabytes.max(1) as u32 };
        let (output, picture, sound, limit) = if full {
            // A new clip beside the one it was made from.
            let folder = clip.path.parent().map(PathBuf::from).unwrap_or(clip_dir);
            (lib::free_path(&folder.join(format!("{} (edit).mp4", file_name(&clip.title)))), Picture::Full, Sound::Tracks, None)
        } else {
            let planned = match plan(source, preset) {
                Ok(p) => p,
                Err(e) => {
                    let message = QString::from(&e.to_string());
                    return self.export_finished(id, QString::default(), 0.0, message);
                }
            };
            let folder = clip_dir.parent().map(PathBuf::from).unwrap_or(clip_dir.clone()).join("Clip Exports");
            // Under the clip's name and nothing else. (Another export of
            // that name already there is kept: this one gets a number.)
            let name = format!("{}.mp4", file_name(&clip.title));
            (lib::free_path(&folder.join(name)), Picture::Fit(planned), Sound::Mixed, Some(preset.max_bytes()))
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let qt = self.qt_thread();
        let mut this = self.rust_mut();
        this.cancel = cancel.clone();
        let exporting = this.exporting.clone();
        exporting.store(true, Ordering::SeqCst);
        std::thread::spawn(move || {
            let job = Job { input: clip.path.clone(), output: output.clone(), edit, picture, sound };
            let progress_qt = qt.clone();
            let mut last = -1.0;
            let result = run(&job, limit, &cancel, move |fraction: f64| {
                // A hundred steps are plenty for a bar.
                if fraction - last >= 0.01 || fraction >= 1.0 {
                    last = fraction;
                    let _ = progress_qt.queue(move |editor| editor.export_progress(id, fraction));
                }
            });
            let (path, bytes, error) = match result {
                Ok(bytes) => {
                    if full && let (Some(db), Ok(meta)) = (library(), lib::probe(&output)) {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                        // Its volumes are in its sound already: it opens as
                        // it is, not at the settings' starting volumes on top.
                        if let Ok(new) = db.add(&output, &clip.game, now, &meta, &clip.tracks)
                            && let Ok(Some(added)) = db.get(new)
                        {
                            let _ = db.set_edit(new, Some(&Edit::new(added.seconds, added.tracks.len())));
                        }
                    }
                    (output.to_string_lossy().into_owned(), bytes as f64, String::new())
                }
                Err(ExportError::Cancelled) => (String::new(), 0.0, "cancelled".to_string()),
                Err(e) => (String::new(), 0.0, e.to_string()),
            };
            exporting.store(false, Ordering::SeqCst);
            let _ = qt.queue(move |editor| editor.export_finished(id, QString::from(&path), bytes, QString::from(&error)));
        });
    }

    fn export_cancel(&self) {
        self.rust().cancel.store(true, Ordering::Relaxed);
    }

    fn export_busy(&self) -> bool {
        self.rust().exporting.load(Ordering::SeqCst)
    }
}
