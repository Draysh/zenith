//! Where the daemon is asked things: a Unix socket on Linux, a named pipe
//! on Windows. What is said over either is the same (one JSON request on a
//! line, one JSON response on a line) and is answered by the same code;
//! only the listening differs.

use crate::daemon::Msg;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use zenith_core::ipc::{Request, Response};

/// How long a client gets to send its request, and how long that may be.
const REQUEST_WITHIN: Duration = Duration::from_secs(5);
const REQUEST_BYTES: u64 = 64 * 1024;

fn log(text: impl std::fmt::Display) {
    crate::warn(format_args!("zenithd: {text}"));
}

/// One request on a connection, one answer.
async fn serve<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, tx: &mpsc::UnboundedSender<Msg>) {
    let mut line = String::new();
    // A client that connects and says nothing, or never stops, is let go.
    let asked = tokio::time::timeout(REQUEST_WITHIN, BufReader::new((&mut *stream).take(REQUEST_BYTES)).read_line(&mut line)).await;
    if !matches!(asked, Ok(Ok(_))) {
        return;
    }
    let response = match serde_json::from_str::<Request>(&line) {
        Err(e) => Response::Error { message: format!("not a request: {e}") },
        Ok(request) => {
            let (reply, answer) = oneshot::channel();
            let _ = tx.send(Msg::Ipc(request, reply));
            answer.await.unwrap_or(Response::Error { message: "the daemon is shutting down".into() })
        }
    };
    let mut text = serde_json::to_string(&response).unwrap_or_else(|_| r#"{"response":"error","message":"the answer could not be written"}"#.into());
    text.push('\n');
    let _ = stream.write_all(text.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Make the folder the socket goes in. The one used when there is no
/// runtime directory is in the shared temporary folder, so it has to be
/// this user's and is closed to everyone else.
#[cfg(unix)]
fn socket_dir(socket: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Some(dir) = socket.parent() else { return Ok(()) };
    std::fs::create_dir_all(dir).map_err(|e| format!("{} could not be made: {e}", dir.display()))?;
    if dir == zenith_core::paths::fallback_runtime_dir() {
        let meta = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
        // SAFETY: geteuid has nothing to get wrong.
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
            return Err(format!("{} is someone else's", dir.display()));
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The one daemon of this socket: a lock held for as long as it runs, and
/// let go by the system however it ends. Only its holder may clear away a
/// socket file left behind.
#[cfg(unix)]
fn only_one(socket: &Path) -> Result<std::fs::File, String> {
    let path = socket.with_extension("lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{} could not be made: {e}", path.display()))?;
    file.try_lock().map_err(|_| "it is already running".to_string())?;
    Ok(file)
}

/// What keeps this the only daemon, for as long as it is held.
#[cfg(unix)]
pub type Only = std::fs::File;
/// On Windows the pipe itself does: it exists for as long as the daemon has
/// an end of it open, and nobody else can make its first end meanwhile.
#[cfg(windows)]
pub struct Only;

/// Start answering requests at `socket`, each handed on as a message.
/// Refuses when another daemon already does.
#[cfg(unix)]
pub fn listen(socket: &Path, tx: mpsc::UnboundedSender<Msg>) -> Result<Only, String> {
    use std::os::unix::fs::PermissionsExt;
    socket_dir(socket)?;
    let only = only_one(socket)?;
    let _ = std::fs::remove_file(socket);
    let listener = tokio::net::UnixListener::bind(socket).map_err(|e| format!("the socket {} could not be made: {e}", socket.display()))?;
    // Only this user may tell the recorder what to do.
    let _ = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600));
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((mut stream, _)) => {
                    let tx = tx.clone();
                    drop(tokio::spawn(async move { serve(&mut stream, &tx).await }));
                }
                // Out of file handles, most likely: wait for some to come back.
                Err(e) => {
                    log(format!("a request could not be taken: {e}"));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    });
    Ok(only)
}

/// Start answering requests at the pipe `socket`, each handed on as a
/// message. Refuses when another daemon already does: the first end of a
/// pipe can be made only while the pipe does not exist.
#[cfg(windows)]
pub fn listen(socket: &Path, tx: mpsc::UnboundedSender<Msg>) -> Result<Only, String> {
    use tokio::net::windows::named_pipe::ServerOptions;
    // As made here the pipe is this user's: others may not write to it,
    // and nothing from another machine may connect.
    let first = ServerOptions::new().first_pipe_instance(true).create(socket).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => "it is already running".to_string(),
        _ => format!("the pipe {} could not be made: {e}", socket.display()),
    })?;
    let name = socket.to_path_buf();
    tokio::spawn(async move {
        let mut waiting = first;
        loop {
            let connected = waiting.connect().await;
            // The next end is made before this one is answered and let go,
            // so the pipe never stops existing: a client always finds an end
            // to connect to, and no second daemon can take the name.
            let next = loop {
                match ServerOptions::new().create(&name) {
                    Ok(next) => break next,
                    Err(e) => {
                        log(format!("the pipe could not be opened again: {e}"));
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            };
            let mut client = std::mem::replace(&mut waiting, next);
            match connected {
                Ok(()) => {
                    let tx = tx.clone();
                    drop(tokio::spawn(async move {
                        serve(&mut client, &tx).await;
                        // Let the client read its answer and hang up first:
                        // closing this end is then never what cuts it short.
                        let _ = tokio::time::timeout(Duration::from_secs(1), client.read(&mut [0u8; 1])).await;
                    }));
                }
                Err(e) => {
                    log(format!("a request could not be taken: {e}"));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    });
    Ok(Only)
}

/// Clear away what `listen` left in the file system: the socket file. A
/// pipe goes by itself when its last end is closed.
pub fn close(socket: &Path) {
    if cfg!(unix) {
        let _ = std::fs::remove_file(socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;
    use zenith_core::ipc::Status;

    /// A daemon that answers every request the same way.
    fn answering(response: Response) -> mpsc::UnboundedSender<Msg> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Msg::Ipc(_, reply) = msg {
                    let _ = reply.send(response.clone());
                }
            }
        });
        tx
    }

    async fn asked(request: &[u8], response: Response) -> String {
        let (mut client, mut server) = duplex(REQUEST_BYTES as usize * 2);
        let tx = answering(response);
        client.write_all(request).await.unwrap();
        let serving = tokio::spawn(async move { serve(&mut server, &tx).await });
        let mut answer = String::new();
        client.read_to_string(&mut answer).await.unwrap();
        serving.await.unwrap();
        answer
    }

    #[tokio::test]
    async fn a_request_on_a_line_gets_its_answer_on_a_line() {
        let status = Response::Status(Status { state: "idle".into(), buffer_secs: 60, ..Default::default() });
        let answer = asked(b"{\"request\":\"status\"}\n", status.clone()).await;
        assert!(answer.ends_with('\n'));
        assert_eq!(serde_json::from_str::<Response>(&answer).unwrap(), status);
    }

    #[tokio::test]
    async fn what_is_not_a_request_is_told_so() {
        let answer = asked(b"hello\n", Response::Ok).await;
        let Response::Error { message } = serde_json::from_str(&answer).unwrap() else { panic!("{answer}") };
        assert!(message.starts_with("not a request"), "{message}");
    }

    #[tokio::test]
    async fn a_request_that_never_ends_is_cut_off() {
        let long = vec![b'x'; REQUEST_BYTES as usize + 100];
        let answer = asked(&long, Response::Ok).await;
        let Response::Error { message } = serde_json::from_str(&answer).unwrap() else { panic!("{answer}") };
        assert!(message.starts_with("not a request"), "{message}");
    }
}
