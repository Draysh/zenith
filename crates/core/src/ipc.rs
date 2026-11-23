//! What the daemon is asked over its socket: one JSON request on a line,
//! one JSON response on a line.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    /// Save the replay buffer as a clip (the shortcut).
    Save,
    Status,
    /// Capture on, off, or decided by the settings (`None`).
    Capture { force: Option<bool> },
    /// Read the settings file again.
    Reload,
    Quit,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum Response {
    Ok,
    /// The clip was written here.
    Saved { path: PathBuf },
    Status(Status),
    Error { message: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Status {
    /// `idle`, `capturing`, `saving`, `restarting` or `failed`.
    pub state: String,
    /// Why it failed, when it has.
    pub detail: Option<String>,
    pub game: Option<String>,
    pub forced: Option<bool>,
    pub buffer_secs: u32,
    pub clips_saved: u32,
    /// How it is recording, while it is: the capture method and encoder.
    #[serde(default)]
    pub recording: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("the Zenith daemon is not running")]
    NotRunning,
    #[error("the daemon did not answer: {0}")]
    Io(#[from] std::io::Error),
    #[error("the daemon's answer was not understood: {0}")]
    Protocol(#[from] serde_json::Error),
}

/// The longest answer that is read: the daemon's are a line of a few
/// hundred bytes, and whatever sits at the other end is not given the
/// chance to send without end.
const MOST: u64 = 1024 * 1024;

/// Whether the socket is in a folder of this user's. The usual place is
/// the user's own runtime directory; without one it is a folder in the
/// shared temporary directory, which someone else could have made first
/// to answer in the daemon's place.
#[cfg(unix)]
fn ours(socket: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Some(dir) = socket.parent().filter(|dir| *dir == crate::paths::fallback_runtime_dir()) else { return true };
    let me = std::fs::metadata("/proc/self").map(|m| m.uid());
    match (std::fs::symlink_metadata(dir), me) {
        (Ok(folder), Ok(me)) => folder.is_dir() && folder.uid() == me,
        // No folder: no daemon either, which connecting will say.
        _ => true,
    }
}

/// Ask the daemon at `socket` one thing and wait for its answer.
#[cfg(unix)]
pub fn ask(socket: &Path, request: &Request, timeout: Duration) -> Result<Response, IpcError> {
    if !ours(socket) {
        return Err(IpcError::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{} is someone else's", socket.display()))));
    }
    let mut stream = std::os::unix::net::UnixStream::connect(socket).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => IpcError::NotRunning,
        _ => IpcError::Io(e),
    })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(stream.take(MOST)).read_line(&mut answer)?;
    Ok(serde_json::from_str(&answer)?)
}

/// Ask the daemon at `socket` one thing and wait for its answer. On
/// Windows the socket is a named pipe, opened like a file; a pipe has no
/// time limit of its own, so the asking is done aside and waited for.
#[cfg(windows)]
pub fn ask(socket: &Path, request: &Request, timeout: Duration) -> Result<Response, IpcError> {
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    let pipe = socket.to_path_buf();
    let (tell, told) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let answer = (|| -> std::io::Result<String> {
            let mut pipe = open_pipe(&pipe)?;
            pipe.write_all(line.as_bytes())?;
            let mut answer = String::new();
            BufReader::new(pipe.take(MOST)).read_line(&mut answer)?;
            Ok(answer)
        })();
        let _ = tell.send(answer);
    });
    match told.recv_timeout(timeout) {
        Ok(Ok(answer)) => Ok(serde_json::from_str(&answer)?),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Err(IpcError::NotRunning),
        Ok(Err(e)) => Err(IpcError::Io(e)),
        Err(_) => Err(IpcError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "no answer in time"))),
    }
}

/// Open the daemon's pipe. While it is answering someone else there is a
/// moment with no free end to connect to (the system says the pipe is
/// busy), which is waited out rather than reported.
#[cfg(windows)]
fn open_pipe(pipe: &Path) -> std::io::Result<std::fs::File> {
    // ERROR_PIPE_BUSY
    const BUSY: i32 = 231;
    let mut tries = 0;
    loop {
        // Whoever holds the other end may learn who is asking, and no
        // more: it cannot act as this user (SECURITY_IDENTIFICATION).
        use std::os::windows::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new().read(true).write(true).security_qos_flags(0x0001_0000).open(pipe) {
            Err(e) if e.raw_os_error() == Some(BUSY) && tries < 50 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            opened => return opened,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_responses_have_a_stable_shape() {
        assert_eq!(serde_json::to_string(&Request::Save).unwrap(), r#"{"request":"save"}"#);
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"request":"capture","force":true}"#).unwrap(),
            Request::Capture { force: Some(true) }
        );
        let saved = Response::Saved { path: "/c/a.mp4".into() };
        assert_eq!(serde_json::to_string(&saved).unwrap(), r#"{"response":"saved","path":"/c/a.mp4"}"#);
        let status = Response::Status(Status { state: "capturing".into(), buffer_secs: 60, ..Default::default() });
        assert_eq!(serde_json::from_str::<Response>(&serde_json::to_string(&status).unwrap()).unwrap(), status);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_daemon_is_said_plainly() {
        let dir = tempfile::tempdir().unwrap();
        let err = ask(&dir.path().join("none.sock"), &Request::Status, Duration::from_secs(1)).unwrap_err();
        assert!(matches!(err, IpcError::NotRunning));
    }

    #[cfg(unix)]
    #[test]
    fn a_request_gets_its_answer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), Request::Save);
            (&stream).write_all(b"{\"response\":\"saved\",\"path\":\"/c/a.mp4\"}\n").unwrap();
        });
        let answer = ask(&path, &Request::Save, Duration::from_secs(2)).unwrap();
        assert_eq!(answer, Response::Saved { path: "/c/a.mp4".into() });
        server.join().unwrap();
    }
}
