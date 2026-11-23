//! Games that did not come through Steam: where the other launchers have
//! installed theirs. A program running from one of those folders is that
//! game, the same way a program in a Steam game's folder is (`registry`).
//!
//! Each launcher keeps its list its own way:
//!
//! - Epic Games: a small JSON file for each install, under ProgramData.
//! - GOG Galaxy: a key for each game in the registry.
//! - Ubisoft Connect: a key for each install, with its folder only.
//! - Battle.net, the EA app, Riot and Rockstar: nothing of their own that
//!   is simple to read, but each game is in Windows' list of installed
//!   programs, where the way it is uninstalled names its launcher.
//! - Xbox (and Game Pass): a folder `XboxGames` on a drive, one folder to
//!   a game.
//!
//! Reading them is the small Windows part at the end; what is made of
//! what was read is plain logic, tried out on every system.

use std::path::{Path, PathBuf};

/// A game a launcher has installed, and the folder its files are in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub dir: PathBuf,
    /// The folder, spelled for comparing.
    folded: String,
}

impl Found {
    /// A game by its name and folder; nothing when either says too
    /// little: no name, or a folder that is a whole drive or nearly
    /// (everything on it would be "the game").
    pub fn new(name: &str, dir: &str) -> Option<Self> {
        let name = name.trim();
        let dir = dir.trim().trim_matches('"').trim_end_matches(['\\', '/']);
        let deep = dir.split(['\\', '/']).filter(|part| !part.is_empty()).count() >= 2;
        (!name.is_empty() && deep).then(|| Self { name: name.to_string(), dir: PathBuf::from(dir), folded: super::steam::folded(dir, true) })
    }
}

/// The game a program belongs to: the one whose folder the program's
/// file is in, however deep.
pub fn game_at<'a>(program: &str, found: &'a [Found]) -> Option<&'a Found> {
    let program = super::steam::folded(program, false);
    found.iter().find(|game| program.starts_with(&game.folded))
}

/// The game an Epic Games manifest (`*.item`) is for. The launcher
/// installs other things the same way (the Unreal Engine, tools): only
/// what it files under games counts.
pub fn epic(manifest: &str) -> Option<Found> {
    let item: serde_json::Value = serde_json::from_str(manifest).ok()?;
    let is_game = item["AppCategories"].as_array().is_none_or(|kinds| kinds.iter().any(|kind| kind == "games"));
    // An add-on has a manifest of its own, in its game's folder.
    let whole = item["MainGameAppName"].as_str().is_none_or(|main| main.is_empty() || Some(main) == item["AppName"].as_str());
    if !is_game || !whole {
        return None;
    }
    Found::new(item["DisplayName"].as_str()?, item["InstallLocation"].as_str()?)
}

/// Whether the way a program is uninstalled (its `UninstallString` in
/// Windows' list of installed programs) names one of the game launchers
/// that keep no simpler list: such a program is one of that launcher's games.
pub fn uninstalled_by_a_launcher(uninstall: &str) -> bool {
    let uninstall = uninstall.to_lowercase();
    ["battle.net", "blizzard uninstaller", "riotclientservices", "eainstaller", "ea desktop", "rockstar games\\launcher"].iter().any(|mark| uninstall.contains(mark))
}

/// The name an Xbox game gives itself in its `MicrosoftGame.config`,
/// when it is written out there (some only point to where it is kept).
pub fn xbox_name(config: &str) -> Option<String> {
    let at = config.find("DefaultDisplayName=\"")? + "DefaultDisplayName=\"".len();
    let name = config[at..].split('"').next()?.trim();
    (!name.is_empty() && !name.starts_with("ms-resource:")).then(|| name.replace("&amp;", "&").replace("&apos;", "'"))
}

/// The games in an `XboxGames` folder: each folder in it is one, called
/// what its own file says or, failing that, what its folder is called.
pub fn xbox(folder: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let named = std::fs::read_to_string(dir.join("Content").join("MicrosoftGame.config")).ok().and_then(|config| xbox_name(&config));
        let name = named.unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
        found.extend(Found::new(&name, &dir.to_string_lossy()));
    }
    found
}

/// The games in a folder of Epic's manifests.
pub fn epic_manifests(folder: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        if entry.path().extension().is_some_and(|end| end == "item") {
            found.extend(std::fs::read_to_string(entry.path()).ok().and_then(|text| epic(&text)));
        }
    }
    found
}

