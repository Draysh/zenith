//! The game's own window, so that it is what is recorded and not the whole
//! screen with whatever else is on it.
//!
//! The daemon knows a game the same two ways: its program is in a folder
//! Steam installs games into, or it is one the user added by its program's
//! name. It passes both on (`-game-dir`, `-game-program`), and which of the
//! windows on screen is a game's is decided here from them, in plain logic.
//! Listing the windows is the small Windows part.

use std::path::Path;

/// A window on screen, as far as choosing one goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// Windows' handle for it, which Windows.Graphics.Capture takes.
    pub handle: u64,
    pub pid: u32,
    /// The full path of the program it belongs to.
    pub program: String,
    pub title: String,
    /// The size of what it shows, in pixels; nothing while minimised.
    pub size: (u32, u32),
    pub minimised: bool,
    /// It is the window in front.
    pub front: bool,
}

/// A window smaller than this, either way, is a splash screen or a
/// dialogue: not what a game is played in.
const SMALLEST: u32 = 200;

/// A path for comparing, however it was written: lower case, backward
/// slashes, without the `\\?\` Windows puts before a full path.
fn folded(path: &str) -> String {
    let path = path.trim().replace('/', "\\").to_lowercase();
    path.strip_prefix("\\\\?\\").unwrap_or(&path).trim_end_matches('\\').to_string()
}

/// A program's name for comparing: its file's name in lower case, without `.exe`.
fn bare(program: &str) -> String {
    let folded = folded(program);
    let name = folded.rsplit('\\').next().unwrap_or("");
    name.strip_suffix(".exe").unwrap_or(name).to_string()
}

/// What Steam keeps beside the games in the same folders, by how its
/// folder's name starts.
const NOT_GAMES: [&str; 3] = ["steamvr", "steamworks", "steam controller configs"];

/// What makes a program a game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    /// Folders games are installed into, each with its folder a level below.
    dirs: Vec<String>,
    /// Programs that are games wherever they are.
    programs: Vec<String>,
}

impl Rules {
    pub fn new(dirs: &[impl AsRef<Path>], programs: &[String]) -> Self {
        Self {
            dirs: dirs.iter().map(|dir| folded(&dir.as_ref().to_string_lossy())).filter(|dir| !dir.is_empty()).map(|dir| dir + "\\").collect(),
            programs: programs.iter().map(|program| bare(program)).filter(|program| !program.is_empty()).collect(),
        }
    }

    /// Nothing is a game: there is nothing to look for.
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty() && self.programs.is_empty()
    }

    /// Whether the program at `path` is a game.
    pub fn is_game(&self, path: &str) -> bool {
        if self.programs.contains(&bare(path)) {
            return true;
        }
        let path = folded(path);
        self.dirs.iter().any(|dir| {
            // In a game's own folder, a level or more below the library's.
            path.strip_prefix(dir.as_str()).is_some_and(|inside| inside.contains('\\') && !NOT_GAMES.iter().any(|not| inside.starts_with(not)))
        })
    }
}

/// The game's window among `windows`: the one in front if a game's is in
/// front; else the one being recorded (`current`), while it is still
/// there; else the largest. A game's launcher is a window of the game's
/// too, and gives way like this once the game itself is in front.
pub fn choose<'a>(rules: &Rules, windows: &'a [Window], current: Option<u64>) -> Option<&'a Window> {
    let games: Vec<&Window> =
        windows.iter().filter(|w| rules.is_game(&w.program)).filter(|w| w.minimised || (w.size.0 >= SMALLEST && w.size.1 >= SMALLEST)).collect();
    let area = |w: &Window| w.size.0 as u64 * w.size.1 as u64;
    games
        .iter()
        .find(|w| w.front)
        .or_else(|| games.iter().find(|w| Some(w.handle) == current))
        // `max_by_key` keeps the last of equals; looked through backwards, that is the first.
        .or_else(|| games.iter().rev().max_by_key(|w| (!w.minimised, area(w))))
        .copied()
}

/// The windows on screen now. None where there is no Windows.
pub fn list() -> Vec<Window> {
    #[cfg(windows)]
    {
        windows_impl::list()
    }
    #[cfg(not(windows))]
    Vec::new()
}

