//! Steam's own files, read offline: which app a process belongs to, what
//! the app is called, and where its cover is.

use std::path::{Path, PathBuf};

/// The Steam app a process was launched for, from its environment block
/// (`/proc/<pid>/environ`: `KEY=value` entries separated by NUL).
pub fn appid_from_environ(environ: &[u8]) -> Option<u32> {
    let mut found = None;
    for entry in environ.split(|b| *b == 0) {
        for key in [&b"SteamAppId="[..], b"SteamGameId="] {
            if let Some(v) = entry.strip_prefix(key) {
                // Non-Steam shortcuts carry a 64-bit id that is not an app.
                match std::str::from_utf8(v).ok().and_then(|s| s.parse::<u32>().ok()) {
                    Some(0) | None => {}
                    Some(id) => found = found.or(Some(id)),
                }
            }
        }
    }
    found
}

/// A value Steam keeps under its key in the registry.
#[cfg(windows)]
fn registry<T: winreg::types::FromRegValue>(name: &str) -> Option<T> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam").ok()?.get_value(name).ok()
}

/// The app Steam is running now, as it notes it in the registry
/// (`RunningAppID`, 0 while none runs).
#[cfg(windows)]
pub fn running_appid() -> Option<u32> {
    registry::<u32>("RunningAppID").filter(|id| *id != 0)
}

/// Where Steam says it is installed (`SteamPath`), which need not be a
/// program folder.
#[cfg(windows)]
fn installed_at() -> Option<PathBuf> {
    registry::<String>("SteamPath").filter(|p| !p.is_empty()).map(PathBuf::from)
}

#[cfg(not(windows))]
fn installed_at() -> Option<PathBuf> {
    None
}

/// The Steam installations: in a home directory on Linux (native,
/// Flatpak); where Steam says it is, and in the program folders, on Windows.
pub fn roots(home: &Path) -> Vec<PathBuf> {
    let places: Vec<PathBuf> = if cfg!(windows) {
        let usual = ["ProgramFiles(x86)", "ProgramFiles"].iter().filter_map(|var| Some(PathBuf::from(std::env::var_os(var)?).join("Steam")));
        installed_at().into_iter().chain(usual).collect()
    } else {
        [".local/share/Steam", ".steam/steam", ".var/app/com.valvesoftware.Steam/.local/share/Steam"].iter().map(|p| home.join(p)).collect()
    };
    places
        .into_iter()
        .filter(|p| p.join("steamapps").is_dir())
        // Windows spells a full path with `\\?\` in front, which other
        // programs (and a path from Steam's own files) do not.
        .filter_map(|p| p.canonicalize().ok().map(|p| p.to_str().and_then(|s| s.strip_prefix(r"\\?\")).filter(|s| !s.starts_with("UNC")).map_or(p.clone(), PathBuf::from)))
        .fold(Vec::new(), |mut v, p| {
            if !v.contains(&p) {
                v.push(p);
            }
            v
        })
}

/// The value of `"key"  "value"` on a line of Valve's text format.
fn vdf_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix('"')?.strip_prefix(key)?.strip_prefix('"')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    rest.strip_suffix('"')
}

/// Every `steamapps` folder of an installation: its own and each extra library's.
pub fn library_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.join("steamapps")];
    if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        for line in text.lines() {
            if let Some(path) = vdf_value(line, "path") {
                let dir = PathBuf::from(path.replace("\\\\", "\\")).join("steamapps");
                if dir.is_dir() && !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
    }
    dirs
}

/// Runtimes and tools Steam starts beside a game, which are not games.
fn is_tool(name: &str) -> bool {
    ["Proton", "Steam Linux Runtime", "Steamworks Common", "SteamVR"].iter().any(|t| name.starts_with(t))
}

/// An installed app's name, from its manifest; or, for a game added to
/// Steam by hand, the name it was added under. `None` for tools and for
/// apps that are not installed.
pub fn app_name(appid: u32, roots: &[PathBuf]) -> Option<String> {
    for root in roots {
        for dir in library_dirs(root) {
            let Ok(text) = std::fs::read_to_string(dir.join(format!("appmanifest_{appid}.acf"))) else { continue };
            let name = text.lines().find_map(|l| vdf_value(l, "name"))?;
            return (!is_tool(name) && !name.is_empty()).then(|| name.to_string());
        }
    }
    shortcuts(roots).into_iter().find(|(id, _)| *id == appid).map(|(_, name)| name)
}

/// A game Steam has installed, and the folder its files are in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub appid: u32,
    pub name: String,
    /// The folder, spelled for comparing: see `folded`.
    dir: String,
}

/// A path for comparing, however it was written: lower case, backward
/// slashes, without the `\\?\` Windows puts before a full path, and with
/// a slash at its end when it is a folder.
pub(super) fn folded(path: &str, folder: bool) -> String {
    let path = path.replace('/', "\\").to_lowercase();
    let path = path.strip_prefix("\\\\?\\").unwrap_or(&path).trim_end_matches('\\').to_string();
    if folder { path + "\\" } else { path }
}

/// Every game Steam has installed (tools are not games), from the
/// manifests of every library.
pub fn installed(roots: &[PathBuf]) -> Vec<Installed> {
    let mut out = Vec::new();
    for root in roots {
        for dir in library_dirs(root) {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let file = entry.file_name().to_string_lossy().into_owned();
                let Some(appid) = file.strip_prefix("appmanifest_").and_then(|f| f.strip_suffix(".acf")).and_then(|id| id.parse().ok()) else { continue };
                let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
                let value = |key| text.lines().find_map(|l| vdf_value(l, key)).filter(|v| !v.is_empty());
                if let (Some(name), Some(folder)) = (value("name"), value("installdir"))
                    && !is_tool(name)
                {
                    let dir = folded(&dir.join("common").join(folder).to_string_lossy(), true);
                    out.push(Installed { appid, name: name.to_string(), dir });
                }
            }
        }
    }
    out
}

