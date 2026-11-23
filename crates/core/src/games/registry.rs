//! Finding the running game on Windows. There is no `/proc` to read each
//! process's environment from; instead Steam itself notes the app it is
//! running in the registry, so a scan is one small read.
//!
//! Steam does not note every game (one started by its own program, or
//! through a launcher, can go unsaid), so the program in front is looked
//! at as well: one whose file is in an installed game's folder is that
//! game, for as long as it runs. The same goes for a program in the folder
//! of a game another launcher installed (`launchers`). And a game the user
//! added by hand is known by its program's name, before any of these.

use super::{Game, launchers, programs, steam};
use crate::config::AddedGame;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// An app whose name was not found is looked up again after this many
/// scans: it may have been installed, or its drive plugged in, since. The
/// list of installed games is read again as often.
const RETRY_NAME_AFTER: u64 = 20;

pub struct Scanner {
    /// Where Steam is looked for again while no installation is known.
    home: Option<PathBuf>,
    steam_roots: Vec<PathBuf>,
    /// Each app's name, and the scan it was looked up in.
    names: HashMap<u32, (Option<String>, u64)>,
    /// The games Steam has installed, and the scan they were listed in.
    installed: Option<(Vec<steam::Installed>, u64)>,
    /// The games other launchers have installed, listed as often. Given
    /// from outside (`with_others`), they are never listed anew.
    others: Option<(Vec<launchers::Found>, u64)>,
    others_given: bool,
    /// The process last seen in front, and the game it turned out to be.
    front: Option<(u32, Option<Game>)>,
    scans: u64,
}

/// What a scan goes by, as Windows gives it.
#[derive(Default)]
pub struct Seen<'a> {
    /// The app Steam says it is running (`RunningAppID`).
    pub steam: Option<u32>,
    /// Every process: its number and its program's file name.
    pub running: &'a [(u32, String)],
    /// The process in front, and where its program is.
    pub front: Option<(u32, &'a str)>,
}

impl Scanner {
    pub fn new(home: &Path) -> Self {
        Self { home: Some(home.into()), ..Self::with_roots(steam::roots(home)) }
    }

    pub fn with_roots(steam_roots: Vec<PathBuf>) -> Self {
        Self { home: None, steam_roots, names: HashMap::new(), installed: None, others: None, others_given: false, front: None, scans: 0 }
    }

    /// With these as the games other launchers have installed, instead of
    /// what this computer's launchers say.
    pub fn with_others(mut self, others: Vec<launchers::Found>) -> Self {
        self.others = Some((others, 0));
        self.others_given = true;
        self
    }

    /// The game that is running now, if one is.
    #[cfg(windows)]
    pub fn scan(&mut self, added: &[AddedGame]) -> Option<Game> {
        let running = programs::names();
        // Where the program in front is, is asked once for each process.
        let front = programs::foreground();
        let path = front.filter(|pid| self.front.as_ref().is_none_or(|(known, _)| known != pid)).and_then(programs::path_of);
        // Steam's note outlives a Steam that crashed: it counts only
        // while Steam itself is running.
        let steam_runs = running.iter().any(|(_, name)| name.eq_ignore_ascii_case("steam.exe"));
        self.saw(Seen { steam: steam::running_appid().filter(|_| steam_runs), running: &running, front: front.map(|pid| (pid, path.as_deref().unwrap_or(""))) }, added)
    }

    /// The game for what was seen: an added game whose program runs, the
    /// app Steam says it is running, or the installed game whose program
    /// is (or was, and still runs) in front. `None` for a tool, and for an
    /// app whose name is not known (yet).
    pub fn saw(&mut self, seen: Seen, added: &[AddedGame]) -> Option<Game> {
        self.scans += 1;
        // Steam installed, or its drive plugged in, after this started.
        if self.steam_roots.is_empty()
            && let Some(home) = &self.home
        {
            self.steam_roots = steam::roots(home);
        }
        if let Some(game) = programs::added(added, seen.running.iter().map(|(_, name)| name.as_str())) {
            return Some(Game { name: game.name.clone(), steam_appid: None });
        }
        if let Some(game) = self.said(seen.steam) {
            return Some(game);
        }
        if let Some((pid, path)) = seen.front
            && self.front.as_ref().is_none_or(|(known, _)| *known != pid)
        {
            let game = steam::game_at(path, self.installed())
                .map(|game| Game { name: game.name.clone(), steam_appid: Some(game.appid) })
                .or_else(|| launchers::game_at(path, self.others()).map(|game| Game { name: game.name.clone(), steam_appid: None }));
            // Another program in front does not end a game still running.
            if game.is_some() || !self.front_runs(seen.running) {
                self.front = Some((pid, game));
            }
        }
        if !self.front_runs(seen.running) {
            self.front = None;
        }
        self.front.as_ref().and_then(|(_, game)| game.clone())
    }

