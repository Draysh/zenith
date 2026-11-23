//! The save hotkey. A Linux desktop binds `zenithd save` to a key in its
//! own settings; Windows has no such setting, so there the daemon holds the
//! key combination itself and a press is a save request like any other.
//!
//! The system is asked for the combination on a thread of its own, because
//! Windows delivers a hotkey to the thread that asked for it, as a message
//! that thread has to fetch. What the thread does with a setting (read it,
//! fall back, take the new combination, let go of the old, say how it
//! went) is plain logic and is tested with a stand-in for the system.

use crate::daemon::Msg;
use tokio::sync::mpsc;

/// The daemon's hold on the hotkey. Where there is none to hold (Linux) it
/// does nothing.
pub struct Hotkeys {
    #[cfg(windows)]
    settings: std::sync::mpsc::Sender<String>,
}

impl Hotkeys {
    /// Get ready to hold a hotkey. Nothing is held until [`Self::set`].
    #[cfg(unix)]
    pub fn start(_tx: mpsc::UnboundedSender<Msg>) -> Self {
        Self {}
    }

    /// Hold the combination the settings name (`capture.hotkey`), in place
    /// of the one held before. How it went comes back as `Msg::Hotkey`.
    #[cfg(unix)]
    pub fn set(&self, _setting: &str) {}
}

/// What takes a key combination for this program, system-wide.
#[cfg(any(windows, test))]
trait Binder {
    fn bind(&mut self, hotkey: zenith_core::hotkey::Hotkey) -> Result<(), String>;
    /// Let go of the combination bound last.
    fn unbind(&mut self);
    /// Hand on the presses that came in since the last call.
    fn pump(&mut self);
}

/// How often presses (and clicks on the tray icon) are fetched: a wait
/// this short is not felt.
#[cfg(any(windows, test))]
pub(crate) const PUMP_EVERY: std::time::Duration = std::time::Duration::from_millis(30);