/// Each folder once, and none that is inside another of the list (a
/// launcher's own folder beside its game's, an add-on in its game).
fn tidy(found: Vec<Found>) -> Vec<Found> {
    let mut kept: Vec<Found> = Vec::new();
    for game in found {
        if !kept.iter().any(|other| game.folded.starts_with(&other.folded)) {
            kept.retain(|other| !other.folded.starts_with(&game.folded));
            kept.push(game);
        }
    }
    kept
}

/// Every game the other launchers have installed on this computer.
#[cfg(windows)]
pub fn installed() -> Vec<Found> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let mut found = Vec::new();

    let data = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    found.extend(epic_manifests(&data.join(r"Epic\EpicGamesLauncher\Data\Manifests")));

    // Every key under `key`, in both halves of the registry a 64-bit
    // Windows keeps for programs, and for this user alone.
    let each = |key: &str, with: &mut dyn FnMut(&RegKey)| {
        for (hive, prefix) in [(HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node"), (HKEY_LOCAL_MACHINE, "SOFTWARE"), (HKEY_CURRENT_USER, "SOFTWARE")] {
            let Ok(list) = RegKey::predef(hive).open_subkey(format!(r"{prefix}\{key}")) else { continue };
            for name in list.enum_keys().flatten() {
                if let Ok(entry) = list.open_subkey(&name) {
                    (with)(&entry);
                }
            }
        }
    };
    let text = |entry: &RegKey, name: &str| entry.get_value::<String, _>(name).ok().filter(|value| !value.trim().is_empty());

    each(r"GOG.com\Games", &mut |game| {
        // An add-on names the game it belongs to.
        if text(game, "dependsOn").is_none()
            && let (Some(name), Some(dir)) = (text(game, "gameName"), text(game, "path"))
        {
            found.extend(Found::new(&name, &dir));
        }
    });
    each(r"Ubisoft\Launcher\Installs", &mut |install| {
        if let Some(dir) = text(install, "InstallDir") {
            // Only the folder is noted: the game is called what it is.
            let name = dir.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or_default().to_string();
            found.extend(Found::new(&name, &dir));
        }
    });
    each(r"Microsoft\Windows\CurrentVersion\Uninstall", &mut |program| {
        if text(program, "UninstallString").is_some_and(|how| uninstalled_by_a_launcher(&how))
            && let (Some(name), Some(dir)) = (text(program, "DisplayName"), text(program, "InstallLocation"))
        {
            found.extend(Found::new(&name, &dir));
        }
    });

    for letter in 'C'..='Z' {
        found.extend(xbox(Path::new(&format!(r"{letter}:\XboxGames"))));
    }
    // What is no longer there is not a game any more.
    tidy(found.into_iter().filter(|game| game.dir.is_dir()).collect())
}

