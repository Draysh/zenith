//! Telling the user what happened: a sound and a desktop notification.
//! Neither may ever hold the daemon up, so both run on their own and their
//! failures are only logged.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SAVED_WAV: &[u8] = include_bytes!("../assets/saved.wav");

/// The save sound as a file a player can be pointed at.
#[cfg(unix)]
fn sound_file() -> Option<PathBuf> {
    let path = zenith_core::paths::cache_dir().join("saved.wav");
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(SAVED_WAV.len() as u64) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, SAVED_WAV).ok()?;
    }
    Some(path)
}

#[cfg(unix)]
pub fn play_saved() {
    let Some(file) = sound_file() else { return };
    std::thread::spawn(move || {
        for player in ["pw-play", "paplay"] {
            let run = Command::new(player).arg(&file).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
            if run.is_ok_and(|s| s.success()) {
                return;
            }
        }
        crate::warn("zenithd: no sound player (pw-play, paplay) could play the save sound");
    });
}

/// Play the save sound. Windows plays a sound held in memory itself, with
/// no player program and no file; it returns at once and plays on.
#[cfg(windows)]
pub fn play_saved() {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    // SAFETY: with SND_MEMORY the first argument is the sound itself, not a
    // name, and with SND_ASYNC it has to stay where it is while it plays:
    // it is part of this program, so it does.
    let played = unsafe { PlaySoundW(SAVED_WAV.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
    if played == 0 {
        crate::warn("zenithd: the save sound could not be played");
    }
}

/// The name Windows knows Zenith's notifications by: its AppUserModelID.
/// The installer's Start Menu shortcut carries the same one, which is
/// where Windows takes the name and the icon it shows on a toast from.
#[cfg(windows)]
pub const APP_ID: &str = "Zenith.Clips";

/// Show a notification as Zenith's.
#[cfg(unix)]
fn show(n: &mut notify_rust::Notification) -> notify_rust::error::Result<notify_rust::NotificationHandle> {
    n.appname("Zenith").icon("zenith-clips").show()
}

/// Show a notification as Zenith's. Windows only lets a program that has
/// a Start Menu shortcut with its id say so; run without one (not
/// installed), the notification is shown the way it was before there was
/// an id, under PowerShell's name, rather than not at all.
#[cfg(windows)]
fn show(n: &mut notify_rust::Notification) -> notify_rust::error::Result<notify_rust::NotificationHandle> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static TOLD: AtomicBool = AtomicBool::new(false);
    let unnamed = n.clone();
    n.app_id(APP_ID).show().or_else(|e| {
        if !TOLD.swap(true, Ordering::Relaxed) {
            crate::warn(format_args!("zenithd: notifications cannot be shown as {APP_ID} (is Zenith installed, with its Start Menu shortcut?): {e}"));
        }
        unnamed.show()
    })
}

fn notify(summary: String, body: String, urgent: bool) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.summary(&summary).body(&body).timeout(if urgent { 8000 } else { 4000 });
        if let Err(e) = show(&mut n) {
            crate::warn(format_args!("zenithd: notification failed: {e}"));
        }
    })
}

/// Start a program for the user: the app, the file manager. As a service,
/// everything this process starts is stopped with it, so the program is
/// put in a scope of its own, where it outlives a restart of the recorder.
/// (Windows stops nothing with the daemon: there it is simply started.)
pub fn launch(program: &std::ffi::OsStr, args: &[&std::ffi::OsStr]) {
    let mut command = if cfg!(unix) && std::env::var_os("INVOCATION_ID").is_some() {
        let mut scoped = Command::new("systemd-run");
        scoped.args(["--user", "--scope", "--quiet", "--collect", "--"]).arg(program);
        scoped
    } else {
        Command::new(program)
    };
    match command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        // Waited for, so that nothing of it is left behind when it ends.
        Ok(mut child) => drop(std::thread::spawn(move || {
            let _ = child.wait();
        })),
        Err(e) => crate::warn(format_args!("zenithd: {} could not be started: {e}", program.to_string_lossy())),
    }
}

/// The app, when it is installed beside this program or on the path.
fn app() -> PathBuf {
    zenith_core::paths::program("zenith-clips")
}

/// Open the app's window.
pub fn open_app() {
    launch(app().as_os_str(), &[]);
}

/// Show a folder in the file manager, made first if it is not there yet.
pub fn open_folder(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let file_manager = if cfg!(windows) { "explorer" } else { "xdg-open" };
    // Explorer opens its own default folder for a path written with
    // forward slashes, which is how a folder picked in the app is kept.
    let dir = if cfg!(windows) { PathBuf::from(dir.to_string_lossy().replace('/', "\\")) } else { dir.to_path_buf() };
    launch(file_manager.as_ref(), &[dir.as_os_str()]);
}

/// Say a clip was saved. Clicking the notification, or its Edit button,
/// opens the clip in the app. (On Windows only the button does: a click on
/// the toast itself is not told apart from it going away.)
///
/// On Windows, while a game fills the screen, the notification is kept
/// back until the game is left: a notice of Zenith's own is drawn over
/// the game as well (see `overlay`), to say so there and then. The
/// notification is still sent, and waits in Windows' notification centre
/// with its Edit button for after the game.
///
/// `seconds` is how long the clip is, when the file could be read.
pub fn clip_saved(game: &str, seconds: Option<f64>, clip: &Path) {
    let body = match seconds {
        Some(seconds) => format!("{game} · {} s", seconds.round() as u32),
        None => format!("{game} · saved, but the file could not be checked"),
    };
    #[cfg(windows)]
    if crate::overlay::covers_screen() {
        crate::overlay::show("Clip saved", &body);
    }
    let clip = clip.to_path_buf();
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.summary("Clip saved").body(&body).timeout(5000);
        // Linux desktops take "default" as the click on the notification
        // itself; Windows would show it as a second button.
        #[cfg(unix)]
        n.action("default", "Edit");
        n.action("edit", "Edit");
        match show(&mut n) {
            // Waits until the notification is acted on or goes away.
            Ok(shown) => shown.wait_for_action(|action| {
                if matches!(action, "default" | "edit") {
                    launch(app().as_os_str(), &["--open".as_ref(), clip.as_os_str()]);
                }
            }),
            Err(e) => crate::warn(format_args!("zenithd: notification failed: {e}")),
        }
    });
}

/// Returns the thread showing it, for a caller that is about to exit.
pub fn problem(summary: &str, detail: &str) -> std::thread::JoinHandle<()> {
    notify(summary.into(), detail.into(), true)
}
