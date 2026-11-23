//! The save hotkey as the settings spell it: `Alt+F9`, `Ctrl+Shift+S`.
//! Only Windows uses it, where the daemon holds the key combination itself;
//! on Linux the desktop's own shortcut settings run `zenithd save`. Reading
//! the text is plain logic, so it lives here and is tested everywhere.

use std::fmt;

/// The combination used when none is set, or the one set cannot be read.
pub const DEFAULT: &str = "Alt+F9";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// `A` to `Z`, in upper case.
    Letter(char),
    /// `0` to `9`, the row above the letters.
    Digit(u8),
    /// `F1` to `F24`.
    F(u8),
    /// `Num0` to `Num9`, the digits of the number pad with Num Lock on.
    Num(u8),
    Named(Named),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Space,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    PrintScreen,
    ScrollLock,
    Pause,
    /// The number pad's `+`, `-`, `*`, `/` and decimal point. They go by
    /// words because `+` is what joins a combination.
    NumPlus,
    NumMinus,
    NumMultiply,
    NumDivide,
    NumDecimal,
}

impl Named {
    pub const ALL: [Self; 15] = [
        Self::Space,
        Self::Insert,
        Self::Delete,
        Self::Home,
        Self::End,
        Self::PageUp,
        Self::PageDown,
        Self::PrintScreen,
        Self::ScrollLock,
        Self::Pause,
        Self::NumPlus,
        Self::NumMinus,
        Self::NumMultiply,
        Self::NumDivide,
        Self::NumDecimal,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Space => "Space",
            Self::Insert => "Insert",
            Self::Delete => "Delete",
            Self::Home => "Home",
            Self::End => "End",
            Self::PageUp => "PageUp",
            Self::PageDown => "PageDown",
            Self::PrintScreen => "PrintScreen",
            Self::ScrollLock => "ScrollLock",
            Self::Pause => "Pause",
            Self::NumPlus => "NumPlus",
            Self::NumMinus => "NumMinus",
            Self::NumMultiply => "NumMultiply",
            Self::NumDivide => "NumDivide",
            Self::NumDecimal => "NumDecimal",
        }
    }

    /// One of the number pad's keys.
    fn on_number_pad(self) -> bool {
        matches!(self, Self::NumPlus | Self::NumMinus | Self::NumMultiply | Self::NumDivide | Self::NumDecimal)
    }
}

impl Key {
    /// A key that types something. Held with nothing but Shift it would be
    /// taken away from every other program's typing.
    fn types(self) -> bool {
        match self {
            Self::Letter(_) | Self::Digit(_) | Self::Num(_) => true,
            Self::F(_) => false,
            Self::Named(named) => named == Named::Space || named.on_number_pad(),
        }
    }
}

/// One word of a combination that is not a modifier, as a key.
fn key(word: &str) -> Option<Key> {
    let upper = word.to_ascii_uppercase();
    let mut chars = upper.chars();
    if let (Some(only), None) = (chars.next(), chars.next()) {
        return match only {
            'A'..='Z' => Some(Key::Letter(only)),
            '0'..='9' => Some(Key::Digit(only as u8 - b'0')),
            _ => None,
        };
    }
    if let Some(number) = upper.strip_prefix('F').filter(|n| !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())) {
        return number.parse::<u8>().ok().filter(|n| (1..=24).contains(n)).map(Key::F);
    }
    if let Some(number) = upper.strip_prefix("NUMPAD").or_else(|| upper.strip_prefix("NUM"))
        && let [digit @ b'0'..=b'9'] = number.as_bytes()
    {
        return Some(Key::Num(digit - b'0'));
    }
    // A few keys go by more than one name.
    let alias = match upper.as_str() {
        "NUMADD" | "NUMPADADD" | "NUMPADPLUS" => "NUMPLUS",
        "NUM-" | "NUMSUBTRACT" | "NUMPADSUBTRACT" | "NUMPADMINUS" => "NUMMINUS",
        "NUM*" | "NUMPADMULTIPLY" => "NUMMULTIPLY",
        "NUM/" | "NUMPADDIVIDE" => "NUMDIVIDE",
        "NUM." | "NUMPADDECIMAL" => "NUMDECIMAL",
        "INS" => "INSERT",
        "DEL" => "DELETE",
        "PGUP" => "PAGEUP",
        "PGDN" | "PGDOWN" => "PAGEDOWN",
        "PRTSC" | "PRINT" => "PRINTSCREEN",
        other => other,
    };
    Named::ALL.into_iter().find(|n| n.name().eq_ignore_ascii_case(alias)).map(Key::Named)
}

