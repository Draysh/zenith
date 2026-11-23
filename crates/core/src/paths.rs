//! Where things live: the XDG base directories on Linux, the user's
//! application data folders on Windows.

use std::path::PathBuf;
use std::process::Command;

#[cfg(not(windows))]
const APP: &str = "zenith-clips";
#[cfg(windows)]
const APP: &str = "Zenith";

/// The user's home folder.
pub fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// The folder an environment variable names, or `fallback` under home.
fn folder(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

/// The settings: `~/.config/zenith-clips/config.toml`, or
/// `%APPDATA%\Zenith\config.toml`.
pub fn config_file() -> PathBuf {
    if cfg!(windows) { folder("APPDATA", "AppData/Roaming") } else { folder("XDG_CONFIG_HOME", ".config") }.join(APP).join("config.toml")
}

/// The library index: `~/.local/share/zenith-clips`, or
/// `%LOCALAPPDATA%\Zenith`.
pub fn data_dir() -> PathBuf {
    if cfg!(windows) { folder("LOCALAPPDATA", "AppData/Local") } else { folder("XDG_DATA_HOME", ".local/share") }.join(APP)
}

/// Thumbnails, filmstrips, waveforms: `~/.cache/zenith-clips`, or
/// `%LOCALAPPDATA%\Zenith\cache`.
pub fn cache_dir() -> PathBuf {
    if cfg!(windows) { data_dir().join("cache") } else { folder("XDG_CACHE_HOME", ".cache").join(APP) }
}

/// The folder the socket is in when there is no runtime directory: one of
/// this user's own in the temporary directory, which the daemon keeps
/// closed to everyone else.
#[cfg(unix)]
pub fn fallback_runtime_dir() -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
    std::env::temp_dir().join(format!("{APP}-{uid}"))
}

/// The daemon's control socket: in the runtime directory, or where
/// `ZENITH_SOCKET` says. On Windows it is a named pipe of this user's.
pub fn socket() -> PathBuf {
    if let Some(path) = std::env::var_os("ZENITH_SOCKET") {
        return PathBuf::from(path);
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        PathBuf::from(format!(r"\\.\pipe\zenith-clips-{user}"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(fallback_runtime_dir)
            .join(format!("{APP}.sock"))
    }
}

/// A program by name: the copy beside this program when there is one (the
/// ones Zenith comes with on Windows), else whichever is found by name.
pub fn program(name: &str) -> PathBuf {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(&file)))
        .filter(|beside| beside.is_file())
        .unwrap_or_else(|| PathBuf::from(file))
}

/// A command for the program `name` ([`program`]). On Windows it opens no
/// console window of its own, which would flash up over the app.
pub fn command(name: &str) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program(name));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        command.creation_flags(0x0800_0000);
    }
    command
}

/// The default clip folder: the user's Videos directory, `Clips` inside.
pub fn default_clip_dir() -> PathBuf {
    if cfg!(windows) {
        return home().join("Videos").join("Clips");
    }
    let videos = std::fs::read_to_string(folder("XDG_CONFIG_HOME", ".config").join("user-dirs.dirs"))
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                let v = l.trim().strip_prefix("XDG_VIDEOS_DIR=")?.trim_matches('"');
                Some(PathBuf::from(v.replace("$HOME", &home().to_string_lossy())))
            })
        })
        .unwrap_or_else(|| home().join("Videos"));
    videos.join("Clips")
}
