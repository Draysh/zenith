//! The recorder, as the screens see it: what it is doing, and asking it to
//! save, switch capture or read the settings again.

use zenith_core::ipc::{self, IpcError, Request, Response};
use zenith_core::paths;
use cxx_qt::Threading;
use cxx_qt_lib::QString;
use serde_json::json;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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
        type Recorder = super::RecorderRust;

        /// Ask the recorder what it is doing. `statusReady` brings
        /// `{running, state, detail, game, forced, bufferSecs}` as JSON;
        /// `running` is false when the daemon is not there. `version` is
        /// the daemon's own (empty from one too old to say) and
        /// `otherVersion` is true when that is not this app's: the daemon
        /// was left running across an update and has to be started again
        /// before it does what this version does. `hotkey` is the save
        /// hotkey the daemon has (on Linux, the one the desktop bound for
        /// it) and `hotkeyMissing` is why it has none of its own, where a
        /// key has to be bound to `zenithd save` by hand. The asking
        /// happens off the window's thread, so a recorder that is slow to
        /// answer does not hold the window up.
        #[qinvokable]
        fn poll(self: Pin<&mut Recorder>);
        #[qsignal]
        fn status_ready(self: Pin<&mut Recorder>, status: QString);
        /// `"on"`, `"off"` or `"auto"`.
        #[qinvokable]
        fn set_capture(self: &Recorder, mode: &QString);
        /// Have the daemon read the settings file again. Returns an error
        /// message, empty when it worked.
        #[qinvokable]
        fn reload(self: &Recorder) -> QString;
    }

    impl cxx_qt::Threading for Recorder {}
}

#[derive(Default)]
pub struct RecorderRust {
    /// A question is on its way; the next one waits for its answer.
    asking: Arc<AtomicBool>,
}

const WAIT: Duration = Duration::from_millis(400);

/// Say, once for each daemon found to be of another version, that it is.
fn note_other_version(theirs: &str) {
    static SAID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if said.as_deref() != Some(theirs) {
        let named = if theirs.is_empty() { "an older version" } else { theirs };
        eprintln!("zenith-clips: the recorder that is running is {named}, not {}; it has to be started again to be this version", env!("CARGO_PKG_VERSION"));
        *said = Some(theirs.to_string());
    }
}

fn status_now() -> String {
    let value = match ipc::ask(&paths::socket(), &Request::Status, Duration::from_secs(2)) {
        Ok(Response::Status(s)) => {
            let other_version = s.other_version(env!("CARGO_PKG_VERSION"));
            if other_version {
                note_other_version(&s.version);
            }
            json!({
                "running": true,
                "version": s.version,
                "otherVersion": other_version,
                "hotkey": s.hotkey,
                "hotkeyMissing": s.hotkey_missing,
                "state": s.state,
                "detail": s.detail,
                "game": s.game,
                "forced": s.forced,
                "bufferSecs": s.buffer_secs,
                "recording": s.recording,
                "fallback": s.recording.as_deref().is_some_and(zenith_core::recorder::is_fallback),
            })
        }
        _ => json!({ "running": false, "state": "stopped" }),
    };
    value.to_string()
}

impl ffi::Recorder {
    fn poll(self: Pin<&mut Self>) {
        let asking = self.asking.clone();
        if asking.swap(true, Ordering::SeqCst) {
            return;
        }
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let status = status_now();
            asking.store(false, Ordering::SeqCst);
            let _ = qt.queue(move |recorder| recorder.status_ready(QString::from(&status)));
        });
    }

    fn set_capture(&self, mode: &QString) {
        let force = match mode.to_string().as_str() {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        let _ = ipc::ask(&paths::socket(), &Request::Capture { force }, WAIT);
    }

    fn reload(&self) -> QString {
        match ipc::ask(&paths::socket(), &Request::Reload, Duration::from_secs(5)) {
            Ok(Response::Error { message }) => QString::from(&message),
            Ok(_) => QString::default(),
            // Not running: it reads the file when it starts.
            Err(IpcError::NotRunning) => QString::default(),
            Err(e) => QString::from(&e.to_string()),
        }
    }
}
