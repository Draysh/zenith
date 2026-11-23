//! The settings for the screens: read, changed, written, and the recorder
//! told to pick them up.

use zenith_core::config::Config;
use zenith_core::ipc::{self, IpcError, Request, Response};
use zenith_core::recorder::{self, outputs};
use zenith_core::paths;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{Value, json};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("zenith/cpp/helpers.h");
        #[namespace = "cct"]
        #[cxx_name = "copyText"]
        fn copy_text_to_clipboard(text: &QString);
        #[namespace = "cct"]
        #[cxx_name = "useIcon"]
        fn use_window_icon(tile: &QString, ink: &QString, dot: &QString);
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Settings = super::SettingsRust;

        /// The settings as JSON, in the shape of the settings file.
        #[qinvokable]
        fn load(self: &Settings) -> QString;
        /// What this machine offers: `{monitors: [{name, width, height}],
        /// codecs: [..], recorder: bool, known: bool}`. Finding out asks
        /// the recorder program, which takes a moment: until it has
        /// answered, `known` is false and the lists are empty, and
        /// `machineReady` says when to ask again.
        #[qinvokable]
        fn machine(self: Pin<&mut Settings>) -> QString;
        #[qsignal]
        fn machine_ready(self: Pin<&mut Settings>);
        /// What the settings in `json` come to: `{megabytes, width, height,
        /// kbps, tracks}` for the monitor they would record.
        #[qinvokable]
        fn estimate(self: &Settings, json: &QString) -> QString;
        /// Write the settings and have the recorder use them. Returns the
        /// settings as they were kept (values out of range are brought in),
        /// with `error` set when they could not be written. Telling the
        /// recorder happens afterwards; `reloadFailed` says when it refused.
        #[qinvokable]
        fn save(self: Pin<&mut Settings>, json: &QString) -> QString;
        #[qsignal]
        fn reload_failed(self: Pin<&mut Settings>, message: QString);
        #[qinvokable]
        fn copy_text(self: &Settings, text: &QString);
        /// The command to bind a key to. The recorder's full path when its
        /// folder is not one the desktop looks for programs in.
        #[qinvokable]
        fn save_command(self: &Settings) -> QString;
        /// What is wrong with a key combination as a clip key (`Alt+F9`);
        /// empty when it can be one. Whether another program already holds
        /// it is the recorder's to find out, once it is set.
        #[qinvokable]
        fn hotkey_problem(self: &Settings, combination: &QString) -> QString;
        /// The programs running now, to pick a game from: `[{program,
        /// title}]`, the title being the window's where the system tells.
        #[qinvokable]
        fn running_programs(self: &Settings) -> QString;
        /// Whether the recorder starts when the user signs in (Windows;
        /// elsewhere the desktop's service manager sees to it).
        #[qinvokable]
        fn starts_at_sign_in(self: &Settings) -> bool;
        #[qinvokable]
        fn set_starts_at_sign_in(self: &Settings, on: bool) -> QString;
        /// Is everything in place to clip? Looking takes a moment, so
        /// `doctorReady` brings the answer: `{worst: "ok"|"warning"|"problem",
        /// checks: [{name, level, detail, fix}]}`.
        #[qinvokable]
        fn doctor_start(self: Pin<&mut Settings>);
        #[qsignal]
        fn doctor_ready(self: Pin<&mut Settings>, report: QString);
        /// Put a report of how things stand on the clipboard, for pasting
        /// to whoever is helping: versions, the setup check, what the
        /// recorder is doing and its last log lines. `reportCopied` follows.
        #[qinvokable]
        fn copy_report(self: Pin<&mut Settings>);
        #[qsignal]
        fn report_copied(self: Pin<&mut Settings>);
        /// Whether the welcome screen has yet to be seen through.
        #[qinvokable]
        fn first_run(self: &Settings) -> bool;
        #[qinvokable]
        fn welcome_done(self: &Settings);
        /// Show the welcome screen at the next start (or now: Main loads it).
        #[qinvokable]
        fn welcome_again(self: &Settings);
        /// Whether a one-time hint (`"first-clip"`) has been seen through.
        #[qinvokable]
        fn hint_seen(self: &Settings, name: &QString) -> bool;
        #[qinvokable]
        fn hint_done(self: &Settings, name: &QString);
        /// Start the background recorder: through the user's service
        /// manager where it has Zenith's unit, else as a program of its
        /// own. Returns why it could not be, empty when it was begun (it
        /// takes a moment to be up: check again after).
        #[qinvokable]
        fn start_recorder(self: &Settings) -> QString;
        /// The games Zenith can tell are running without being told:
        /// `{count, names: [up to 3], from: "Steam" | "Steam, Epic, ..."}`.
        #[qinvokable]
        fn games_found(self: &Settings) -> QString;
        /// The app's icon in the colours the app is in: the tile, the Z
        /// on it and the dot, each "#RRGGBB". The window's icon at once;
        /// on Linux also the icon the desktop shows for the app (its
        /// launcher, its dock), kept in the user's own icons.
        #[qinvokable]
        fn use_icon(self: &Settings, tile: &QString, ink: &QString, dot: &QString);
    }

    impl cxx_qt::Threading for Settings {}
}

