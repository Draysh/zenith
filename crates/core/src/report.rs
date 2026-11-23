//! A report of how things stand, to paste to whoever is helping: versions,
//! what the setup check found, what the recorder is doing, and the last
//! things it logged. It holds nothing but that: no clip, and nothing from
//! other programs. The log lines do name the clips that were saved and the
//! game that was running.

use crate::doctor::{Check, Level};
use crate::ipc::Status;
use crate::recorder::outputs::Output;

pub struct Report<'a> {
    pub version: &'a str,
    pub system: String,
    /// How this copy is installed: `portable`, `packaged`, `development`.
    pub installed: &'a str,
    /// The recorder's answer, when it gave one.
    pub status: Option<&'a Status>,
    pub checks: &'a [Check],
    pub monitors: &'a [Output],
    pub codecs: &'a [String],
    /// The recorder's last log lines, oldest first.
    pub log: &'a [String],
    /// What the window's own log says: how its last starts ended, and
    /// its last lines (`window_log`).
    pub window: &'a [String],
}

/// The operating system, as it names itself.
pub fn system() -> String {
    #[cfg(windows)]
    {
        let ver = crate::paths::command("cmd").args(["/c", "ver"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        ver.filter(|v| !v.is_empty()).unwrap_or_else(|| "Windows".into())
    }
    #[cfg(not(windows))]
    {
        let name = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| text.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
            .unwrap_or_else(|| std::env::consts::OS.to_string());
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|k| k.trim().to_string()).unwrap_or_default();
        if kernel.is_empty() { name } else { format!("{name}, kernel {kernel}") }
    }
}

/// A log file's text with the one put aside before it in front: a log
/// that was just started anew still has what led up to now.
fn with_old(path: &std::path::Path) -> String {
    let read = |path: &std::path::Path| std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default();
    read(&crate::applog::old(path)) + &read(path)
}

/// What the window's own log says (see `applog`): how each of its last
/// starts ended, and its last `lines` lines, which reach back over the
/// starts before this one.
pub fn window_log(lines: usize) -> Vec<String> {
    crate::applog::for_report(&with_old(&crate::applog::file()), lines)
}

/// The recorder's last `lines` log lines: from its log files on Windows,
/// from the journal where it runs as a service.
pub fn log_tail(lines: usize) -> Vec<String> {
    let last = |text: &str| -> Vec<String> {
        let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        all[all.len().saturating_sub(lines)..].iter().map(|l| l.to_string()).collect()
    };
    #[cfg(windows)]
    {
        let dir = crate::paths::data_dir().join("logs");
        let mut out = Vec::new();
        for file in ["zenithd.log", "zenith-rec.log"] {
            let text = with_old(&dir.join(file));
            if !text.is_empty() {
                out.push(format!("-- {file}"));
                out.extend(last(&text));
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        let journal = std::process::Command::new("journalctl")
            .args(["--user", "-u", "zenithd", "-o", "cat", "--no-pager", "-n", &lines.to_string()])
            .stdin(std::process::Stdio::null())
            .output();
        journal.ok().map(|o| last(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
    }
}

/// The report as text.
pub fn text(r: &Report) -> String {
    let mut out = vec![format!("Zenith {} ({}), {}", r.version, r.installed, r.system)];
    out.push(String::new());
    out.push("Recorder".into());
    match r.status {
        None => out.push("  not running, or it did not answer".into()),
        Some(s) => {
            out.push(format!("  state: {}{}", s.state, s.detail.as_ref().map(|d| format!(" ({d})")).unwrap_or_default()));
            out.push(format!("  game: {}", s.game.as_deref().unwrap_or("none")));
            if let Some(way) = &s.recording {
                out.push(format!("  recording with: {way}"));
            }
            out.push(format!("  buffer: {} s, clips saved: {}", s.buffer_secs, s.clips_saved));
        }
    }
    out.push(String::new());
    out.push("Setup check".into());
    for check in r.checks {
        let mark = match check.level {
            Level::Ok => "ok",
            Level::Warning => "WARNING",
            Level::Problem => "PROBLEM",
        };
        out.push(format!("  [{mark}] {}: {}{}", check.name, check.detail, if check.fix.is_empty() { String::new() } else { format!(" {}", check.fix) }));
    }
    out.push(String::new());
    let monitors: Vec<String> = r.monitors.iter().map(|m| format!("{} {}x{}", m.name, m.size.0, m.size.1)).collect();
    out.push(format!("Monitors: {}", if monitors.is_empty() { "none listed".into() } else { monitors.join(", ") }));
    out.push(format!("Encoders: {}", if r.codecs.is_empty() { "none listed".into() } else { r.codecs.join(", ") }));
    out.push(String::new());
    out.push("Last log lines".into());
    if r.log.is_empty() {
        out.push("  none found".into());
    }
    out.extend(r.log.iter().map(|l| format!("  {l}")));
    out.push(String::new());
    out.push("The window (zenith-clips), its last starts and log lines".into());
    if r.window.is_empty() {
        out.push("  none found".into());
    }
    out.extend(r.window.iter().map(|l| format!("  {l}")));
    without_home(&out.join("\n"), &crate::paths::home().to_string_lossy())
}

/// The report with the user's home folder written `~`: its paths are in
/// the log lines and the checks, and carry the user's name, which whoever
/// is helping has no need of.
fn without_home(text: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    if home.len() < 3 {
        return text.to_string();
    }
    // Windows writes its paths with either slash.
    text.replace(home, "~").replace(&home.replace('\\', "/"), "~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_folder_is_not_named() {
        assert_eq!(without_home("saved /home/ada/Videos/Clips/a.mp4", "/home/ada"), "saved ~/Videos/Clips/a.mp4");
        assert_eq!(without_home(r"C:\Users\Ada\Videos and C:/Users/Ada/Videos", r"C:\Users\Ada"), r"~\Videos and ~/Videos");
        assert_eq!(without_home("/ is the root", "/"), "/ is the root");
    }

    #[test]
    fn a_report_says_how_things_stand_in_plain_text() {
        let status = Status {
            state: "capturing".into(),
            game: Some("STRAFE".into()),
            buffer_secs: 60,
            clips_saved: 2,
            recording: Some("gdigrab -> libx264 (frames downloaded)".into()),
            ..Default::default()
        };
        let checks = [
            Check { name: "Recorder", level: Level::Ok, detail: "zenith-rec is installed.".into(), fix: String::new() },
            Check { name: "Save hotkey", level: Level::Warning, detail: "Alt+F9 is taken.".into(), fix: "Pick another key.".into() },
        ];
        let monitors = [Output { name: "DISPLAY1".into(), size: (2560, 1440) }];
        let log = ["recorder started".to_string(), "clip saved".to_string()];
        let text = text(&Report {
            version: "0.4.0",
            system: "Windows 11".into(),
            installed: "portable",
            status: Some(&status),
            checks: &checks,
            monitors: &monitors,
            codecs: &["h264".to_string()],
            log: &log,
            window: &["0.4.0 started 2026-03-04 05:06:07: DID NOT CLOSE (it crashed, or was ended from outside)".to_string()],
        });
        assert!(text.starts_with("Zenith 0.4.0 (portable), Windows 11\n"), "{text}");
        for wanted in [
            "  state: capturing\n  game: STRAFE\n  recording with: gdigrab -> libx264 (frames downloaded)\n  buffer: 60 s, clips saved: 2",
            "  [ok] Recorder: zenith-rec is installed.\n  [WARNING] Save hotkey: Alt+F9 is taken. Pick another key.",
            "Monitors: DISPLAY1 2560x1440\nEncoders: h264",
            "Last log lines\n  recorder started\n  clip saved",
            "The window (zenith-clips), its last starts and log lines\n  0.4.0 started 2026-03-04 05:06:07: DID NOT CLOSE",
        ] {
            assert!(text.contains(wanted), "missing {wanted:?} in:\n{text}");
        }
    }

    #[test]
    fn a_report_with_nothing_to_go_on_still_reads() {
        let text = text(&Report { version: "0.4.0", system: "Linux".into(), installed: "packaged", status: None, checks: &[], monitors: &[], codecs: &[], log: &[], window: &[] });
        assert!(text.contains("not running, or it did not answer") && text.contains("Monitors: none listed") && text.matches("  none found").count() == 2, "{text}");
    }
}