/// The installed game a program belongs to: the one whose folder the
/// program's file is in, however deep.
pub fn game_at<'a>(program: &str, installed: &'a [Installed]) -> Option<&'a Installed> {
    let program = folded(program, false);
    installed.iter().find(|game| program.starts_with(&game.dir))
}

/// The games in a `shortcuts.vdf`: the ones added to Steam by hand
/// ("non-Steam games"), each with the id Steam gave it and the name it
/// goes by. The file is Steam's binary form of its text format: a field is
/// a type byte, its key, a NUL, and its value (four bytes for a number, a
/// NUL-ended text otherwise). Only the two fields needed are looked for.
pub fn parse_shortcuts(bytes: &[u8]) -> Vec<(u32, String)> {
    // Where `key` (as a field of the given type) next is, whatever its case.
    let field = |from: usize, kind: u8, key: &[u8]| -> Option<usize> {
        let wanted = key.len() + 2;
        (from..bytes.len().saturating_sub(wanted - 1))
            .find(|&at| bytes[at] == kind && bytes[at + 1..at + 1 + key.len()].eq_ignore_ascii_case(key) && bytes[at + 1 + key.len()] == 0)
            .map(|at| at + wanted)
    };
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(value) = field(at, 0x02, b"appid") {
        let Some(id) = bytes.get(value..value + 4) else { break };
        let id = u32::from_le_bytes([id[0], id[1], id[2], id[3]]);
        at = value + 4;
        // Its name is before the next shortcut begins.
        let next = field(at, 0x02, b"appid").unwrap_or(bytes.len());
        if let Some(start) = field(at, 0x01, b"appname").filter(|start| *start < next) {
            let end = bytes[start..].iter().position(|b| *b == 0).map_or(bytes.len(), |n| start + n);
            let name = String::from_utf8_lossy(&bytes[start..end]);
            // Added by picking a program, a game is named after its file.
            let name = name.trim();
            // (Cut by characters, not bytes: a name in another script must
            // not be cut through the middle of a letter.)
            let stem = name.len().checked_sub(4).filter(|at| name.is_char_boundary(*at) && name[*at..].eq_ignore_ascii_case(".exe"));
            let name = stem.map_or(name, |at| &name[..at]);
            if id != 0 && !name.trim().is_empty() {
                out.push((id, name.trim().to_string()));
            }
        }
    }
    out
}

