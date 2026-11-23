//! Which game is running: its name, for the folder a clip goes in, and its
//! Steam id, for the cover.

pub mod launchers;
pub mod process;
pub mod programs;
pub mod registry;
pub mod steam;

/// What finds the running game: the processes' environments on Linux,
/// Steam's note in the registry on Windows. Both are asked the same way,
/// and both know a game the user added by the program it runs.
#[cfg(not(windows))]
pub use process::Scanner;
#[cfg(windows)]
pub use registry::Scanner;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub name: String,
    pub steam_appid: Option<u32>,
}

/// The name clips are filed under when no game is running.
pub const DESKTOP: &str = "Desktop";

/// A game's name as a folder name: nothing a file system refuses, no marks
/// (™ ® ©), no trailing dots or spaces, never empty.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '™' | '®' | '©'))
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { ' ' } else { c })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = joined.trim_matches(|c: char| c == '.' || c == ' ');
    let short: String = trimmed.chars().take(80).collect();
    // Cut short, it may end in a dot or a space again, which Windows drops.
    let short = short.trim_end_matches(['.', ' ']);
    // The names Windows keeps for its devices, with or without an ending.
    let stem = short.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.ends_with(|c: char| c.is_ascii_digit()));
    match short {
        "" => DESKTOP.into(),
        _ if device => format!("{short}_"),
        _ => short.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("Counter-Strike 2"), "Counter-Strike 2");
        assert_eq!(folder_name("Tom Clancy's Rainbow Six® Siege"), "Tom Clancy's Rainbow Six Siege");
        assert_eq!(folder_name("NieR:Automata™"), "NieR Automata");
        assert_eq!(folder_name("a/b\\c"), "a b c");
        assert_eq!(folder_name("  ..  "), DESKTOP);
        assert_eq!(folder_name(&"x".repeat(200)).len(), 80);
        assert_eq!(folder_name(&format!("{}. tail", "x".repeat(79))), "x".repeat(79), "no dot at the cut");
        assert_eq!(folder_name("CON"), "CON_");
        assert_eq!(folder_name("com1.exe"), "com1.exe_");
        assert_eq!(folder_name("Control"), "Control");
    }
}
