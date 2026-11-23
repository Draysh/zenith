//! Telling the recorder what to do. The two recorders take the same command
//! line and differ only in how they are spoken to once they run:
//! gpu-screen-recorder listens for signals; `zenith-rec`, on Windows where
//! there are none, reads a line at a time from its input. This is the one
//! place that knows which is which.

use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};

/// The line that makes `zenith-rec` write a clip and print its path.
pub const SAVE: &str = "save\n";
/// The line that makes it stop. Its input closing means the same.
pub const STOP: &str = "stop\n";
/// A line is a few bytes into a pipe; a recorder that has stopped reading
/// must not hold the daemon up over one.
const WRITE_WITHIN: Duration = Duration::from_secs(1);

/// How a recorder is spoken to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// SIGUSR1 to save, SIGINT to stop.
    #[cfg(unix)]
    Signals,
    /// `save` and `stop` as lines on its input.
    Stdin,
}

impl Control {
    /// What this system's recorder understands.
    pub fn native() -> Self {
        #[cfg(unix)]
        return Self::Signals;
        #[cfg(windows)]
        return Self::Stdin;
    }

    /// The control a name stands for; anything but `stdin` is this system's
    /// own. The tests use it to run the Windows way on Linux.
    pub fn named(name: Option<&str>) -> Self {
        if name == Some("stdin") { Self::Stdin } else { Self::native() }
    }

    /// The command that runs the recorder `program`, with its input set up
    /// for this control. Its output and arguments are the caller's to add.
    pub fn command(self, program: &str) -> Command {
        #[cfg(unix)]
        let mut command = Command::new(program);
        // On Windows the recorder is beside this program, and must not open
        // a console window of its own.
        #[cfg(windows)]
        let mut command = Command::from(zenith_core::paths::command(program));
        command.stdin(match self {
            #[cfg(unix)]
            Self::Signals => Stdio::null(),
            Self::Stdin => Stdio::piped(),
        });
        // SAFETY: only `prctl`, which may be called between fork and exec.
        // The recorder is told to stop when this process is gone, however
        // it went, so it never records on with nobody to save its clips.
        // (On Windows its input closes with this process, which it takes
        // the same way.)
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        command
    }

    /// The way to speak to a recorder just started with [`Self::command`].
    /// `None` when it ended as it started.
    pub fn link(self, child: &mut Child) -> Option<Link> {
        Some(Link { control: self, pid: child.id()?, stdin: child.stdin.take() })
    }
}

/// The daemon's end of a running recorder.
pub struct Link {
    control: Control,
    pid: u32,
    stdin: Option<ChildStdin>,
}

impl Link {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Ask for a clip. Only to be called while the recorder has not been
    /// waited for: its process number is someone else's after that.
    pub async fn save(&mut self) {
        match self.control {
            #[cfg(unix)]
            Control::Signals => {
                // SAFETY: a signal to our own child, which is still ours to wait for.
                unsafe { libc::kill(self.pid as i32, libc::SIGUSR1) };
            }
            Control::Stdin => self.say(SAVE).await,
        }
    }

    /// Ask it to stop. Whether it does is for the caller to wait and see.
    pub async fn stop(&mut self) {
        match self.control {
            #[cfg(unix)]
            Control::Signals => {
                // SAFETY: a signal to our own child, which has not been waited for yet.
                unsafe { libc::kill(self.pid as i32, libc::SIGINT) };
            }
            Control::Stdin => {
                self.say(STOP).await;
                // Closed as well: a recorder too busy to read the line
                // still sees its input end.
                self.stdin = None;
            }
        }
    }

    /// Write a line to the recorder. One that cannot be written means the
    /// recorder is going or gone, which its exit reports better than this.
    async fn say(&mut self, line: &str) {
        let Some(stdin) = &mut self.stdin else { return };
        let written = tokio::time::timeout(WRITE_WITHIN, async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        })
        .await;
        if !matches!(written, Ok(Ok(()))) {
            crate::warn(format_args!("zenithd: the recorder did not take \"{}\"", line.trim_end()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_lines_are_what_zenith_rec_reads() {
        assert_eq!((SAVE, STOP), ("save\n", "stop\n"));
    }

    #[test]
    fn only_stdin_is_asked_for_by_name() {
        assert_eq!(Control::named(Some("stdin")), Control::Stdin);
        assert_eq!(Control::named(None), Control::native());
        assert_eq!(Control::named(Some("signals")), Control::native());
        #[cfg(unix)]
        assert_eq!(Control::native(), Control::Signals);
    }

    /// A recorder that writes down what it is told, until its input ends.
    #[cfg(unix)]
    async fn told(control: Control, script: &str) -> (Link, Child) {
        let mut command = control.command("sh");
        let mut child = command.args(["-c", script]).stdout(Stdio::piped()).spawn().unwrap();
        let link = control.link(&mut child).unwrap();
        (link, child)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn save_and_stop_reach_a_recorder_that_reads_lines() {
        let (mut link, child) = told(Control::Stdin, "while IFS= read -r line; do echo \"got $line\"; done; echo closed").await;
        assert!(link.pid() > 0);
        link.save().await;
        link.save().await;
        link.stop().await;
        let out = child.wait_with_output().await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "got save\ngot save\ngot stop\nclosed\n", "the lines, then its input closing");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_recorder_that_is_gone_does_not_hold_anything_up() {
        let (mut link, mut child) = told(Control::Stdin, "exit 0").await;
        child.wait().await.unwrap();
        link.save().await;
        link.stop().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn signals_reach_a_recorder_that_listens_for_them() {
        let script = "trap 'echo saved' USR1; trap 'echo stopped; exit 0' INT; echo ready; while :; do sleep 0.05 & wait $!; done";
        let (mut link, mut child) = told(Control::Signals, script).await;
        let mut lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(child.stdout.take().unwrap()));
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("ready"));
        link.save().await;
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("saved"));
        link.stop().await;
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("stopped"));
        assert!(child.wait().await.unwrap().success());
    }
}
