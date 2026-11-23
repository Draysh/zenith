//! Thumbnails, filmstrips and waveforms for the screens. Making one takes
//! ffmpeg a moment, so it happens on worker threads: a screen asks, gets
//! what is cached at once, and is told when the rest is ready.

use zenith_core::jobs::Jobs;
use zenith_core::media::Cache;
use zenith_core::paths;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};

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
        type Media = super::MediaRust;

        /// A clip's thumbnail: its file when it is cached, else empty, and
        /// `thumbnailReady` follows.
        #[qinvokable]
        fn thumbnail(self: Pin<&mut Media>, path: &QString, seconds: f64) -> QString;
        /// The card that asked for `path`'s thumbnail is not on screen any
        /// more: if it is not being made yet it is left unmade, so the
        /// cards that are on screen get theirs first.
        #[qinvokable]
        fn thumbnail_unwanted(self: &Media, path: &QString);
        /// `count` frames along the clip in one picture; as `thumbnail`.
        #[qinvokable]
        fn filmstrip(self: Pin<&mut Media>, path: &QString, seconds: f64, count: i32) -> QString;
        /// An audio track's shape; `waveformReady` brings it as a JSON array.
        #[qinvokable]
        fn waveform(self: Pin<&mut Media>, path: &QString, track: i32, buckets: i32);

        #[qsignal]
        fn thumbnail_ready(self: Pin<&mut Media>, path: QString, file: QString);
        #[qsignal]
        fn filmstrip_ready(self: Pin<&mut Media>, path: QString, count: i32, file: QString);
        #[qsignal]
        fn waveform_ready(self: Pin<&mut Media>, path: QString, track: i32, peaks: QString);
    }

    impl cxx_qt::Threading for Media {}
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Job {
    Thumbnail { path: PathBuf, seconds_ms: u64 },
    Filmstrip { path: PathBuf, seconds_ms: u64, count: u32 },
    Waveform { path: PathBuf, track: u32, buckets: usize },
}

impl Job {
    /// What someone is looking at right now (the editor's filmstrip and
    /// waveforms, a card being scrubbed) goes before the library's thumbnails.
    fn urgent(&self) -> bool {
        !matches!(self, Job::Thumbnail { .. })
    }
}

#[derive(Default)]
struct Shared {
    /// What the workers have to do, the urgent line first.
    queue: Mutex<Jobs<Job>>,
    more: Condvar,
}

#[derive(Default)]
pub struct MediaRust {
    shared: Arc<Shared>,
    started: bool,
}

fn cache() -> Cache {
    Cache::new(paths::cache_dir())
}

fn text(path: &std::path::Path) -> QString {
    QString::from(path.to_string_lossy().as_ref())
}

fn work(shared: Arc<Shared>, qt: cxx_qt::CxxQtThread<ffi::Media>) {
    let cache = cache();
    loop {
        let job = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if let Some(job) = queue.take() {
                    break job;
                }
                queue = shared.more.wait(queue).unwrap();
            }
        };
        // Still pending while it is made, so a second request waits for this one.
        let made = job.clone();
        let mut failed = false;
        // A failure is left silent on the screen (the card keeps its
        // placeholder) and said once here.
        let sent = match job {
            Job::Thumbnail { path, seconds_ms } => match cache.thumbnail(&path, seconds_ms as f64 / 1000.0) {
                Ok(file) => qt.queue(move |media| media.thumbnail_ready(text(&path), text(&file))),
                Err(e) => {
                    eprintln!("zenith-clips: no thumbnail for {}: {e}", path.display());
                    failed = true;
                    Ok(())
                }
            },
            Job::Filmstrip { path, seconds_ms, count } => match cache.filmstrip(&path, seconds_ms as f64 / 1000.0, count) {
                Ok(file) => qt.queue(move |media| media.filmstrip_ready(text(&path), count as i32, text(&file))),
                Err(e) => {
                    eprintln!("zenith-clips: no filmstrip for {}: {e}", path.display());
                    failed = true;
                    Ok(())
                }
            },
            Job::Waveform { path, track, buckets } => {
                let peaks = cache.waveform(&path, track, buckets).unwrap_or_default();
                let json = serde_json::to_string(&peaks).unwrap_or_else(|_| "[]".into());
                qt.queue(move |media| media.waveform_ready(text(&path), track as i32, QString::from(&json)))
            }
        };
        shared.queue.lock().unwrap().done(made, failed);
        if sent.is_err() {
            // The window is gone.
            return;
        }
    }
}

impl ffi::Media {
    fn submit(mut self: Pin<&mut Self>, job: Job) {
        let urgent = job.urgent();
        if !self.shared.queue.lock().unwrap().ask(job, urgent) {
            return;
        }
        self.shared.more.notify_one();
        if !self.started {
            for _ in 0..3 {
                let (shared, qt) = (self.shared.clone(), self.qt_thread());
                std::thread::spawn(move || work(shared, qt));
            }
            self.as_mut().rust_mut().started = true;
        }
    }

    fn thumbnail(self: Pin<&mut Self>, path: &QString, seconds: f64) -> QString {
        let path = PathBuf::from(path.to_string());
        let cached = cache().thumbnail_path(&path);
        if cached.is_file() {
            return text(&cached);
        }
        self.submit(Job::Thumbnail { path, seconds_ms: (seconds.max(0.0) * 1000.0) as u64 });
        QString::default()
    }

    fn thumbnail_unwanted(&self, path: &QString) {
        let unwanted = PathBuf::from(path.to_string());
        self.shared.queue.lock().unwrap().withdraw(|job| matches!(job, Job::Thumbnail { path, .. } if *path == unwanted));
    }

    fn filmstrip(self: Pin<&mut Self>, path: &QString, seconds: f64, count: i32) -> QString {
        let path = PathBuf::from(path.to_string());
        let count = count.clamp(1, 120) as u32;
        let cached = cache().filmstrip_path(&path, count);
        if cached.is_file() {
            return text(&cached);
        }
        self.submit(Job::Filmstrip { path, seconds_ms: (seconds.max(0.0) * 1000.0) as u64, count });
        QString::default()
    }

    fn waveform(self: Pin<&mut Self>, path: &QString, track: i32, buckets: i32) {
        let job = Job::Waveform { path: PathBuf::from(path.to_string()), track: track.max(0) as u32, buckets: buckets.clamp(1, 8000) as usize };
        self.submit(job);
    }
}
