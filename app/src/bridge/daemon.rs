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
        /// `running` is false when the daemon is not there. The asking
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

fn status_now() -> String {
    let value = match ipc::ask(&paths::socket(), &Request::Status, Duration::from_secs(2)) {
        Ok(Response::Status(s)) => json!({
            "running": true,
            "state": s.state,
            "detail": s.detail,
            "game": s.game,
            "forced": s.forced,
            "bufferSecs": s.buffer_secs,
            "recording": s.recording,
            "fallback": s.recording.as_deref().is_some_and(zenith_core::recorder::is_fallback),
        }),
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
