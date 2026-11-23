//! Where this program keeps its files, and the one thing it asks of the
//! operating system about its child: that it never outlives us.

use std::path::PathBuf;
use std::process::Child;

/// Zenith's own folder for things that are not the user's clips:
/// `%LOCALAPPDATA%\Zenith` on Windows. Elsewhere this program only runs in
/// tests, and the temporary directory will do.
pub fn data_dir() -> PathBuf {
    if cfg!(windows)
        && let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty())
    {
        return PathBuf::from(local).join("Zenith");
    }
    std::env::temp_dir()
}

/// Where the log and the diagnose reports are written.
pub fn logs_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ZENITH_REC_LOG_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if cfg!(windows) { data_dir().join("logs") } else { data_dir() }
}

/// Where the pieces of the replay buffer are kept: a folder of this run's
/// own, so two recorders never clear each other's pieces. On Windows they
/// sit together under `ring`, where one that was left behind is found.
pub fn ring_dir() -> PathBuf {
    let pid = std::process::id();
    if cfg!(windows) { rings_dir().join(pid.to_string()) } else { data_dir().join(format!("zenith-rec-ring-{pid}")) }
}

/// The folder the rings of every run are in, on Windows.
pub fn rings_dir() -> PathBuf {
    data_dir().join("ring")
}

/// A line naming the system, for the top of the log.
pub fn os_description() -> String {
    let name = sysinfo::System::long_os_version().or_else(|_| sysinfo::System::name()).unwrap_or_else(|_| std::env::consts::OS.to_string());
    let build = sysinfo::System::kernel_version().unwrap_or_else(|_| "?".into());
    format!("{name} (build {build}, {})", std::env::consts::ARCH)
}

/// The Windows build number, when this is Windows and it can be read.
pub fn windows_build() -> Option<u32> {
    if cfg!(windows) { sysinfo::System::kernel_version().ok()?.trim().parse().ok() } else { None }
}

/// Recording one program's sound apart from the rest came with
/// Windows 10 2004, which is build 19041.
pub fn build_has_app_audio(build: Option<u32>) -> bool {
    build.is_some_and(|b| b >= 19041)
}

/// Ask that the calling thread runs before ordinary ones. A game takes
/// every processor it can get, and a thread that reads the sound devices
/// late has gaps to show for it. Does nothing where there is no Windows.
pub fn prefer_this_thread() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
        // SAFETY: the pseudo-handle of the calling thread is always valid.
        if let Err(err) = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } {
            crate::note!("sound: its thread could not be put before the others ({err})");
        }
    }
}

/// Ties children to this process: when the daemon kills us without a word,
/// Windows closes the job with us and takes ffmpeg along. Without it an
/// orphaned ffmpeg would go on capturing the screen into the ring forever.
pub struct ChildLeash {
    #[cfg(windows)]
    job: Option<windows::Win32::Foundation::HANDLE>,
}

impl ChildLeash {
    #[cfg(not(windows))]
    pub fn new() -> Self {
        Self {}
    }

    /// Nothing to do: only tests run here, and they stop us with `stop`.
    #[cfg(not(windows))]
    pub fn hold(&self, _child: &Child) {}

    #[cfg(windows)]
    pub fn new() -> Self {
        use windows::Win32::System::JobObjects::*;
        // SAFETY: plain calls with a zeroed, correctly sized information
        // structure; the handle is kept for the life of the process.
        let job = unsafe {
            CreateJobObjectW(None, windows::core::PCWSTR::null()).ok().and_then(|job| {
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
                match SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const info).cast(), size) {
                    Ok(()) => Some(job),
                    Err(err) => {
                        crate::note!("the job that ties ffmpeg to this process could not be set up: {err}");
                        None
                    }
                }
            })
        };
        Self { job }
    }

    #[cfg(windows)]
    pub fn hold(&self, child: &Child) {
        use std::os::windows::io::AsRawHandle;
        let Some(job) = self.job else { return };
        let process = windows::Win32::Foundation::HANDLE(child.as_raw_handle());
        // SAFETY: both handles are open and owned by this process.
        if let Err(err) = unsafe { windows::Win32::System::JobObjects::AssignProcessToJobObject(job, process) } {
            crate::note!("ffmpeg could not be tied to this process ({err}); it may outlive a crash");
        }
    }
}

impl Default for ChildLeash {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_audio_needs_windows_10_2004() {
        assert!(!build_has_app_audio(None));
        assert!(!build_has_app_audio(Some(18363)));
        assert!(build_has_app_audio(Some(19041)));
        assert!(build_has_app_audio(Some(26100)));
    }
}