/// Hold whichever combination was set last, until the daemon is gone, and
/// tell it how each setting went: the combination now held, or what is
/// wrong, in words for the user.
#[cfg(any(windows, test))]
fn hold(mut binder: impl Binder, settings: std::sync::mpsc::Receiver<String>, tx: mpsc::UnboundedSender<Msg>) {
    use std::sync::mpsc::RecvTimeoutError;
    use zenith_core::hotkey::parse_or_default;
    let mut held = None;
    loop {
        binder.pump();
        let setting = match settings.recv_timeout(PUMP_EVERY) {
            Ok(setting) => setting,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let (hotkey, unreadable) = parse_or_default(&setting);
        let mut outcome = match unreadable {
            Some(why) => Err(format!("The save hotkey in the settings cannot be used: {why}")),
            None => Ok(hotkey.to_string()),
        };
        // Set again while a combination could not be taken, it is tried
        // again: whoever had it may have let go.
        if held != Some(hotkey) {
            if held.take().is_some() {
                binder.unbind();
            }
            match binder.bind(hotkey) {
                Ok(()) => held = Some(hotkey),
                Err(why) => outcome = Err(format!("The save hotkey {hotkey} does not work: {why}")),
            }
        }
        if tx.send(Msg::Hotkey(outcome)).is_err() {
            break;
        }
    }
    if held.is_some() {
        binder.unbind();
    }
}

/// Fetch the messages Windows has for this thread and hand each to the
/// window it is for. The hidden windows behind the hotkey and the tray
/// icon hear of a press or a click only this way, each on the thread that
/// made it.
#[cfg(windows)]
pub(crate) fn pump_messages() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};
    // SAFETY: `msg` is ours for each call to fill in and read, and a null
    // window means "any message of this thread".
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::{Binder, Hotkeys, hold};
    use crate::daemon::Msg;
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use tokio::sync::{mpsc, oneshot};
    use zenith_core::hotkey::{Hotkey, Key, Named};
    use zenith_core::ipc::Request;

    const LETTERS: [Code; 26] = [
        Code::KeyA,
        Code::KeyB,
        Code::KeyC,
        Code::KeyD,
        Code::KeyE,
        Code::KeyF,
        Code::KeyG,
        Code::KeyH,
        Code::KeyI,
        Code::KeyJ,
        Code::KeyK,
        Code::KeyL,
        Code::KeyM,
        Code::KeyN,
        Code::KeyO,
        Code::KeyP,
        Code::KeyQ,
        Code::KeyR,
        Code::KeyS,
        Code::KeyT,
        Code::KeyU,
        Code::KeyV,
        Code::KeyW,
        Code::KeyX,
        Code::KeyY,
        Code::KeyZ,
    ];
    const DIGITS: [Code; 10] =
        [Code::Digit0, Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4, Code::Digit5, Code::Digit6, Code::Digit7, Code::Digit8, Code::Digit9];
    const NUMBER_PAD: [Code; 10] = [
        Code::Numpad0,
        Code::Numpad1,
        Code::Numpad2,
        Code::Numpad3,
        Code::Numpad4,
        Code::Numpad5,
        Code::Numpad6,
        Code::Numpad7,
        Code::Numpad8,
        Code::Numpad9,
    ];
    const FUNCTION: [Code; 24] = [
        Code::F1,
        Code::F2,
        Code::F3,
        Code::F4,
        Code::F5,
        Code::F6,
        Code::F7,
        Code::F8,
        Code::F9,
        Code::F10,
        Code::F11,
        Code::F12,
        Code::F13,
        Code::F14,
        Code::F15,
        Code::F16,
        Code::F17,
        Code::F18,
        Code::F19,
        Code::F20,
        Code::F21,
        Code::F22,
        Code::F23,
        Code::F24,
    ];

    /// A key as the hotkey library names it. `None` for one outside what
    /// the settings can name, which the parser never gives.
    fn code(key: Key) -> Option<Code> {
        match key {
            Key::Letter(c) => LETTERS.get((c as usize).checked_sub('A' as usize)?).copied(),
            Key::Digit(d) => DIGITS.get(d as usize).copied(),
            Key::F(n) => FUNCTION.get((n as usize).checked_sub(1)?).copied(),
            Key::Num(d) => NUMBER_PAD.get(d as usize).copied(),
            Key::Named(named) => Some(match named {
                Named::Space => Code::Space,
                Named::Insert => Code::Insert,
                Named::Delete => Code::Delete,
                Named::Home => Code::Home,
                Named::End => Code::End,
                Named::PageUp => Code::PageUp,
                Named::PageDown => Code::PageDown,
                Named::PrintScreen => Code::PrintScreen,
                Named::ScrollLock => Code::ScrollLock,
                Named::Pause => Code::Pause,
                Named::NumPlus => Code::NumpadAdd,
                Named::NumMinus => Code::NumpadSubtract,
                Named::NumMultiply => Code::NumpadMultiply,
                Named::NumDivide => Code::NumpadDivide,
                Named::NumDecimal => Code::NumpadDecimal,
            }),
        }
    }

    fn modifiers(hotkey: Hotkey) -> Modifiers {
        let mut held = Modifiers::empty();
        for (on, modifier) in [(hotkey.ctrl, Modifiers::CONTROL), (hotkey.alt, Modifiers::ALT), (hotkey.shift, Modifiers::SHIFT), (hotkey.win, Modifiers::META)] {
            held.set(modifier, on);
        }
        held
    }

    /// Windows' own hotkeys, through the hidden window the library makes
    /// on this thread.
    struct System {
        manager: GlobalHotKeyManager,
        /// The combination, and for a number pad key its other self.
        bound: Vec<HotKey>,
    }

    /// What Windows calls a number pad key while Num Lock is off: its
    /// digits and decimal point are then the arrows and their neighbours,
    /// and a combination held for the digit alone would not be noticed.
    fn without_num_lock(key: Key) -> Option<Code> {
        const TWINS: [Code; 10] = [
            Code::Insert,
            Code::End,
            Code::ArrowDown,
            Code::PageDown,
            Code::ArrowLeft,
            Code::NumpadClear,
            Code::ArrowRight,
            Code::Home,
            Code::ArrowUp,
            Code::PageUp,
        ];
        match key {
            Key::Num(d) => TWINS.get(d as usize).copied(),
            Key::Named(Named::NumDecimal) => Some(Code::Delete),
            _ => None,
        }
    }

    impl Binder for System {
        fn bind(&mut self, hotkey: Hotkey) -> Result<(), String> {
            let key = HotKey::new(Some(modifiers(hotkey)), code(hotkey.key).ok_or("Windows has no such key")?);
            self.manager.register(key).map_err(|e| match e {
                global_hotkey::Error::AlreadyRegistered(_) => "another program is using it".to_string(),
                other => other.to_string(),
            })?;
            self.bound.push(key);
            // The same key with Num Lock off, where it has another name.
            // If that one is taken the combination still works with Num
            // Lock on, which is how it was picked.
            if let Some(twin) = without_num_lock(hotkey.key).map(|code| HotKey::new(Some(modifiers(hotkey)), code))
                && self.manager.register(twin).is_ok()
            {
                self.bound.push(twin);
            }
            Ok(())
        }

        fn unbind(&mut self) {
            for key in self.bound.drain(..) {
                let _ = self.manager.unregister(key);
            }
        }

        /// A press arrives as a message for the library's window, which
        /// only reaches it when this thread fetches and hands it on.
        fn pump(&mut self) {
            super::pump_messages();
        }
    }

    impl Hotkeys {
        /// Start the thread that holds the hotkey. Nothing is held until
        /// [`Self::set`].
        pub fn start(tx: mpsc::UnboundedSender<Msg>) -> Self {
            let (settings, received) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let manager = match GlobalHotKeyManager::new() {
                    Ok(manager) => manager,
                    Err(e) => {
                        let _ = tx.send(Msg::Hotkey(Err(format!("The save hotkey cannot be used: {e}"))));
                        return;
                    }
                };
                // A press is a save request, as if `zenithd save` had been
                // run; what comes of it reaches the user as a notification.
                let presses = tx.clone();
                GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
                    if event.state == HotKeyState::Pressed {
                        let (reply, _) = oneshot::channel();
                        let _ = presses.send(Msg::Ipc(Request::Save, reply));
                    }
                }));
                hold(System { manager, bound: Vec::new() }, received, tx);
            });
            Self { settings }
        }

        /// Hold the combination the settings name (`capture.hotkey`), in
        /// place of the one held before. How it went comes back as
        /// `Msg::Hotkey`.
        pub fn set(&self, setting: &str) {
            let _ = self.settings.send(setting.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use zenith_core::hotkey::Hotkey;

    /// A system where `taken` is someone else's, which writes down what it
    /// is asked.
    struct Fake {
        taken: Option<Hotkey>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Binder for Fake {
        fn bind(&mut self, hotkey: Hotkey) -> Result<(), String> {
            self.asked.lock().unwrap().push(format!("bind {hotkey}"));
            if self.taken == Some(hotkey) { Err("another program is using it".into()) } else { Ok(()) }
        }

        fn unbind(&mut self) {
            self.asked.lock().unwrap().push("unbind".into());
        }

        fn pump(&mut self) {}
    }

    /// Run `hold` over these settings and give what it asked of the system
    /// and what it told the daemon.
    fn held(taken: Option<&str>, settings: &[&str]) -> (Vec<String>, Vec<Result<String, String>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (send, receive) = std::sync::mpsc::channel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        for setting in settings {
            send.send(setting.to_string()).unwrap();
        }
        drop(send);
        hold(Fake { taken: taken.map(|t| t.parse().unwrap()), asked: asked.clone() }, receive, tx);
        let mut told = Vec::new();
        while let Ok(Msg::Hotkey(outcome)) = rx.try_recv() {
            told.push(outcome);
        }
        let asked = asked.lock().unwrap().clone();
        (asked, told)
    }

    #[test]
    fn the_setting_is_held_and_let_go_at_the_end() {
        let (asked, told) = held(None, &["alt+f9"]);
        assert_eq!(asked, ["bind Alt+F9", "unbind"]);
        assert_eq!(told, [Ok("Alt+F9".to_string())]);
    }

    #[test]
    fn a_new_setting_replaces_the_old_and_the_same_one_is_left_alone() {
        let (asked, told) = held(None, &["Alt+F9", "ALT + F9", "Ctrl+Shift+S"]);
        assert_eq!(asked, ["bind Alt+F9", "unbind", "bind Ctrl+Shift+S", "unbind"]);
        assert_eq!(told, [Ok("Alt+F9".to_string()), Ok("Alt+F9".to_string()), Ok("Ctrl+Shift+S".to_string())]);
    }

    #[test]
    fn a_setting_that_cannot_be_read_holds_the_default_and_says_so() {
        let (asked, told) = held(None, &["Alt+Banana"]);
        assert_eq!(asked, ["bind Alt+F9", "unbind"]);
        let why = told[0].clone().unwrap_err();
        assert!(why.contains("Banana") && why.contains("Alt+F9 is used"), "{why}");
    }

    #[test]
    fn a_combination_that_is_taken_is_said_and_tried_again_when_set_again() {
        let (asked, told) = held(Some("Alt+F9"), &["Alt+F9", "Alt+F9", "Alt+F10"]);
        assert_eq!(asked, ["bind Alt+F9", "bind Alt+F9", "bind Alt+F10", "unbind"], "nothing to let go of until one is held");
        assert_eq!(told[0], Err("The save hotkey Alt+F9 does not work: another program is using it".to_string()));
        assert_eq!(told[1], told[0]);
        assert_eq!(told[2], Ok("Alt+F10".to_string()));
    }
}