/// The games added to Steam by hand, of every Steam user on this computer.
pub fn shortcuts(roots: &[PathBuf]) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for root in roots {
        for user in std::fs::read_dir(root.join("userdata")).into_iter().flatten().flatten() {
            if let Ok(bytes) = std::fs::read(user.path().join("config/shortcuts.vdf")) {
                out.extend(parse_shortcuts(&bytes));
            }
        }
    }
    out
}

/// The installed app with this name (as a clip folder spells it), for
/// finding a game's cover from the folder its clips are in. Reading every
/// manifest is slow and this is asked for every game on every look at the
/// library, so an answer is remembered for a while.
pub fn appid_by_name(folder: &str, roots: &[PathBuf]) -> Option<u32> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    type Known = HashMap<(Vec<PathBuf>, String), (Option<u32>, Instant)>;
    static KNOWN: OnceLock<Mutex<Known>> = OnceLock::new();
    // A game that was not found may be installed any moment; one that was stays.
    let fresh = |found: &Option<u32>, at: &Instant| at.elapsed() < Duration::from_secs(if found.is_some() { 600 } else { 60 });
    let key = (roots.to_vec(), folder.to_string());
    let mut known = KNOWN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((found, at)) = known.get(&key)
        && fresh(found, at)
    {
        return *found;
    }
    let found = find_appid_by_name(folder, roots);
    known.insert(key, (found, Instant::now()));
    found
}

fn find_appid_by_name(folder: &str, roots: &[PathBuf]) -> Option<u32> {
    for root in roots {
        for dir in library_dirs(root) {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let file = entry.file_name().to_string_lossy().into_owned();
                let Some(id) = file.strip_prefix("appmanifest_").and_then(|f| f.strip_suffix(".acf")) else { continue };
                let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
                let name = text.lines().find_map(|l| vdf_value(l, "name"));
                if name.is_some_and(|n| super::folder_name(n) == folder) {
                    return id.parse().ok();
                }
            }
        }
    }
    shortcuts(roots).into_iter().find(|(_, name)| super::folder_name(name) == folder).map(|(id, _)| id)
}

