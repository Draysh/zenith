//! Starting the recorder when the user signs in to Windows: a value under
//! the user's own `Run` key, which the setup program writes and the
//! settings can take away or put back. Elsewhere the desktop's service
//! manager starts the recorder, and this has nothing to do.

use std::path::Path;

/// The name of the value under the `Run` key.
pub const NAME: &str = "Zenith";
#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// What the value holds: the app, asked to start the recorder and nothing
/// else. The app has no console window, so neither does the recorder it
/// starts.
pub fn command(app: &Path) -> String {
    format!("\"{}\" --start-recorder", app.display())
}

/// Whether the recorder is started at sign-in.
pub fn enabled() -> bool {
    #[cfg(windows)]
    {
        crate::paths::command("reg").args(["query", RUN_KEY, "/v", NAME]).output().is_ok_and(|out| out.status.success())
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Have the recorder start at sign-in, or not. `app` is the app's program.
pub fn set(on: bool, app: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut reg = crate::paths::command("reg");
        if on {
            reg.args(["add", RUN_KEY, "/v", NAME, "/t", "REG_SZ", "/d", &command(app), "/f"]);
        } else if !enabled() {
            return Ok(());
        } else {
            reg.args(["delete", RUN_KEY, "/v", NAME, "/f"]);
        }
        match reg.output() {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => Err(format!("Windows would not change it: {}", String::from_utf8_lossy(&out.stderr).trim())),
            Err(e) => Err(format!("Windows would not change it: {e}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (on, app);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_is_asked_to_start_the_recorder_and_nothing_else() {
        let app = Path::new(r"C:\Users\a b\AppData\Local\Programs\Zenith\zenith-clips.exe");
        assert_eq!(command(app), r#""C:\Users\a b\AppData\Local\Programs\Zenith\zenith-clips.exe" --start-recorder"#);
    }
}
