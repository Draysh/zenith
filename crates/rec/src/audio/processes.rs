//! The programs that are running, as far as sound goes: their names, and
//! who started whom, to find the voice programs a track takes or leaves out.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: Option<u32>,
    /// The executable's file name, as the system gives it.
    pub name: String,
}

fn bare(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    lower.strip_suffix(".exe").map(String::from).unwrap_or(lower)
}

/// Whether the program `exe` is the app the settings call `app`: the same
/// name whatever the case, with or without `.exe`.
pub fn matches(app: &str, exe: &str) -> bool {
    !bare(app).is_empty() && bare(app) == bare(exe)
}

/// The top process of each running instance of `apps`: a process that is
/// one of them and was not started by another of the same app. Recording
/// (or leaving out) its tree covers the helpers it started, which is where
/// a program like Discord actually plays its sound. In the order the apps
/// are named, then by pid.
pub fn roots(apps: &[String], procs: &[Proc]) -> Vec<Proc> {
    let mut out = Vec::new();
    for app in apps {
        let mut found: Vec<&Proc> = procs
            .iter()
            .filter(|p| matches(app, &p.name))
            .filter(|p| !p.parent.is_some_and(|parent| procs.iter().any(|q| q.pid == parent && q.pid != p.pid && matches(app, &q.name))))
            .collect();
        found.sort_by_key(|p| p.pid);
        for p in found {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
    }
    out
}

/// The latest list of running programs, shared between the thread that
/// reads it from the system and the tracks that plan by it.
#[derive(Default)]
pub struct ProcessTable(Mutex<Vec<Proc>>);

impl ProcessTable {
    pub fn snapshot(&self) -> Vec<Proc> {
        self.0.lock().map(|list| list.clone()).unwrap_or_default()
    }

    pub fn replace(&self, procs: Vec<Proc>) {
        if let Ok(mut list) = self.0.lock() {
            *list = procs;
        }
    }
}

/// Asks the system which programs are running.
pub struct Scanner(Option<sysinfo::System>);

impl Scanner {
    pub fn new() -> Self {
        let system = sysinfo::System::new();
        if let Err(err) = &system {
            crate::note!("the running programs cannot be listed ({err}): voice programs will not be told apart");
        }
        Self(system.ok())
    }

    /// The programs running now; none when the system cannot be asked.
    pub fn scan(&mut self) -> Vec<Proc> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
        let Some(system) = self.0.as_mut() else { return Vec::new() };
        // Names and parents only: nothing about memory, CPU or disk is read.
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
        system
            .processes()
            .values()
            .map(|p| Proc { pid: p.pid().as_u32(), parent: p.parent().map(|pid| pid.as_u32()), name: p.name().to_string_lossy().into_owned() })
            .collect()
    }
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

/// Keep `table` up to date until `stop` is set. Reading every process takes
/// a few milliseconds, too long for the mixer's own thread.
pub fn keep_scanning(table: Arc<ProcessTable>, stop: Arc<AtomicBool>) {
    let mut scanner = Scanner::new();
    while !stop.load(Ordering::Relaxed) {
        table.replace(scanner.scan());
        // Sleep in short steps, so stopping does not wait for a whole pause.
        for _ in 0..(super::RESCAN.as_millis() / 2 / 100) {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: u32, name: &str) -> Proc {
        Proc { pid, parent: (parent != 0).then_some(parent), name: name.into() }
    }

    #[test]
    fn names_match_whatever_the_case_and_ending() {
        assert!(matches("Discord", "Discord.exe"));
        assert!(matches("discord.exe", "DISCORD.EXE"));
        assert!(matches("Discord.exe", "discord"));
        assert!(matches("TeamSpeak 3", "teamspeak 3.exe"));
        assert!(!matches("Discord", "DiscordPTB.exe"));
        assert!(!matches("", "anything.exe"));
        assert!(!matches(".exe", ".exe"));
    }

    #[test]
    fn the_root_is_the_one_not_started_by_its_own_kind() {
        let procs = [
            proc(900, 4, "explorer.exe"),
            proc(1210, 1200, "Discord.exe"),
            proc(1200, 900, "Discord.exe"),
            proc(1230, 1210, "Discord.exe"),
            proc(5000, 900, "Discord.exe"),
        ];
        let pids: Vec<u32> = roots(&["discord".into()], &procs).iter().map(|p| p.pid).collect();
        assert_eq!(pids, [1200, 5000], "two instances, each with its own tree");
    }

    #[test]
    fn a_parent_that_is_gone_or_another_program_makes_a_root() {
        // Started by an updater that has since exited (its pid is nobody's).
        let procs = [proc(1200, 777, "Discord.exe"), proc(1210, 1200, "Discord.exe"), proc(10, 10, "self.exe")];
        assert_eq!(roots(&["Discord".into()], &procs)[0].pid, 1200);
        assert_eq!(roots(&["self".into()], &procs).len(), 1, "a process named as its own parent is still found");
        assert!(roots(&["Mumble".into()], &procs).is_empty());
    }

    #[test]
    fn roots_come_in_the_order_the_apps_are_named() {
        let procs = [proc(100, 1, "discord.exe"), proc(50, 1, "mumble.exe")];
        let names: Vec<String> = roots(&["Discord".into(), "Mumble".into(), "discord.exe".into()], &procs).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["discord.exe", "mumble.exe"], "and none twice");
    }

    #[test]
    fn the_system_is_asked_and_knows_this_process() {
        let procs = Scanner::new().scan();
        let me = procs.iter().find(|p| p.pid == std::process::id()).expect("this test is running");
        assert!(!me.name.is_empty());
        assert!(me.parent.is_some());
    }
}
