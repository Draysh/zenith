//! The programs that are running, by the name of their file: how a game
//! the user added by hand is known again (`[[games.added]]` in the
//! settings), and the list they pick one from.

use crate::config::AddedGame;

/// A program's name for comparing: lower case, without `.exe`.
pub fn bare(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    lower.strip_suffix(".exe").map(String::from).unwrap_or(lower)
}

/// The file's name at the end of a path, whichever way its slashes lean
/// (a Windows game under Proton runs as `Z:\games\Game.exe`).
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path).trim()
}

/// The first of the added games that is among the programs running.
pub fn added<'a, 'b>(added: &'a [AddedGame], running: impl IntoIterator<Item = &'b str>) -> Option<&'a AddedGame> {
    if added.is_empty() {
        return None;
    }
    let running: Vec<String> = running.into_iter().map(bare).collect();
    added.iter().find(|game| {
        let wanted = bare(&game.program);
        !wanted.is_empty() && running.contains(&wanted)
    })
}

/// A program to pick as a game: its file's name and, where the system
/// tells, what its window is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub program: String,
    pub title: String,
}

/// One of each program, in the order of the alphabet.
fn tidy(mut list: Vec<Running>) -> Vec<Running> {
    list.sort_by_key(|r| (bare(&r.program), r.title.is_empty()));
    list.dedup_by_key(|r| bare(&r.program));
    list
}