impl std::str::FromStr for Hotkey {
    type Err = String;

    /// Modifiers and one key joined by `+`, in any case and any order:
    /// `alt+f9`, `Ctrl + Shift + S`.
    fn from_str(text: &str) -> Result<Self, String> {
        let mut hotkey = Self { ctrl: false, alt: false, shift: false, win: false, key: Key::F(9) };
        let mut found = None;
        for word in text.split('+').map(str::trim) {
            let modifier = match word.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut hotkey.ctrl,
                "alt" => &mut hotkey.alt,
                "shift" => &mut hotkey.shift,
                "win" | "super" | "meta" => &mut hotkey.win,
                "" => return Err(format!("\"{text}\" is not a key combination such as {DEFAULT}")),
                _ => {
                    let key = key(word).ok_or_else(|| format!("\"{word}\" is not a key Zenith knows"))?;
                    if found.replace(key).is_some() {
                        return Err(format!("\"{text}\" names more than one key"));
                    }
                    continue;
                }
            };
            *modifier = true;
        }
        hotkey.key = found.ok_or_else(|| format!("\"{text}\" names no key, only modifiers"))?;
        if hotkey.key.types() && !(hotkey.ctrl || hotkey.alt || hotkey.win) {
            return Err(format!("\"{text}\" would take a key that types away from every program; add Ctrl or Alt"));
        }
        // Held with Shift, the number pad's digits and point are other
        // keys to the system (the arrows and their neighbours).
        if hotkey.shift && matches!(hotkey.key, Key::Num(_) | Key::Named(Named::NumDecimal)) {
            return Err(format!("\"{text}\" cannot be told from the arrow keys: a number pad digit does not go with Shift"));
        }
        // Windows turns Scroll Lock into another key while Ctrl is held.
        if cfg!(windows) && hotkey.ctrl && hotkey.key == Key::Named(Named::ScrollLock) {
            return Err(format!("\"{text}\" never reaches Zenith on Windows: Scroll Lock does not go with Ctrl"));
        }
        Ok(hotkey)
    }
}

impl fmt::Display for Hotkey {
    /// The combination the way it is written on a keyboard's keys.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")] {
            if held {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Letter(c) => write!(f, "{c}"),
            Key::Digit(d) => write!(f, "{d}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Num(d) => write!(f, "Num{d}"),
            Key::Named(n) => f.write_str(n.name()),
        }
    }
}

