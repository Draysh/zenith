//! New versions, for the screens: looking for one, fetching it, putting it
//! in place, and starting it. The deciding (is it ours, is it newer, did
//! it start) is the core crate's; this fetches and reports.

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::json;
use std::io::Read;
use std::pin::Pin;
use std::time::Duration;
use zenith_core::config::Config;
use zenith_core::install::{self, Here};
use zenith_core::update::{self, File, Manifest};
use zenith_core::paths;

/// The keys a release must be signed with, one to a line.
const KEYS: &str = include_str!("../../update.pub");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The patch notes of every version, as this build was made with them.
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

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
        type Updates = super::UpdatesRust;

        /// How things stand: `{current, here: "portable"|"packaged"|
        /// "development", state: "idle"|"checking"|"current"|"available"|
        /// "downloading"|"ready"|"failed", latest, notes, progress (0..1),
        /// megabytes, error, rolledBack, currentNotes}`. `notes` are the
        /// patch notes of the version on offer, `currentNotes` those of
        /// the version running.
        #[qinvokable]
        fn status(self: &Updates) -> QString;
        /// `status` is different now.
        #[qsignal]
        fn changed(self: Pin<&mut Updates>);
        /// Look for a new version. `asked` is false when the app looks by
        /// itself, which it only does when the settings allow it.
        #[qinvokable]
        fn check(self: Pin<&mut Updates>, asked: bool);
        /// Fetch the new version and put it in place of this one.
        #[qinvokable]
        fn install(self: Pin<&mut Updates>);
        /// Start the version that was put in place. The app quits after.
        #[qinvokable]
        fn restart(self: &Updates);
        /// This version has run for a while: a trial of it is over.
        #[qinvokable]
        fn started_well(self: &Updates);
    }

    impl cxx_qt::Threading for Updates {}
}

#[derive(Default)]
pub struct UpdatesRust {
    state: State,
    offer: Option<(Manifest, File)>,
}

#[derive(Default, Clone, PartialEq)]
enum State {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading(f64),
    Ready,
    Failed(String),
}

/// Fetch `url`, at most `limit` bytes of it, telling `seen` how far it is.
fn fetch(url: &str, limit: u64, mut seen: impl FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30 * 60))).build().into();
    let mut response = agent.get(url).call().map_err(|e| format!("{url} could not be fetched: {e}"))?;
    let total = response.body().content_length();
    let mut reader = response.body_mut().with_config().limit(limit).reader();
    let mut bytes = Vec::with_capacity(total.unwrap_or(0).min(limit) as usize);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("the download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        seen(bytes.len() as u64, total);
    }
    Ok(bytes)
}

fn text(url: &str) -> Result<String, String> {
    String::from_utf8(fetch(url, 1024 * 1024, |_, _| {})?).map_err(|_| format!("{url} is not text"))
}

/// What the newest release offers this copy, if anything.
fn look() -> Result<Option<(Manifest, File)>, String> {
    let manifest = match text(&update::url(update::MANIFEST)) {
        Ok(manifest) => manifest,
        // Nothing has been released yet: there is nothing newer.
        Err(why) if why.ends_with("http status: 404") => return Ok(None),
        Err(why) => return Err(why),
    };
    let signature = text(&update::url(&format!("{}.minisig", update::MANIFEST)))?;
    update::offer(KEYS, &manifest, &signature, VERSION).map_err(|e| e.to_string())
}