/// The program a process runs, from its command line (`/proc/<pid>/cmdline`:
/// the words separated by NUL, the program first) and the file it was
/// started from (`/proc/<pid>/exe`). A Windows game under Wine or Proton
/// is started from Wine's own file and names its `.exe` first on the
/// command line; any other program is its file, since some write their
/// whole command line as one word.
pub fn of_process(cmdline: &[u8], exe: Option<&std::path::Path>) -> Option<String> {
    let first = cmdline.split(|b| *b == 0).next().and_then(|first| std::str::from_utf8(first).ok()).map(file_name).unwrap_or("");
    let file = exe.and_then(|exe| exe.file_name()?.to_str()).map(|name| name.strip_suffix(" (deleted)").unwrap_or(name));
    let name = match file {
        Some(file) if !first.to_ascii_lowercase().ends_with(".exe") => file,
        _ => first,
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// The program of the process whose folder under `/proc` is `dir`.
pub fn of_process_in(dir: &std::path::Path) -> Option<String> {
    of_process(&std::fs::read(dir.join("cmdline")).ok()?, std::fs::read_link(dir.join("exe")).ok().as_deref())
}

/// The user's own programs among the processes under `proc_root`.
#[cfg(unix)]
pub fn running_in(proc_root: &std::path::Path) -> Vec<Running> {
    use std::os::unix::fs::MetadataExt;
    let me = std::fs::metadata(proc_root.join("self")).map(|m| m.uid()).ok();
    let mut list = Vec::new();
    for entry in std::fs::read_dir(proc_root).into_iter().flatten().flatten() {
        if entry.file_name().to_str().is_none_or(|n| n.parse::<u32>().is_err()) {
            continue;
        }
        if me.is_some() && entry.metadata().map(|m| m.uid()).ok() != me {
            continue;
        }
        if let Some(program) = of_process_in(&entry.path()) {
            list.push(Running { program, title: String::new() });
        }
    }
    tidy(list)
}

/// The programs to pick a game from: every program of the user's on
/// Linux, the ones with a window on Windows.
#[cfg(unix)]
pub fn running() -> Vec<Running> {
    running_in(std::path::Path::new("/proc"))
}

#[cfg(windows)]
pub use windows::{foreground, names, path_of, running};

#[cfg(windows)]
mod windows {
    use super::{Running, file_name, tidy};
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
    };
    use windows_sys::core::BOOL;

    fn text(wide: &[u16]) -> String {
        let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
        String::from_utf16_lossy(&wide[..end])
    }

    /// Every process: its number and its program's file name.
    pub fn names() -> Vec<(u32, String)> {
        let mut list = Vec::new();
        // SAFETY: the snapshot is ours until closed, and `entry` is a
        // structure of the size it says, for each call to fill in.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return list;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                list.push((entry.th32ProcessID, text(&entry.szExeFile)));
                more = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
        }
        list
    }

    /// Where a process's program is, when Windows lets it be asked.
    pub fn path_of(pid: u32) -> Option<String> {
        // SAFETY: the handle is closed again, and the buffer is as long as
        // `len` says, which comes back as the length written.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut buffer = [0u16; 1024];
            let mut len = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len);
            CloseHandle(process);
            (ok != 0 && len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
        }
    }

    fn pid_of(window: HWND) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: `pid` is ours to be written to.
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        (pid != 0).then_some(pid)
    }

    /// The process whose window is in front.
    pub fn foreground() -> Option<u32> {
        // SAFETY: asks which window is in front; none is null.
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() { None } else { pid_of(window) }
    }

    /// Called for each top-level window; `list` is the `Vec<Running>` of
    /// `running`.
    unsafe extern "system" fn each(window: HWND, list: LPARAM) -> BOOL {
        // SAFETY: `list` is the pointer `running` gave, alive for the
        // whole of `EnumWindows`; the buffer is as long as is said.
        unsafe {
            let list = &mut *(list as *mut Vec<Running>);
            let style = GetWindowLongW(window, GWL_EXSTYLE) as u32;
            if IsWindowVisible(window) == 0 || style & WS_EX_TOOLWINDOW != 0 {
                return 1;
            }
            let mut buffer = [0u16; 256];
            let len = GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
            let title = text(&buffer[..len.max(0) as usize]);
            if title.trim().is_empty() {
                return 1;
            }
            if let Some(pid) = pid_of(window).filter(|pid| *pid != std::process::id())
                && let Some(path) = path_of(pid)
            {
                list.push(Running { program: file_name(&path).to_string(), title: title.trim().to_string() });
            }
        }
        1
    }

    /// The programs with a window to see: what a game is, while it runs.
    pub fn running() -> Vec<Running> {
        let mut list: Vec<Running> = Vec::new();
        // SAFETY: `each` gets the list for as long as this call lasts.
        unsafe { EnumWindows(Some(each), &mut list as *mut Vec<Running> as LPARAM) };
        // Windows' own shell has windows too, and is no game.
        list.retain(|r| !["explorer.exe", "textinputhost.exe", "applicationframehost.exe", "systemsettings.exe"].contains(&r.program.to_lowercase().as_str()));
        tidy(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(name: &str, program: &str) -> AddedGame {
        AddedGame { name: name.into(), program: program.into() }
    }

    #[test]
    fn a_program_is_known_whatever_its_case_and_with_or_without_exe() {
        let games = [game("Sinking Star", "SinkingStar.exe"), game("Other", "other")];
        assert_eq!(added(&games, ["steam.exe", "sinkingstar.EXE"]).map(|g| g.name.as_str()), Some("Sinking Star"));
        assert_eq!(added(&games, ["sinkingstar"]).map(|g| g.name.as_str()), Some("Sinking Star"));
        assert_eq!(added(&games, ["Other.exe"]).map(|g| g.name.as_str()), Some("Other"));
        assert_eq!(added(&games, ["sinkingstar2.exe", "another"]), None);
        assert_eq!(added(&[], ["other"]), None);
        // The first in the settings wins when both run.
        assert_eq!(added(&games, ["other", "sinkingstar.exe"]).map(|g| g.name.as_str()), Some("Sinking Star"));
        assert_eq!(added(&[game("Nameless", " ")], [""]), None);
    }

    #[test]
    fn the_program_is_its_file_or_the_exe_wine_was_given() {
        use std::path::Path;
        let of = |cmdline: &[u8], exe: Option<&str>| of_process(cmdline, exe.map(Path::new));
        assert_eq!(of(b"/usr/bin/supertux2\0--fullscreen\0", Some("/usr/bin/supertux2")).as_deref(), Some("supertux2"));
        assert_eq!(of(b"./game\0", None).as_deref(), Some("game"));
        // Under Wine the file is Wine's and the game is the first word.
        assert_eq!(of(b"Z:\\games\\Sinking Star\\Star.EXE\0-windowed\0", Some("/usr/bin/wine64-preloader")).as_deref(), Some("Star.EXE"));
        assert_eq!(of(b"/games/star.exe\0", Some("/opt/proton/files/bin/wine-preloader")).as_deref(), Some("star.exe"));
        // A command line written as one word is not a file's name.
        assert_eq!(of(b"/opt/app/app.asar --enable-sandbox --type=renderer", Some("/opt/app/electron")).as_deref(), Some("electron"));
        assert_eq!(of(b"/usr/bin/old\0", Some("/usr/bin/old (deleted)")).as_deref(), Some("old"));
        assert_eq!(of(b"", None), None, "a kernel thread has neither");
        assert_eq!(of(b"/usr/bin/\0", None), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_programs_to_pick_from_are_listed_once_each_in_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("self")).unwrap();
        for (pid, cmdline) in [(10, &b"/usr/bin/zsh\0"[..]), (11, b"C:\\Game\\Star.exe\0"), (12, b"/usr/bin/zsh\0-l\0"), (13, b""), (14, b"/opt/Alpha\0")] {
            let process = dir.path().join(pid.to_string());
            std::fs::create_dir_all(&process).unwrap();
            std::fs::write(process.join("cmdline"), cmdline).unwrap();
        }
        std::fs::create_dir_all(dir.path().join("not-a-process")).unwrap();
        let names: Vec<String> = running_in(dir.path()).into_iter().map(|r| r.program).collect();
        assert_eq!(names, ["Alpha", "Star.exe", "zsh"]);
    }
}