/// The combination a setting names. One that cannot be read gives way to
/// the default, with what was wrong with it to tell the user: a typing
/// mistake in the settings must not leave them without a way to clip.
pub fn parse_or_default(text: &str) -> (Hotkey, Option<String>) {
    match text.parse() {
        Ok(hotkey) => (hotkey, None),
        Err(why) => {
            let default = Hotkey { ctrl: false, alt: true, shift: false, win: false, key: Key::F(9) };
            (default, Some(format!("{why}; {default} is used")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Hotkey {
        text.parse().unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn the_default_is_alt_f9() {
        let hotkey = parsed(DEFAULT);
        assert_eq!(hotkey, Hotkey { ctrl: false, alt: true, shift: false, win: false, key: Key::F(9) });
        assert_eq!(parse_or_default(DEFAULT), (hotkey, None));
        assert_eq!(hotkey.to_string(), DEFAULT);
    }

    #[test]
    fn modifiers_come_in_any_case_and_order() {
        let expected = Hotkey { ctrl: true, alt: false, shift: true, win: false, key: Key::Letter('S') };
        for text in ["Ctrl+Shift+S", "shift+ctrl+s", " CONTROL + Shift + s ", "s+ctrl+shift"] {
            assert_eq!(parsed(text), expected, "{text}");
        }
        assert_eq!(expected.to_string(), "Ctrl+Shift+S");
        let all = parsed("win+alt+ctrl+shift+f12");
        assert_eq!((all.ctrl, all.alt, all.shift, all.win, all.key), (true, true, true, true, Key::F(12)));
        assert_eq!(all.to_string(), "Ctrl+Alt+Shift+Win+F12");
        assert!(parsed("Super+1").win && parsed("meta+1").win);
    }

    #[test]
    fn letters_digits_function_and_named_keys_are_known() {
        assert_eq!(parsed("alt+z").key, Key::Letter('Z'));
        assert_eq!(parsed("ctrl+0").key, Key::Digit(0));
        assert_eq!(parsed("ctrl+9").key, Key::Digit(9));
        assert_eq!(parsed("F1").key, Key::F(1));
        assert_eq!(parsed("f24").key, Key::F(24));
        assert_eq!(parsed("PrintScreen").key, Key::Named(Named::PrintScreen));
        assert_eq!(parsed("ctrl+pgdn").key, Key::Named(Named::PageDown));
        assert_eq!(parsed("Shift+Insert").key, Key::Named(Named::Insert));
        assert_eq!(parsed("alt+space").key, Key::Named(Named::Space));
        for named in Named::ALL {
            // Alt, which goes with every one of them on every system.
            let text = format!("Alt+{}", named.name());
            assert_eq!(parsed(&text).to_string(), text, "a name reads back as itself");
        }
    }

    #[test]
    fn the_number_pad_has_keys_of_its_own() {
        assert_eq!(parsed("Alt+NumMinus").key, Key::Named(Named::NumMinus));
        assert_eq!(parsed("alt+num-").to_string(), "Alt+NumMinus");
        assert_eq!(parsed("Ctrl+NumpadSubtract").key, Key::Named(Named::NumMinus));
        assert_eq!(parsed("Ctrl+NumAdd").key, Key::Named(Named::NumPlus));
        assert_eq!(parsed("Ctrl+Num*").key, Key::Named(Named::NumMultiply));
        assert_eq!(parsed("Ctrl+Num/").key, Key::Named(Named::NumDivide));
        assert_eq!(parsed("Ctrl+Num.").key, Key::Named(Named::NumDecimal));
        // Its digits are not the ones above the letters.
        assert_eq!(parsed("Alt+Num0").key, Key::Num(0));
        assert_eq!(parsed("alt+numpad9").key, Key::Num(9));
        assert_ne!(parsed("Alt+Num5"), parsed("Alt+5"));
        assert_eq!(parsed("Alt+Num5").to_string(), "Alt+Num5");
        // They type, so they need more than Shift.
        for text in ["NumMinus", "Shift+NumPlus", "Num5"] {
            assert!(text.parse::<Hotkey>().unwrap_err().contains("add Ctrl or Alt"), "{text}");
        }
        for text in ["Ctrl+Shift+Num5", "Alt+Shift+NumDecimal"] {
            assert!(text.parse::<Hotkey>().unwrap_err().contains("does not go with Shift"), "{text}");
        }
        assert!("Ctrl+Shift+NumMinus".parse::<Hotkey>().is_ok(), "the signs are the same keys either way");
        for text in ["Alt+Num", "Alt+Num10", "Alt+NumEnter"] {
            assert!(text.parse::<Hotkey>().is_err(), "{text:?} was read");
        }
    }

    #[test]
    fn what_is_not_a_combination_is_refused_with_a_reason() {
        for text in ["", "+", "Alt+", "Alt++F9", "Alt", "Ctrl+Shift", "F0", "F25", "F09", "Alt+F9+F10", "Alt+Hyper", "Alt+é", "Alt+AB", "Alt F9"] {
            assert!(text.parse::<Hotkey>().is_err(), "{text:?} was read");
        }
        // A key that types needs more than Shift; others may stand alone.
        for text in ["A", "Shift+A", "5", "Space", "Shift+Space"] {
            let why = text.parse::<Hotkey>().unwrap_err();
            assert!(why.contains("add Ctrl or Alt"), "{text}: {why}");
        }
        assert!("Alt+Hyper".parse::<Hotkey>().unwrap_err().contains("\"Hyper\""));
    }

    #[test]
    fn a_setting_that_cannot_be_read_gives_way_to_the_default() {
        let (hotkey, complaint) = parse_or_default("Alt+Banana");
        assert_eq!(hotkey.to_string(), DEFAULT);
        let complaint = complaint.unwrap();
        assert!(complaint.contains("Banana") && complaint.ends_with("Alt+F9 is used"), "{complaint}");
        assert_eq!(parse_or_default("ctrl+f10"), (parsed("Ctrl+F10"), None));
    }
}