/// Start the app at `staged` to see that it runs here: what it prints for
/// `--version`.
#[cfg(not(windows))]
fn runs(staged: &std::path::Path) -> std::io::Result<String> {
    let out = std::process::Command::new(staged).arg("--version").stdin(std::process::Stdio::null()).output()?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        return Err(std::io::Error::other(format!("the new version does not start here: {}", said.lines().last().unwrap_or("it gave no reason"))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Where a downloaded setup program waits to be run: in Zenith's own
/// folder, not the temporary one every program of the user's writes to.
#[cfg(windows)]
fn update_dir() -> std::path::PathBuf {
    paths::data_dir().join("update")
}

#[cfg(windows)]
fn setup_file(file: &File) -> std::path::PathBuf {
    update_dir().join(&file.name)
}

/// The signature kept beside the setup program, to check it by once more
/// right before it is run.
#[cfg(windows)]
fn signature_file(file: &File) -> std::path::PathBuf {
    update_dir().join(format!("{}.minisig", file.name))
}

/// Put a verified download in place of this version. On Linux the
/// programs in it are switched for the ones in `dir` there and then.
#[cfg(not(windows))]
fn put_in_place(dir: &std::path::Path, _file: &File, bytes: &[u8], _signature: &str, version: &str) -> Result<(), String> {
    let programs = install::unpack(bytes).map_err(|e| format!("the download does not unpack: {e}"))?;
    install::put_in_place(dir, &programs, VERSION, version, runs).map_err(|e| e.to_string())
}

/// Put a verified download in place of this version. On Windows it is a
/// setup program, which cannot replace files that are in use: it is kept,
/// and run when the app is restarted.
#[cfg(windows)]
fn put_in_place(_dir: &std::path::Path, file: &File, bytes: &[u8], signature: &str, _version: &str) -> Result<(), String> {
    let folder = update_dir();
    // What an earlier update left there is not kept.
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).map_err(|e| format!("{} could not be made: {e}", folder.display()))?;
    for (path, what) in [(setup_file(file), bytes), (signature_file(file), signature.as_bytes())] {
        std::fs::write(&path, what).map_err(|e| format!("{} could not be written: {e}", path.display()))?;
    }
    Ok(())
}

impl ffi::Updates {
    fn status(&self) -> QString {
        let me = self.rust();
        let here = install::here();
        let (state, progress, error) = match &me.state {
            State::Idle => ("idle", 0.0, ""),
            State::Checking => ("checking", 0.0, ""),
            State::Current => ("current", 0.0, ""),
            State::Available => ("available", 0.0, ""),
            State::Downloading(p) => ("downloading", *p, ""),
            State::Ready => ("ready", 1.0, ""),
            State::Failed(why) => ("failed", 0.0, why.as_str()),
        };
        let dir = install::current_exe().and_then(|exe| exe.parent().map(|d| d.to_path_buf()));
        let value = json!({
            "current": VERSION,
            "here": here.word(),
            "state": state,
            "latest": me.offer.as_ref().map(|(m, _)| m.version.clone()),
            "notes": me.offer.as_ref().map(|(m, _)| m.notes.clone()).unwrap_or_default(),
            "megabytes": me.offer.as_ref().map(|(_, f)| f.size as f64 / (1024.0 * 1024.0)).unwrap_or(0.0),
            "progress": progress,
            "error": error,
            "rolledBack": dir.and_then(|d| install::rolled_back(&d)),
            "currentNotes": update::notes_for(CHANGELOG, VERSION),
        });
        QString::from(&value.to_string())
    }

    fn set(mut self: Pin<&mut Self>, state: State) {
        self.as_mut().rust_mut().state = state;
        self.changed();
    }

    fn check(mut self: Pin<&mut Self>, asked: bool) {
        if matches!(self.rust().state, State::Checking | State::Downloading(_) | State::Ready) {
            return;
        }
        if !asked {
            let allowed = Config::load(&paths::config_file()).map(|c| c.updates.check).unwrap_or(true);
            if !allowed || install::here() == Here::Development || !update::has_key(KEYS) {
                return;
            }
        }
        self.as_mut().set(State::Checking);
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let found = look();
            let _ = qt.queue(move |mut updates| match found {
                Ok(Some(offer)) => {
                    updates.as_mut().rust_mut().offer = Some(offer);
                    updates.set(State::Available);
                }
                Ok(None) => {
                    updates.as_mut().rust_mut().offer = None;
                    updates.set(State::Current);
                }
                Err(why) => updates.set(State::Failed(why)),
            });
        });
    }

    fn install(mut self: Pin<&mut Self>) {
        let Some((manifest, file)) = self.rust().offer.clone() else { return };
        if self.rust().state != State::Available {
            return;
        }
        if install::here() != Here::Portable {
            self.set(State::Failed("This copy does not update itself.".into()));
            return;
        }
        let Some(dir) = install::current_exe().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) else { return };
        self.as_mut().set(State::Downloading(0.0));
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let progress = qt.clone();
            let mut last = 0u64;
            let done = (|| -> Result<(), String> {
                // No more is taken than the release says its file is (which
                // is part of what was signed), and a release is a few
                // hundred megabytes at the very most.
                let most = if file.size > 0 { file.size.saturating_add(1024 * 1024) } else { u64::MAX };
                let bytes = fetch(&update::url(&file.name), most.min(2 * 1024 * 1024 * 1024), |have, total| {
                    let total = total.unwrap_or(file.size).max(1);
                    let percent = have * 100 / total;
                    if percent != last {
                        last = percent;
                        let _ = progress.queue(move |updates| updates.set(State::Downloading((percent as f64 / 100.0).min(1.0))));
                    }
                })?;
                let signature = text(&update::url(&format!("{}.minisig", file.name)))?;
                update::verify(KEYS, &signature, &file.name, &bytes).map_err(|e| e.to_string())?;
                put_in_place(&dir, &file, &bytes, &signature, &manifest.version)
            })();
            let _ = qt.queue(move |updates| match done {
                Ok(()) => updates.set(State::Ready),
                Err(why) => updates.set(State::Failed(why)),
            });
        });
    }

    /// On Windows the new version is a setup program, kept ready: it is
    /// run now. It ends this window, puts the new files in and opens
    /// Zenith again.
    ///
    /// The setup program is not started as it is, which would show its own
    /// window: the recorder program knows how to run it unseen behind a
    /// small window in Zenith's colours (`zenithd update`). The files in
    /// Zenith's folder are about to be replaced, so that is done by a copy
    /// of the recorder program kept elsewhere. Where the copy cannot be
    /// made or started, the setup program is run as before.
    #[cfg(windows)]
    fn restart(&self) {
        use std::os::windows::process::CommandExt;
        // Only what was fetched and checked in this run is ever started.
        if self.rust().state != State::Ready {
            return;
        }
        let Some((manifest, file)) = self.rust().offer.clone() else { return };
        let setup = setup_file(&file);
        // It has lain there since it was fetched: it is run only if it
        // still is the file that was signed.
        let still_ours = std::fs::read(&setup)
            .and_then(|bytes| Ok((bytes, std::fs::read_to_string(signature_file(&file))?)))
            .map_err(|e| e.to_string())
            .and_then(|(bytes, signature)| update::verify(KEYS, &signature, &file.name, &bytes).map_err(|e| e.to_string()));
        if let Err(why) = still_ours {
            eprintln!("zenith-clips: the update is not run: {why}");
            return;
        }
        let styled = || -> std::io::Result<()> {
            let app = install::current_exe().ok_or_else(|| std::io::Error::other("where Zenith is installed is not known"))?;
            let folder = app.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
            let copy = update_dir().join("zenith-update.exe");
            std::fs::copy(folder.join(install::DAEMON), &copy)?;
            // The copy runs from another folder. Where the C runtime's
            // libraries are kept beside Zenith's programs and not in
            // Windows' own folder, it needs them beside it there too, or
            // it would not start and nothing would say so.
            for entry in std::fs::read_dir(folder)?.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                let runtime = ["vcruntime", "msvcp", "ucrtbase", "api-ms-win-crt"].iter().any(|start| name.starts_with(start));
                if runtime && name.ends_with(".dll") {
                    let _ = std::fs::copy(entry.path(), update_dir().join(entry.file_name()));
                }
            }
            // CREATE_NO_WINDOW: it is a console program, and shows none.
            std::process::Command::new(copy).arg("update").arg(&setup).arg(&manifest.version).arg(&app).creation_flags(0x0800_0000).spawn()?;
            // Which way an update went in is in the log, either way.
            eprintln!("zenith-clips: the update to {} goes in behind Zenith's own window (see update.log)", manifest.version);
            Ok(())
        };
        let started = styled().or_else(|why| {
            eprintln!("zenith-clips: the update is shown by the setup program itself: {why}");
            std::process::Command::new(&setup).args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]).spawn().map(|_| ())
        });
        if let Err(e) = started {
            eprintln!("zenith-clips: the new version could not be started: {e}");
        }
    }

    #[cfg(not(windows))]
    fn restart(&self) {
        if self.rust().state != State::Ready {
            return;
        }
        let Some(exe) = install::current_exe() else { return };
        // The recorder is replaced too: have it start as the new one. Then,
        // once this window is gone, open the new one.
        let script = "systemctl --user try-restart zenithd.service 2>/dev/null; while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; exec \"$2\"";
        let started = std::process::Command::new("sh")
            .args(["-c", script, "sh", &std::process::id().to_string()])
            .arg(&exe)
            .stdin(std::process::Stdio::null())
            .spawn();
        if let Err(e) = started {
            eprintln!("zenith-clips: the new version could not be started: {e}");
        }
    }

    fn started_well(&self) {
        if let Some(dir) = install::current_exe().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) {
            install::started_well(&dir);
        }
    }
}