    /// Whether the game found in front is still among the processes.
    fn front_runs(&self, running: &[(u32, String)]) -> bool {
        matches!(&self.front, Some((pid, Some(_))) if running.iter().any(|(p, _)| p == pid))
    }

    /// The game Steam names by its app.
    fn said(&mut self, running: Option<u32>) -> Option<Game> {
        let appid = running.filter(|id| *id != 0)?;
        let (roots, scans) = (&self.steam_roots, self.scans);
        let name = self.names.entry(appid).or_insert_with(|| (steam::app_name(appid, roots), scans));
        if name.0.is_none() && scans - name.1 >= RETRY_NAME_AFTER {
            *name = (steam::app_name(appid, roots), scans);
        }
        Some(Game { name: name.0.clone()?, steam_appid: Some(appid) })
    }

    /// The installed games, listed anew now and then.
    fn installed(&mut self) -> &[steam::Installed] {
        let scans = self.scans;
        if self.installed.as_ref().is_none_or(|(_, listed)| scans - listed >= RETRY_NAME_AFTER) {
            self.installed = Some((steam::installed(&self.steam_roots), scans));
        }
        self.installed.as_ref().map_or(&[], |(games, _)| games.as_slice())
    }
}

impl Scanner {
    /// The games of other launchers, listed anew now and then.
    fn others(&mut self) -> &[launchers::Found] {
        let scans = self.scans;
        if !self.others_given && self.others.as_ref().is_none_or(|(_, listed)| scans - listed >= RETRY_NAME_AFTER) {
            self.others = Some((launchers::installed(), scans));
        }
        self.others.as_ref().map_or(&[], |(games, _)| games.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_from_another_launchers_game_is_that_game_while_it_runs() {
        let (_dir, scanner) = setup();
        let mut scanner = scanner.with_others(vec![launchers::Found::new("Fortnite", r"D:\Epic Games\Fortnite").unwrap()]);
        let all = running(&[(4, "System"), (80, "FortniteClient-Win64-Shipping.exe"), (60, "chrome.exe")]);
        let fortnite = Some(Game { name: "Fortnite".into(), steam_appid: None });
        let mut saw = |front: Option<(u32, &str)>, running: &[(u32, String)]| scanner.saw(Seen { steam: None, running, front }, &[]);
        assert_eq!(saw(Some((60, "C:\\Program Files\\chrome.exe")), &all), None);
        assert_eq!(saw(Some((80, r"D:\Epic Games\Fortnite\FortniteGame\Binaries\Win64\FortniteClient-Win64-Shipping.exe")), &all), fortnite);
        assert_eq!(saw(Some((60, "C:\\Program Files\\chrome.exe")), &all), fortnite, "still running behind the browser");
        assert_eq!(saw(None, &running(&[(4, "System"), (60, "chrome.exe")])), None);
    }

    fn setup() -> (tempfile::TempDir, Scanner) {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("steam/steamapps");
        std::fs::create_dir_all(&apps).unwrap();
        for (id, name) in [(730, "Counter-Strike 2"), (228980, "Steamworks Common Redistributables")] {
            std::fs::write(apps.join(format!("appmanifest_{id}.acf")), format!("\t\"name\"\t\t\"{name}\"\n\t\"installdir\"\t\t\"{name}\"\n")).unwrap();
        }
        let scanner = Scanner::with_roots(vec![dir.path().join("steam")]);
        (dir, scanner)
    }

    #[test]
    fn the_app_steam_names_is_the_game_and_none_is_no_game() {
        let (_dir, mut scanner) = setup();
        assert_eq!(scanner.saw(Seen { steam: None, ..Seen::default() }, &[]), None);
        assert_eq!(scanner.saw(Seen { steam: Some(0), ..Seen::default() }, &[]), None, "Steam writes 0 when nothing runs");
        assert_eq!(scanner.saw(Seen { steam: Some(730), ..Seen::default() }, &[]), Some(Game { name: "Counter-Strike 2".into(), steam_appid: Some(730) }));
        assert_eq!(scanner.saw(Seen { steam: Some(0), ..Seen::default() }, &[]), None);
    }

    #[test]
    fn tools_and_apps_that_are_not_installed_are_not_games() {
        let (_dir, mut scanner) = setup();
        assert_eq!(scanner.saw(Seen { steam: Some(228980), ..Seen::default() }, &[]), None);
        assert_eq!(scanner.saw(Seen { steam: Some(4_000_000_000), ..Seen::default() }, &[]), None, "a shortcut that is not a Steam app");
    }

    #[test]
    fn a_game_installed_while_it_is_looked_for_is_found_after_a_while() {
        let (dir, mut scanner) = setup();
        assert_eq!(scanner.saw(Seen { steam: Some(570), ..Seen::default() }, &[]), None);
        std::fs::write(dir.path().join("steam/steamapps/appmanifest_570.acf"), "\t\"name\"\t\t\"Dota 2\"\n").unwrap();
        assert_eq!(scanner.saw(Seen { steam: Some(570), ..Seen::default() }, &[]), None, "the manifests are not read at every scan");
        let found: Vec<bool> = (0..RETRY_NAME_AFTER).map(|_| scanner.saw(Seen { steam: Some(570), ..Seen::default() }, &[]).is_some()).collect();
        assert_eq!(found.last(), Some(&true), "{found:?}");
    }

    fn running(programs: &[(u32, &str)]) -> Vec<(u32, String)> {
        programs.iter().map(|(pid, name)| (*pid, name.to_string())).collect()
    }

    #[test]
    fn a_game_added_by_hand_is_known_by_its_program_and_comes_first() {
        let (_dir, mut scanner) = setup();
        let added = [AddedGame { name: "Order of the Sinking Star".into(), program: "star.exe".into() }];
        let programs = running(&[(4, "System"), (90, "Star.EXE")]);
        let seen = || Seen { steam: Some(730), running: &programs, front: None };
        assert_eq!(scanner.saw(seen(), &added), Some(Game { name: "Order of the Sinking Star".into(), steam_appid: None }));
        assert_eq!(scanner.saw(seen(), &[]).unwrap().name, "Counter-Strike 2");
        assert_eq!(scanner.saw(Seen { steam: None, running: &running(&[(4, "System")]), front: None }, &added), None);
    }

    #[test]
    fn a_program_in_front_from_a_games_folder_is_that_game_while_it_runs() {
        let (dir, mut scanner) = setup();
        let game = dir.path().join("steam/steamapps/common/Counter-Strike 2/bin/cs2.exe").to_string_lossy().into_owned();
        let tool = dir.path().join("steam/steamapps/common/Steamworks Common Redistributables/setup.exe").to_string_lossy().into_owned();
        let all = running(&[(4, "System"), (50, "cs2.exe"), (60, "chrome.exe"), (70, "setup.exe")]);
        let cs2 = Some(Game { name: "Counter-Strike 2".into(), steam_appid: Some(730) });
        let mut saw = |front: Option<(u32, &str)>, running: &[(u32, String)]| scanner.saw(Seen { steam: None, running, front }, &[]);

        assert_eq!(saw(Some((60, "C:\\Program Files\\chrome.exe")), &all), None, "a program from elsewhere");
        assert_eq!(saw(Some((70, &tool)), &all), None, "a tool is not a game");
        assert_eq!(saw(Some((50, &game)), &all), cs2);
        // Its path is asked for once: after that the process is known.
        assert_eq!(saw(Some((50, "")), &all), cs2);
        // Something else in front, and nothing: the game still runs.
        assert_eq!(saw(Some((60, "C:\\Program Files\\chrome.exe")), &all), cs2);
        assert_eq!(saw(None, &all), cs2);
        // Gone when its process is.
        assert_eq!(saw(None, &running(&[(4, "System"), (60, "chrome.exe")])), None);
        assert_eq!(saw(Some((60, "")), &all), None);
    }
}
