//! Finding the running game among the processes. Each process is looked at
//! once, when it first appears, so a scan every few seconds costs a
//! directory listing, plus two small reads for each process that is new.
//! Now and then every process is looked at again, to catch a process number
//! that was given to another process in the meantime.

use super::{Game, programs, steam};
use crate::config::AddedGame;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Every this many scans, the processes already known are checked to still
/// be the ones they were.
pub const VERIFY_EVERY: u64 = 20;
/// An app whose name was not found is looked up again after this many
/// scans: it may have been installed, or its drive plugged in, since.
const RETRY_NAME_AFTER: u64 = 20;

pub struct Scanner {
    proc_root: PathBuf,
    /// Where Steam is looked for again while no installation is known.
    home: Option<PathBuf>,
    steam_roots: Vec<PathBuf>,
    /// Each process seen, by pid.
    seen: HashMap<u32, Seen>,
    /// Each app's name, and the scan it was looked up in.
    names: HashMap<u32, (Option<String>, u64)>,
    scans: u64,
}

/// What is kept of a process: when it started, the Steam app it was
/// launched for, and the program it runs.
#[derive(Clone)]
struct Seen {
    started: u64,
    appid: Option<u32>,
    program: Option<String>,
}

/// A process's start time (field 22 of its `stat`), which tells a reused
/// pid from the process that had it before.
fn start_time(stat: &str) -> Option<u64> {
    // The command name is in brackets and may hold spaces: count after it.
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

impl Scanner {
    pub fn new(home: &Path) -> Self {
        Self { home: Some(home.into()), ..Self::with_roots(Path::new("/proc"), steam::roots(home)) }
    }

    pub fn with_roots(proc_root: &Path, steam_roots: Vec<PathBuf>) -> Self {
        Self { proc_root: proc_root.into(), home: None, steam_roots, seen: HashMap::new(), names: HashMap::new(), scans: 0 }
    }

    /// The game that is running now, if one is: one of the `added` games
    /// whose program runs, or else a Steam game. With several of Steam's,
    /// the one whose process started last.
    pub fn scan(&mut self, added: &[AddedGame]) -> Option<Game> {
        self.scans += 1;
        // Steam installed, or its drive mounted, after this started.
        if self.steam_roots.is_empty()
            && let Some(home) = &self.home
        {
            self.steam_roots = steam::roots(home);
        }
        let verify = self.scans.is_multiple_of(VERIFY_EVERY);
        let mut alive: HashMap<u32, Seen> = HashMap::new();
        for entry in std::fs::read_dir(&self.proc_root).into_iter().flatten().flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if !verify && let Some(known) = self.seen.get(&pid) {
                alive.insert(pid, known.clone());
                continue;
            }
            let dir = entry.path();
            let Some(started) = std::fs::read_to_string(dir.join("stat")).ok().as_deref().and_then(start_time) else { continue };
            let seen = match self.seen.get(&pid).filter(|known| known.started == started) {
                Some(known) => known.clone(),
                None => Seen {
                    started,
                    // Other users' processes cannot be read: not a game of ours.
                    appid: std::fs::read(dir.join("environ")).ok().and_then(|e| steam::appid_from_environ(&e)),
                    program: programs::of_process_in(&dir),
                },
            };
            alive.insert(pid, seen);
        }
        self.seen = alive;

        // A game the user added says more than Steam's guess.
        if let Some(game) = programs::added(added, self.seen.values().filter_map(|seen| seen.program.as_deref())) {
            return Some(Game { name: game.name.clone(), steam_appid: None });
        }

        let mut newest: Option<(u64, u32)> = None;
        for Seen { started, appid, .. } in self.seen.values() {
            let Some(appid) = *appid else { continue };
            let (roots, scans) = (&self.steam_roots, self.scans);
            let name = self.names.entry(appid).or_insert_with(|| (steam::app_name(appid, roots), scans));
            if name.0.is_none() && scans - name.1 >= RETRY_NAME_AFTER {
                *name = (steam::app_name(appid, roots), scans);
            }
            if name.0.is_some() && newest.is_none_or(|(s, _)| *started > s) {
                newest = Some((*started, appid));
            }
        }
        let (_, appid) = newest?;
        Some(Game { name: self.names[&appid].0.clone()?, steam_appid: Some(appid) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(proc_root: &Path, pid: u32, started: u64, environ: &[u8]) {
        let dir = proc_root.join(pid.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let rest = (0..18).map(|_| "0").collect::<Vec<_>>().join(" ");
        std::fs::write(dir.join("stat"), format!("{pid} (some game) S {rest} {started} 0 0\n")).unwrap();
        std::fs::write(dir.join("environ"), environ).unwrap();
    }

    fn setup() -> (tempfile::TempDir, Scanner) {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("steam/steamapps");
        std::fs::create_dir_all(&apps).unwrap();
        for (id, name) in [(730, "Counter-Strike 2"), (570, "Dota 2"), (1493710, "Proton Experimental")] {
            std::fs::write(apps.join(format!("appmanifest_{id}.acf")), format!("\t\"name\"\t\t\"{name}\"\n")).unwrap();
        }
        let proc_root = dir.path().join("proc");
        std::fs::create_dir_all(proc_root.join("self")).unwrap();
        let scanner = Scanner::with_roots(&proc_root, vec![dir.path().join("steam")]);
        (dir, scanner)
    }

    #[test]
    fn start_time_survives_spaces_and_brackets_in_the_name() {
        let rest = (0..18).map(|_| "0").collect::<Vec<_>>().join(" ");
        assert_eq!(start_time(&format!("12 (a (b) c) S {rest} 4242 0")), Some(4242));
    }

    #[test]
    fn a_game_is_found_while_it_runs_and_gone_after() {
        let (dir, mut scanner) = setup();
        let proc_root = dir.path().join("proc");
        process(&proc_root, 10, 100, b"HOME=/h\0");
        assert_eq!(scanner.scan(&[]), None);
        process(&proc_root, 20, 200, b"SteamAppId=730\0");
        assert_eq!(scanner.scan(&[]), Some(Game { name: "Counter-Strike 2".into(), steam_appid: Some(730) }));
        std::fs::remove_dir_all(proc_root.join("20")).unwrap();
        assert_eq!(scanner.scan(&[]), None);
    }

    #[test]
    fn a_game_added_by_hand_is_known_by_its_program() {
        let (dir, mut scanner) = setup();
        let proc_root = dir.path().join("proc");
        let added = [AddedGame { name: "Order of the Sinking Star".into(), program: "Star.exe".into() }];
        process(&proc_root, 20, 200, b"SteamAppId=730\0");
        process(&proc_root, 30, 300, b"");
        std::fs::write(proc_root.join("30/cmdline"), b"Z:\\games\\star.exe\0-windowed\0").unwrap();
        assert_eq!(scanner.scan(&added), Some(Game { name: "Order of the Sinking Star".into(), steam_appid: None }), "it comes before Steam's");
        // Added to the settings later, a process already seen still counts.
        assert_eq!(scanner.scan(&[]).unwrap().name, "Counter-Strike 2");
        assert_eq!(scanner.scan(&added).unwrap().name, "Order of the Sinking Star");
        std::fs::remove_dir_all(proc_root.join("30")).unwrap();
        assert_eq!(scanner.scan(&added).unwrap().name, "Counter-Strike 2");
    }

    #[test]
    fn tools_are_ignored_and_the_newest_game_wins() {
        let (dir, mut scanner) = setup();
        let proc_root = dir.path().join("proc");
        process(&proc_root, 20, 200, b"SteamAppId=730\0");
        process(&proc_root, 30, 300, b"SteamAppId=1493710\0");
        assert_eq!(scanner.scan(&[]).unwrap().name, "Counter-Strike 2");
        process(&proc_root, 40, 400, b"SteamAppId=570\0");
        assert_eq!(scanner.scan(&[]).unwrap().name, "Dota 2");
    }

    #[test]
    fn a_process_is_read_once_and_a_reused_pid_is_read_again() {
        let (dir, mut scanner) = setup();
        let proc_root = dir.path().join("proc");
        process(&proc_root, 20, 200, b"SteamAppId=730\0");
        assert!(scanner.scan(&[]).is_some());
        // The environment is not read a second time for the same process.
        std::fs::write(proc_root.join("20/environ"), b"").unwrap();
        assert!(scanner.scan(&[]).is_some());
        // The same pid with another start time is another process, which
        // the next look at every process finds out.
        process(&proc_root, 20, 999, b"");
        let found: Vec<bool> = (0..VERIFY_EVERY).map(|_| scanner.scan(&[]).is_some()).collect();
        assert_eq!(found.last(), Some(&false), "{found:?}");
    }

    // Steam in a home folder is how Linux has it.
    #[cfg(unix)]
    #[test]
    fn steam_and_a_game_installed_later_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let proc_root = dir.path().join("proc");
        std::fs::create_dir_all(&proc_root).unwrap();
        let mut scanner = Scanner { home: Some(dir.path().into()), ..Scanner::with_roots(&proc_root, vec![]) };
        process(&proc_root, 20, 200, b"SteamAppId=730\0");
        assert_eq!(scanner.scan(&[]), None, "no Steam yet");
        let apps = dir.path().join(".local/share/Steam/steamapps");
        std::fs::create_dir_all(&apps).unwrap();
        assert_eq!(scanner.scan(&[]), None, "Steam, but not the game");
        std::fs::write(apps.join("appmanifest_730.acf"), "\t\"name\"\t\t\"Counter-Strike 2\"\n").unwrap();
        let found: Vec<bool> = (0..=RETRY_NAME_AFTER).map(|_| scanner.scan(&[]).is_some()).collect();
        assert_eq!(found.last(), Some(&true), "{found:?}");
    }
}