/// How many saves are still on their way to the recorder.
static TELLING: AtomicUsize = AtomicUsize::new(0);

/// One of them, done when it goes.
struct Told;

impl Drop for Told {
    fn drop(&mut self) {
        TELLING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Wait, up to `most`, until the recorder has been told of every save: for
/// when the program is leaving right after settings were kept, which must
/// not cut the telling off. (A recorder that takes longer than that to
/// answer has the request by then, and reads the file either way.)
pub fn wait_for_recorder(most: Duration) {
    let until = std::time::Instant::now() + most;
    while TELLING.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Default)]
pub struct SettingsRust {
    /// The monitors, codecs and sound devices, once the recorder program
    /// has listed them.
    machine: Option<(Vec<outputs::Output>, Vec<String>, Vec<outputs::AudioDevice>)>,
    asked: bool,
}

impl SettingsRust {
    fn monitors(&self) -> &[outputs::Output] {
        self.machine.as_ref().map_or(&[], |(monitors, _, _)| monitors.as_slice())
    }
}

fn as_json(cfg: &Config) -> Value {
    serde_json::to_value(cfg).unwrap_or_default()
}

impl ffi::Settings {
    fn load(&self) -> QString {
        // Every value that can be read: one that cannot must not put the
        // rest back to their defaults, which the next change would save.
        let cfg = match Config::load_lenient(&paths::config_file()) {
            Ok((cfg, ignored)) => {
                if !ignored.is_empty() {
                    eprintln!("zenith-clips: settings not understood, left at their defaults: {}", ignored.join(", "));
                }
                cfg
            }
            Err(e) => {
                eprintln!("zenith-clips: {e}");
                Config::default()
            }
        };
        QString::from(&as_json(&cfg).to_string())
    }

    fn machine(mut self: Pin<&mut Self>) -> QString {
        let Some((monitors, codecs, devices)) = &self.machine else {
            if !self.asked {
                self.as_mut().rust_mut().asked = true;
                let qt = self.qt_thread();
                std::thread::spawn(move || {
                    let found = (outputs::list(recorder::PROGRAM), outputs::codecs(recorder::PROGRAM), outputs::audio_devices(recorder::PROGRAM));
                    let _ = qt.queue(move |mut settings| {
                        settings.as_mut().rust_mut().machine = Some(found);
                        settings.machine_ready();
                    });
                });
            }
            // Not known yet is not "no recorder": the page says nothing until it is.
            return QString::from(&json!({ "monitors": [], "codecs": [], "speakers": [], "microphones": [], "recorder": true, "known": false }).to_string());
        };
        let monitors: Vec<Value> = monitors.iter().map(|m| json!({ "name": m.name, "width": m.size.0, "height": m.size.1 })).collect();
        // What plays (speakers, headphones) apart from what listens (microphones).
        let listed = |input: bool| -> Vec<Value> {
            devices
                .iter()
                .filter(|d| d.is_input() == input)
                .map(|d| {
                    let label = match d.name.as_str() {
                        outputs::DEFAULT_OUTPUT => "The default speakers or headphones",
                        outputs::DEFAULT_INPUT => "The default microphone",
                        _ => d.plain_label(),
                    };
                    json!({ "name": d.name, "label": label })
                })
                .collect()
        };
        let value = json!({
            "monitors": monitors, "codecs": codecs, "speakers": listed(false), "microphones": listed(true),
            "recorder": !codecs.is_empty(), "known": true,
        });
        QString::from(&value.to_string())
    }

    fn estimate(&self, json: &QString) -> QString {
        let Ok(cfg) = serde_json::from_str::<Config>(&json.to_string()) else { return QString::from("{}") };
        let (_, native) = outputs::resolve(&cfg.capture.source, self.monitors());
        let (width, height) = cfg.capture.resolution.on(native);
        let tracks = recorder::tracks(&cfg).len() as u32;
        let value = json!({
            "megabytes": cfg.capture.buffer_megabytes(native, tracks),
            "width": width,
            "height": height,
            "kbps": cfg.capture.bitrate_kbps(native),
            "tracks": tracks,
        });
        QString::from(&value.to_string())
    }