/// The portrait cover Steam keeps for an app, if it has one.
pub fn cover(appid: u32, roots: &[PathBuf]) -> Option<PathBuf> {
    const NAME: &str = "library_600x900.jpg";
    for root in roots {
        let cache = root.join("appcache/librarycache");
        // The old layout, then the current one (a folder per app, sometimes
        // with the picture one hashed folder further down).
        let flat = cache.join(format!("{appid}_{NAME}"));
        if flat.is_file() {
            return Some(flat);
        }
        let dir = cache.join(appid.to_string());
        if dir.join(NAME).is_file() {
            return Some(dir.join(NAME));
        }
        for sub in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = sub.path().join(NAME);
            if p.is_file() {
                return Some(p);
            }
        }
        // A game added by hand has the cover its user gave it, if any.
        for user in std::fs::read_dir(root.join("userdata")).into_iter().flatten().flatten() {
            for ext in ["png", "jpg", "jpeg", "webp"] {
                let p = user.path().join(format!("config/grid/{appid}p.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `shortcuts.vdf` with two games, as Steam writes it.
    fn shortcuts_file() -> Vec<u8> {
        let mut b = b"\x00shortcuts\x00".to_vec();
        for (index, id, name) in [("0", 3616217283u32, "STRAFE.exe"), ("1", 3680450140, "AnyMaker")] {
            b.extend(b"\x00");
            b.extend(index.as_bytes());
            b.extend(b"\x00\x02appid\x00");
            b.extend(id.to_le_bytes());
            b.extend(b"\x01AppName\x00");
            b.extend(name.as_bytes());
            b.extend(b"\x00\x01Exe\x00\"/games/x\"\x00\x02IsHidden\x00\x00\x00\x00\x00\x00tags\x00\x08\x08");
        }
        b.extend(b"\x08\x08");
        b
    }

    #[test]
    fn games_added_to_steam_by_hand_are_read_from_its_shortcuts() {
        assert_eq!(parse_shortcuts(&shortcuts_file()), [(3616217283, "STRAFE".to_string()), (3680450140, "AnyMaker".to_string())]);
        assert!(parse_shortcuts(b"").is_empty());
        assert!(parse_shortcuts(b"\x00shortcuts\x00\x08\x08").is_empty());
        // Cut short in the middle of a number: what came before still counts.
        let mut cut = shortcuts_file();
        cut.extend(b"\x00\x002\x00\x02appid\x00\x01\x02");
        assert_eq!(parse_shortcuts(&cut).len(), 2);
    }

    #[test]
    fn a_shortcut_named_in_another_script_is_read_whole() {
        // Four bytes from the end of these is the middle of a letter.
        for name in ["原神", "ab日本", "Ys Ⅷ", "日本.exe"] {
            let mut b = b"\x00shortcuts\x00\x000\x00\x02appid\x00".to_vec();
            b.extend(7u32.to_le_bytes());
            b.extend(b"\x01AppName\x00");
            b.extend(name.as_bytes());
            b.extend(b"\x00\x08\x08\x08\x08");
            assert_eq!(parse_shortcuts(&b), [(7, name.trim_end_matches(".exe").to_string())], "{name}");
        }
    }

    #[test]
    fn a_game_added_by_hand_has_a_name_and_a_cover() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let config = root.join("userdata/84842704/config");
        std::fs::create_dir_all(config.join("grid")).unwrap();
        std::fs::write(config.join("shortcuts.vdf"), shortcuts_file()).unwrap();
        let roots = [root];
        assert_eq!(app_name(3616217283, &roots), Some("STRAFE".into()));
        assert_eq!(app_name(42, &roots), None);
        assert_eq!(find_appid_by_name("STRAFE", &roots), Some(3616217283));
        assert_eq!(cover(3616217283, &roots), None);
        std::fs::write(config.join("grid/3616217283p.png"), b"").unwrap();
        assert_eq!(cover(3616217283, &roots), Some(config.join("grid/3616217283p.png")));
    }

    #[test]
    fn the_app_id_is_read_from_the_environment() {
        assert_eq!(appid_from_environ(b"HOME=/h\0SteamAppId=730\0X=1\0"), Some(730));
        assert_eq!(appid_from_environ(b"SteamGameId=1091500\0"), Some(1091500));
        assert_eq!(appid_from_environ(b"SteamAppId=0\0SteamGameId=570\0"), Some(570));
        assert_eq!(appid_from_environ(b"SteamGameId=13906274329824329728\0"), None, "a non-Steam shortcut");
        assert_eq!(appid_from_environ(b"NotSteamAppId=730\0"), None);
        assert_eq!(appid_from_environ(b""), None);
    }

    fn steam(dir: &Path) -> PathBuf {
        let root = dir.join(".local/share/Steam");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        root
    }

    // Steam in a home folder is how Linux has it.
    #[cfg(unix)]
    #[test]
    fn names_come_from_the_manifests_of_every_library() {
        let dir = tempfile::tempdir().unwrap();
        let root = steam(dir.path());
        let extra = dir.path().join("games/SteamLibrary");
        std::fs::create_dir_all(extra.join("steamapps")).unwrap();
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!("\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n", extra.display()),
        )
        .unwrap();
        let manifest = |name: &str| format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"1\"\n\t\"name\"\t\t\"{name}\"\n}}\n");
        std::fs::write(root.join("steamapps/appmanifest_730.acf"), manifest("Counter-Strike 2")).unwrap();
        std::fs::write(extra.join("steamapps/appmanifest_1091500.acf"), manifest("Cyberpunk 2077")).unwrap();
        std::fs::write(root.join("steamapps/appmanifest_2805730.acf"), manifest("Proton 9.0")).unwrap();

        let roots = roots(dir.path());
        assert_eq!(roots.len(), 1);
        assert_eq!(app_name(730, &roots).as_deref(), Some("Counter-Strike 2"));
        assert_eq!(app_name(1091500, &roots).as_deref(), Some("Cyberpunk 2077"));
        assert_eq!(app_name(2805730, &roots), None, "a tool is not a game");
        assert_eq!(app_name(42, &roots), None);
        assert_eq!(appid_by_name("Cyberpunk 2077", &roots), Some(1091500));
        assert_eq!(appid_by_name("Nothing", &roots), None);
    }

    #[test]
    fn a_program_in_a_games_folder_belongs_to_that_game() {
        let dir = tempfile::tempdir().unwrap();
        let root = steam(dir.path());
        let apps = root.join("steamapps");
        let manifest = |name: &str, folder: &str| format!("\t\"name\"\t\t\"{name}\"\n\t\"installdir\"\t\t\"{folder}\"\n");
        std::fs::write(apps.join("appmanifest_499170.acf"), manifest("Order of the Sinking Star", "Sinking Star")).unwrap();
        std::fs::write(apps.join("appmanifest_730.acf"), manifest("Counter-Strike 2", "Counter-Strike Global Offensive")).unwrap();
        std::fs::write(apps.join("appmanifest_228980.acf"), manifest("Steamworks Common Redistributables", "Steamworks Shared")).unwrap();
        std::fs::write(apps.join("appmanifest_1.acf"), "\t\"name\"\t\t\"No folder\"\n").unwrap();
        let games = installed(std::slice::from_ref(&root));
        assert_eq!(games.len(), 2, "{games:?}");
        let at = |program: &std::path::Path| game_at(&program.to_string_lossy(), &games).map(|g| (g.appid, g.name.clone()));
        let star = apps.join("common/Sinking Star");
        assert_eq!(at(&star.join("star.exe")), Some((499170, "Order of the Sinking Star".into())));
        assert_eq!(at(&star.join("bin/win64/star.exe")), Some((499170, "Order of the Sinking Star".into())));
        // Windows writes the same place in more than one way.
        let shouted = star.join("STAR.EXE").to_string_lossy().to_uppercase().replace('/', "\\");
        assert_eq!(game_at(&format!("\\\\?\\{shouted}"), &games).map(|g| g.appid), Some(499170));
        assert_eq!(at(&apps.join("common/Sinking Star 2/star.exe")), None, "a folder that only begins the same");
        assert_eq!(at(&apps.join("common/Steamworks Shared/setup.exe")), None, "a tool is not a game");
        assert_eq!(at(std::path::Path::new("/usr/bin/zsh")), None);
    }

    #[test]
    fn covers_are_found_in_each_layout() {
        let dir = tempfile::tempdir().unwrap();
        let root = steam(dir.path());
        let cache = root.join("appcache/librarycache");
        std::fs::create_dir_all(cache.join("730/abc123")).unwrap();
        std::fs::create_dir_all(cache.join("570")).unwrap();
        std::fs::write(cache.join("730/abc123/library_600x900.jpg"), b"x").unwrap();
        std::fs::write(cache.join("570/library_600x900.jpg"), b"x").unwrap();
        std::fs::write(cache.join("440_library_600x900.jpg"), b"x").unwrap();
        let roots = vec![root];
        assert!(cover(730, &roots).unwrap().ends_with("730/abc123/library_600x900.jpg"));
        assert!(cover(570, &roots).is_some());
        assert!(cover(440, &roots).is_some());
        assert_eq!(cover(1, &roots), None);
    }
}