#[cfg(windows)]
mod windows_impl {
    use super::Window;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetClientRect, GetForegroundWindow, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        WS_EX_TOOLWINDOW,
    };
    use windows::core::{BOOL, PWSTR};

    /// Where a process's program is, when Windows lets it be asked.
    fn path_of(pid: u32) -> Option<String> {
        // SAFETY: the handle is closed again, and the buffer is as long as
        // `len` says, which comes back as the length written.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buffer = [0u16; 1024];
            let mut len = buffer.len() as u32;
            let asked = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &raw mut len);
            let _ = CloseHandle(process);
            (asked.is_ok() && len > 0).then(|| String::from_utf16_lossy(&buffer[..len as usize]))
        }
    }

    /// A window that is there but not shown (one of another virtual
    /// desktop, a suspended store app) is "cloaked".
    fn cloaked(window: HWND) -> bool {
        let mut value = 0u32;
        // SAFETY: `value` is the four bytes this attribute is.
        let asked = unsafe { DwmGetWindowAttribute(window, DWMWA_CLOAKED, (&raw mut value).cast(), std::mem::size_of::<u32>() as u32) };
        asked.is_ok() && value != 0
    }

    fn describe(window: HWND, front: HWND) -> Option<Window> {
        // SAFETY: every call takes the handle and plain memory of ours of
        // the size it is told; a window closed meanwhile makes them fail.
        unsafe {
            let style = GetWindowLongW(window, GWL_EXSTYLE) as u32;
            if !IsWindowVisible(window).as_bool() || style & WS_EX_TOOLWINDOW.0 != 0 || cloaked(window) {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&raw mut pid));
            if pid == 0 || pid == std::process::id() {
                return None;
            }
            let program = path_of(pid)?;
            let mut text = [0u16; 256];
            let len = GetWindowTextW(window, &mut text).clamp(0, text.len() as i32) as usize;
            let mut area = RECT::default();
            let _ = GetClientRect(window, &raw mut area);
            Some(Window {
                handle: window.0 as usize as u64,
                pid,
                program,
                title: String::from_utf16_lossy(&text[..len]).trim().to_string(),
                size: ((area.right - area.left).max(0) as u32, (area.bottom - area.top).max(0) as u32),
                minimised: IsIconic(window).as_bool(),
                front: window == front,
            })
        }
    }

    unsafe extern "system" fn collect(window: HWND, list: LPARAM) -> BOOL {
        // SAFETY: `list` is the `Vec` `list()` passed in, alive for the
        // whole of `EnumWindows`, which calls this on its thread.
        let list = unsafe { &mut *(list.0 as *mut Vec<HWND>) };
        list.push(window);
        TRUE
    }

    pub fn list() -> Vec<Window> {
        let mut handles: Vec<HWND> = Vec::new();
        // SAFETY: the callback only touches `handles`, which outlives the call.
        let _ = unsafe { EnumWindows(Some(collect), LPARAM(&raw mut handles as isize)) };
        // SAFETY: asks which window is in front; none is null.
        let front = unsafe { GetForegroundWindow() };
        handles.into_iter().filter_map(|window| describe(window, front)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules::new(&[r"C:\Program Files (x86)\Steam\steamapps\common", r"D:\SteamLibrary\steamapps\common\"], &["Star.exe".to_string(), " ".to_string()])
    }

    fn window(handle: u64, program: &str, size: (u32, u32)) -> Window {
        Window { handle, pid: handle as u32 + 1000, program: program.into(), title: "A window".into(), size, minimised: false, front: false }
    }

    const CS2: &str = r"C:\Program Files (x86)\Steam\steamapps\common\Counter-Strike Global Offensive\game\bin\win64\cs2.exe";

    #[test]
    fn a_program_in_a_games_folder_is_a_game_however_its_path_is_written() {
        let rules = rules();
        assert!(rules.is_game(CS2));
        assert!(rules.is_game(&CS2.to_uppercase()));
        assert!(rules.is_game(r"\\?\D:\SteamLibrary\steamapps\common\Hades\x64\Hades.exe"));
        assert!(rules.is_game("d:/steamlibrary/steamapps/common/Hades/Hades.exe"));
        assert!(!rules.is_game(r"C:\Program Files (x86)\Steam\steam.exe"));
        assert!(!rules.is_game(r"C:\Program Files (x86)\Steam\steamapps\common\loose.exe"), "not in a game's own folder");
        assert!(!rules.is_game(r"C:\Program Files\Mozilla Firefox\firefox.exe"));
        assert!(!rules.is_game(r"E:\steamapps\common\Hades\Hades.exe"), "another drive's folder was not named");
    }

    #[test]
    fn steams_own_tools_are_not_games() {
        let rules = rules();
        assert!(!rules.is_game(r"C:\Program Files (x86)\Steam\steamapps\common\SteamVR\bin\win64\vrmonitor.exe"));
        assert!(!rules.is_game(r"C:\Program Files (x86)\Steam\steamapps\common\Steamworks Shared\_CommonRedist\setup.exe"));
    }

    #[test]
    fn an_added_program_is_a_game_wherever_it_is() {
        let rules = rules();
        assert!(rules.is_game(r"E:\Games\Star\star.exe"));
        assert!(rules.is_game("STAR.EXE"));
        assert!(!rules.is_game(r"E:\Games\Star\starlauncher.exe"));
        assert!(!rules.is_game(""));
        assert!(Rules::default().is_empty());
        assert!(!rules.is_empty());
        assert!(Rules::new(&[""], &[]).is_empty());
    }

    #[test]
    fn the_games_window_is_chosen_and_nothing_else_is() {
        let rules = rules();
        let browser = window(1, r"C:\Program Files\Mozilla Firefox\firefox.exe", (2560, 1440));
        let game = window(2, CS2, (1920, 1080));
        assert_eq!(choose(&rules, std::slice::from_ref(&browser), None), None);
        assert_eq!(choose(&rules, &[browser.clone(), game.clone()], None), Some(&game));
        // A browser in front is still not a game.
        let front = Window { front: true, ..browser };
        assert_eq!(choose(&rules, &[front, game.clone()], None), Some(&game));
        assert_eq!(choose(&Rules::default(), &[game], None), None);
    }

    #[test]
    fn the_launcher_gives_way_to_the_game_once_it_is_in_front() {
        let rules = rules();
        let launcher = window(1, r"D:\SteamLibrary\steamapps\common\Game\launcher.exe", (1280, 720));
        let game = window(2, r"D:\SteamLibrary\steamapps\common\Game\bin\game.exe", (1024, 600));
        // Alone, the launcher is what there is to record.
        assert_eq!(choose(&rules, std::slice::from_ref(&launcher), None), Some(&launcher));
        // The game appears behind it: what is being recorded is kept.
        assert_eq!(choose(&rules, &[launcher.clone(), game.clone()], Some(1)), Some(&launcher));
        // It comes to the front.
        let playing = Window { front: true, ..game };
        assert_eq!(choose(&rules, &[launcher.clone(), playing.clone()], Some(1)), Some(&playing));
        // And stays chosen when something else is in front.
        let behind = Window { front: false, ..playing };
        assert_eq!(choose(&rules, &[launcher, behind.clone()], Some(2)), Some(&behind));
    }

    #[test]
    fn of_several_the_largest_is_taken_and_splash_screens_never() {
        let rules = rules();
        let splash = window(1, CS2, (180, 400));
        let small = window(2, CS2, (800, 600));
        let large = window(3, CS2, (1920, 1080));
        let twin = window(4, CS2, (1920, 1080));
        assert_eq!(choose(&rules, std::slice::from_ref(&splash), None), None);
        assert_eq!(choose(&rules, &[splash, small, large.clone(), twin], None), Some(&large), "and the first of equals");
    }

    #[test]
    fn a_minimised_game_is_still_the_game() {
        let rules = rules();
        let minimised = Window { minimised: true, size: (0, 0), ..window(1, CS2, (0, 0)) };
        assert_eq!(choose(&rules, std::slice::from_ref(&minimised), Some(1)), Some(&minimised));
        // One that can be seen is taken before it.
        let seen = window(2, CS2, (640, 480));
        assert_eq!(choose(&rules, &[minimised, seen.clone()], None), Some(&seen));
    }

    #[test]
    fn where_there_is_no_windows_there_are_no_windows() {
        if cfg!(not(windows)) {
            assert!(list().is_empty());
        }
    }
}