    fn doctor_start(self: Pin<&mut Self>) {
        use zenith_core::doctor::{assess, gather, worst};
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let cfg = Config::load_usable(&paths::config_file());
            let checks = assess(&gather(&cfg, recorder::PROGRAM, &paths::socket()), &cfg);
            let report = json!({ "worst": worst(&checks), "checks": checks }).to_string();
            let _ = qt.queue(move |settings| settings.doctor_ready(QString::from(&report)));
        });
    }

    fn copy_report(self: Pin<&mut Self>) {
        use zenith_core::doctor::{assess, gather};
        use zenith_core::{install, report};
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let cfg = Config::load_usable(&paths::config_file());
            let checks = assess(&gather(&cfg, recorder::PROGRAM, &paths::socket()), &cfg);
            let status = match ipc::ask(&paths::socket(), &Request::Status, Duration::from_secs(2)) {
                Ok(Response::Status(status)) => Some(status),
                _ => None,
            };
            let text = report::text(&report::Report {
                version: env!("CARGO_PKG_VERSION"),
                system: report::system(),
                installed: install::here().word(),
                status: status.as_ref(),
                checks: &checks,
                monitors: &outputs::list(recorder::PROGRAM),
                codecs: &outputs::codecs(recorder::PROGRAM),
                log: &report::log_tail(80),
                window: &report::window_log(150),
            });
            // The clipboard is the window's: set from its own thread.
            let _ = qt.queue(move |settings| {
                ffi::copy_text_to_clipboard(&QString::from(&text));
                settings.report_copied();
            });
        });
    }

    fn first_run(&self) -> bool {
        !paths::data_dir().join("welcomed").exists()
    }

    fn use_icon(&self, tile: &QString, ink: &QString, dot: &QString) {
        let colours = [tile.to_string(), ink.to_string(), dot.to_string()];
        let is_colour = |c: &String| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|d| d.is_ascii_hexdigit());
        if !colours.iter().all(is_colour) {
            return;
        }
        ffi::use_window_icon(tile, ink, dot);
        #[cfg(target_os = "linux")]
        keep_desktop_icon(&colours);
    }

    fn welcome_done(&self) {
        let marker = paths::data_dir().join("welcomed");
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(marker, b"");
    }

    fn welcome_again(&self) {
        let _ = std::fs::remove_file(paths::data_dir().join("welcomed"));
    }

    fn hint_seen(&self, name: &QString) -> bool {
        hint_marker(&name.to_string()).is_some_and(|marker| marker.exists())
    }

    fn hint_done(&self, name: &QString) {
        let Some(marker) = hint_marker(&name.to_string()) else { return };
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(marker, b"");
    }

    fn start_recorder(&self) -> QString {
        QString::from(&start_recorder().err().unwrap_or_default())
    }

    fn games_found(&self) -> QString {
        use zenith_core::games::{launchers, steam};
        let mut names: Vec<String> = steam::installed(&steam::roots(&paths::home())).into_iter().map(|g| g.name).collect();
        let from_steam = !names.is_empty();
        let others: Vec<String> = launchers::installed().into_iter().map(|g| g.name).collect();
        names.extend(others.iter().cloned());
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        let from = if cfg!(windows) { "Steam, Epic, GOG, Ubisoft, Battle.net, the EA app, Riot, Rockstar and Xbox" } else { "Steam" };
        let value = json!({ "count": names.len(), "names": names.iter().take(3).collect::<Vec<_>>(), "from": from, "steam": from_steam });
        QString::from(&value.to_string())
    }

    fn hotkey_problem(&self, combination: &QString) -> QString {
        let wrong = combination.to_string().parse::<zenith_core::hotkey::Hotkey>().err().unwrap_or_default();
        QString::from(&wrong)
    }

    fn running_programs(&self) -> QString {
        let programs: Vec<Value> =
            zenith_core::games::programs::running().into_iter().map(|r| json!({ "program": r.program, "title": r.title })).collect();
        QString::from(&Value::Array(programs).to_string())
    }

    fn starts_at_sign_in(&self) -> bool {
        zenith_core::autostart::enabled()
    }

    /// An error message, empty when it worked.
    fn set_starts_at_sign_in(&self, on: bool) -> QString {
        let app = zenith_core::install::current_exe().unwrap_or_else(|| paths::program("zenith-clips"));
        QString::from(&zenith_core::autostart::set(on, &app).err().unwrap_or_default())
    }

    fn copy_text(&self, text: &QString) {
        ffi::copy_text_to_clipboard(text);
    }

    fn save_command(&self) -> QString {
        let beside = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("zenithd"))).filter(|p| p.is_file());
        let found = |dir: &std::path::Path| std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d == dir));
        let program = match beside {
            Some(file) if !file.parent().is_some_and(found) => {
                let file = file.to_string_lossy().into_owned();
                if file.contains(' ') { format!("\"{file}\"") } else { file }
            }
            _ => "zenithd".into(),
        };
        QString::from(&format!("{program} save"))
    }

    fn save(self: Pin<&mut Self>, json: &QString) -> QString {
        let mut cfg = match serde_json::from_str::<Config>(&json.to_string()) {
            Ok(cfg) => cfg,
            Err(e) => return QString::from(&json!({ "error": format!("These settings are not valid: {e}") }).to_string()),
        };
        cfg.clamp();
        let mut out = as_json(&cfg);
        if let Err(e) = cfg.save(&paths::config_file()) {
            out["error"] = json!(e.to_string());
            return QString::from(&out.to_string());
        }
        // The recorder can take seconds to answer (it restarts capture).
        let qt = self.qt_thread();
        TELLING.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            // Counted until the recorder has answered or been given up on.
            let _told = Told;
            let problem = match ipc::ask(&paths::socket(), &Request::Reload, Duration::from_secs(5)) {
                Ok(Response::Error { message }) => message,
                // Not running: it reads the file when it starts.
                Ok(_) | Err(IpcError::NotRunning) => return,
                Err(e) => e.to_string(),
            };
            let _ = qt.queue(move |settings| settings.reload_failed(QString::from(&problem)));
        });
        QString::from(&out.to_string())
    }
}