/// Elsewhere, games are known by what Steam put into their surroundings
/// (`process`): there is nothing to list.
#[cfg(not(windows))]
pub fn installed() -> Vec<Found> {
    tidy(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_epic_manifest_names_a_game_and_its_folder() {
        let fortnite = r#"{"FormatVersion":0,"DisplayName":"Fortnite","InstallLocation":"D:\\Epic Games\\Fortnite","AppName":"Fortnite","MainGameAppName":"Fortnite","AppCategories":["public","games","applications"]}"#;
        let found = epic(fortnite).unwrap();
        assert_eq!((found.name.as_str(), found.dir.to_str()), ("Fortnite", Some(r"D:\Epic Games\Fortnite")));
        let engine = r#"{"DisplayName":"Unreal Engine","InstallLocation":"C:\\Program Files\\Epic Games\\UE_5.4","AppName":"UE_5.4","AppCategories":["public","engines"]}"#;
        assert_eq!(epic(engine), None, "the launcher installs more than games");
        let addon = r#"{"DisplayName":"A Pack","InstallLocation":"D:\\Epic Games\\Fortnite","AppName":"Pack","MainGameAppName":"Fortnite","AppCategories":["games"]}"#;
        assert_eq!(epic(addon), None, "an add-on is not a game of its own");
        assert_eq!(epic(r#"{"DisplayName":"Nowhere","InstallLocation":""}"#), None);
        assert_eq!(epic("not json"), None);
    }

    #[test]
    fn a_program_in_a_found_games_folder_is_that_game() {
        let found = [Found::new("Fortnite", r"D:\Epic Games\Fortnite\").unwrap(), Found::new("Cyberpunk 2077", "E:/GOG Games/Cyberpunk 2077").unwrap()];
        let game = |program: &str| game_at(program, &found).map(|game| game.name.as_str());
        assert_eq!(game(r"d:\epic games\fortnite\FortniteGame\Binaries\Win64\FortniteClient-Win64-Shipping.exe"), Some("Fortnite"));
        assert_eq!(game(r"\\?\E:\GOG Games\Cyberpunk 2077\bin\x64\Cyberpunk2077.exe"), Some("Cyberpunk 2077"));
        assert_eq!(game(r"D:\Epic Games\Fortnite 2\game.exe"), None, "a folder that only begins the same");
        assert_eq!(game(r"C:\Windows\explorer.exe"), None);
    }

    #[test]
    fn a_whole_drive_is_never_a_games_folder() {
        assert_eq!(Found::new("Game", r"C:\"), None);
        assert_eq!(Found::new("Game", "D:"), None);
        assert_eq!(Found::new("Game", ""), None);
        assert_eq!(Found::new("  ", r"D:\Games\Game"), None);
        assert!(Found::new("Game", r#""D:\Games\Game""#).is_some(), "the registry writes some with quotes");
    }

    #[test]
    fn launchers_are_known_by_how_their_games_are_uninstalled() {
        assert!(uninstalled_by_a_launcher(r#""C:\ProgramData\Battle.net\Agent\Blizzard Uninstaller.exe" --lang=enUS --uid=prometheus"#));
        assert!(uninstalled_by_a_launcher(r#""C:\Riot Games\Riot Client\RiotClientServices.exe" --uninstall-product=valorant --uninstall-patchline=live"#));
        assert!(uninstalled_by_a_launcher(r#""C:\Program Files\Common Files\EAInstaller\Apex\Cleanup.exe" uninstall_game -autologging"#));
        assert!(uninstalled_by_a_launcher(r#""C:\Program Files\Rockstar Games\Launcher\Launcher.exe" -uninstall=gta5"#));
        assert!(!uninstalled_by_a_launcher(r#"MsiExec.exe /X{90160000-0011-0000-0000-0000000FF1CE}"#));
        assert!(!uninstalled_by_a_launcher(r#""C:\Program Files (x86)\Steam\steam.exe" steam://uninstall/730"#), "Steam's are found through Steam");
    }

    #[test]
    fn xbox_games_are_the_folders_of_their_folder() {
        assert_eq!(xbox_name(r#"<ShellVisuals DefaultDisplayName="Forza Horizon 5" PublisherDisplayName="x" />"#).as_deref(), Some("Forza Horizon 5"));
        assert_eq!(xbox_name(r#"<ShellVisuals DefaultDisplayName="ms-resource:AppName" />"#), None);
        assert_eq!(xbox_name(r#"<ShellVisuals DefaultDisplayName="Ori &amp; the Will" />"#).as_deref(), Some("Ori & the Will"));

        let dir = tempfile::tempdir().unwrap();
        let games = dir.path().join("XboxGames");
        std::fs::create_dir_all(games.join("Forza Horizon 5/Content")).unwrap();
        std::fs::write(games.join("Forza Horizon 5/Content/MicrosoftGame.config"), r#"<Game><ShellVisuals DefaultDisplayName="Forza Horizon 5"/></Game>"#).unwrap();
        std::fs::create_dir_all(games.join("Halo Infinite/Content")).unwrap();
        std::fs::write(games.join(".GamingRoot"), "RGBX").unwrap();
        let mut names: Vec<String> = xbox(&games).into_iter().map(|game| game.name).collect();
        names.sort();
        assert_eq!(names, ["Forza Horizon 5", "Halo Infinite"]);
        assert!(xbox(&dir.path().join("nothing here")).is_empty());
    }

    #[test]
    fn epic_manifests_are_read_from_their_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A1.item"), r#"{"DisplayName":"Alan Wake 2","InstallLocation":"D:\\Epic\\AlanWake2","AppCategories":["games"]}"#).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a manifest").unwrap();
        std::fs::write(dir.path().join("B2.item"), "{ broken").unwrap();
        assert_eq!(epic_manifests(dir.path()).into_iter().map(|game| game.name).collect::<Vec<_>>(), ["Alan Wake 2"]);
    }

    #[test]
    fn a_folder_is_listed_once_and_not_inside_another() {
        let game = |name: &str, dir: &str| Found::new(name, dir).unwrap();
        let found = tidy(vec![game("Pack", r"D:\Games\Diablo\Pack"), game("Diablo", r"D:\Games\Diablo"), game("Diablo again", r"d:\games\diablo\"), game("Other", r"D:\Games\Other")]);
        assert_eq!(found.iter().map(|game| game.name.as_str()).collect::<Vec<_>>(), ["Diablo", "Other"]);
    }
}