/// The icon the desktop shows for the app, drawn in `[tile, ink, dot]` and
/// kept in the user's own icons, which the desktop looks in before the
/// system's. Zenith's own colours need no file there: one is written for
/// them only over one that is there already (an install by hand puts its
/// icon in the same place).
#[cfg(target_os = "linux")]
fn keep_desktop_icon(colours: &[String; 3]) {
    const MARK: &str = include_str!("../../icons/zenith.svg");
    const OWN: [&str; 3] = ["#3DE0F5", "#02161A", "#FF3DA5"];
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share")));
    let Some(data) = data else { return };
    let theme = data.join("icons/hicolor");
    let file = theme.join("scalable/apps/zenith-clips.svg");
    let is_own = colours.iter().zip(OWN).all(|(c, own)| c.eq_ignore_ascii_case(own));
    if is_own && !file.exists() {
        return;
    }
    // The three at once, by a stand-in each: one colour may be another's.
    let mut drawn = MARK.to_string();
    for (n, own) in OWN.iter().enumerate() {
        drawn = drawn.replace(own, &format!("@{n}@"));
    }
    for (n, colour) in colours.iter().enumerate() {
        drawn = drawn.replace(&format!("@{n}@"), &colour.to_uppercase());
    }
    if std::fs::read_to_string(&file).is_ok_and(|kept| kept == drawn) {
        return;
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&file, drawn).is_ok() {
        // A desktop that remembers its icons looks again at a folder that
        // has changed.
        let _ = std::fs::File::open(&theme).and_then(|dir| dir.set_modified(std::time::SystemTime::now()));
    }
}

/// Where a one-time hint is marked as seen. Only plain names: a name is
/// a file's.
fn hint_marker(name: &str) -> Option<std::path::PathBuf> {
    let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    plain.then(|| paths::data_dir().join("hints").join(name))
}

/// Begin the background recorder. On Linux through systemd when the user
/// has Zenith's unit (so it also starts with the next session), else, and
/// on Windows, as a program of its own without a window.
fn start_recorder() -> Result<(), String> {
    use std::process::Stdio;
    if ipc::ask(&paths::socket(), &Request::Status, Duration::from_millis(500)).is_ok() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let has_unit = std::process::Command::new("systemctl")
            .args(["--user", "cat", "zenithd.service"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if has_unit {
            let out = std::process::Command::new("systemctl")
                .args(["--user", "enable", "--now", "zenithd.service"])
                .stdin(Stdio::null())
                .output()
                .map_err(|e| format!("The recorder could not be started: {e}"))?;
            if out.status.success() {
                return Ok(());
            }
            let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(format!("The recorder could not be started: {said}"));
        }
    }
    paths::command("zenithd")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("The recorder could not be started: {e}"))
}
