//! The save hotkey. Windows has no setting for one, so there the daemon
//! holds the key combination itself and a press is a save request like any
//! other. On Linux no program may take a key by itself under Wayland: the
//! daemon asks the desktop for one through its GlobalShortcuts portal
//! (`portal`, below), and where the desktop has no such portal, or the user
//! says no, `zenithd save` bound to a key in the desktop's own settings
//! works as it always has.
//!
//! The system is asked for the combination on a thread of its own, because
//! Windows delivers a hotkey to the thread that asked for it, as a message
//! that thread has to fetch. What the thread does with a setting (read it,
//! fall back, take the new combination, let go of the old, say how it
//! went) is plain logic and is tested with a stand-in for the system.

use crate::daemon::Msg;
use tokio::sync::mpsc;

/// The daemon's hold on the hotkey.
pub struct Hotkeys {
    #[cfg(windows)]
    settings: std::sync::mpsc::Sender<String>,
    /// `None` when no hotkey is to be asked for (`ZENITH_NO_HOTKEY`).
    #[cfg(unix)]
    settings: Option<tokio::sync::watch::Sender<Option<String>>>,
}

#[cfg(unix)]
impl Hotkeys {
    /// Get ready to ask the desktop for a hotkey, in a task of its own:
    /// the asking can wait on the user for as long as they like, and must
    /// never hold the daemon up. Nothing is asked for until [`Self::set`].
    /// With `ZENITH_NO_HOTKEY` in the environment nothing ever is (the
    /// tests, which must not reach the desktop of whoever runs them).
    pub fn start(tx: mpsc::UnboundedSender<Msg>) -> Self {
        if std::env::var_os("ZENITH_NO_HOTKEY").is_some() {
            return Self { settings: None };
        }
        let (settings, changes) = tokio::sync::watch::channel(None);
        tokio::spawn(portal::keep(changes, tx));
        Self { settings: Some(settings) }
    }

    /// Ask for the combination the settings name (`capture.hotkey`), in
    /// place of the one asked for before. How it went comes back as
    /// `Msg::Hotkey` (bound, and as what) or `Msg::NoHotkey` (why not).
    pub fn set(&self, setting: &str) {
        if let Some(settings) = &self.settings {
            let _ = settings.send(Some(setting.to_string()));
        }
    }
}

/// The save hotkey on Linux, through the desktop's GlobalShortcuts portal
/// (`org.freedesktop.portal.GlobalShortcuts` on the session bus): a session
/// is made, one shortcut called `save` is bound in it with the configured
/// combination as the one preferred, and each time the desktop says it was
/// pressed a clip is saved. The desktop has the last word on the key: it
/// may show the user a dialog, and the user may pick another or say no.
#[cfg(unix)]
mod portal {
    use crate::daemon::Msg;
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::sync::{mpsc, oneshot, watch};
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
    use zbus::{Connection, Proxy};
    use zenith_core::hotkey::{Hotkey, Key, Named, parse_or_default};
    use zenith_core::ipc::Request;

    const DESKTOP: &str = "org.freedesktop.portal.Desktop";
    const AT: &str = "/org/freedesktop/portal/desktop";
    const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
    /// What the shortcut is called between Zenith and the desktop.
    const SAVE: &str = "save";
    /// Who is asking, as the desktop knows Zenith: its launcher entry.
    const APP_ID: &str = "zenith-clips";
    /// How long until a desktop without the portal is asked again, at
    /// first and at most: it may only not be up yet, soon after login.
    const FIRST_RETRY: Duration = Duration::from_secs(30);
    const LONGEST_RETRY: Duration = Duration::from_secs(30 * 60);

    /// A combination the way the portal wants one named (the XDG shortcuts
    /// format): the modifiers in capitals, then the key by the name
    /// xkbcommon has for it, joined by `+`. `Alt+F9` is `ALT+F9`,
    /// `Ctrl+Shift+S` is `CTRL+SHIFT+s`.
    pub(super) fn trigger(hotkey: Hotkey) -> String {
        let mut text = String::new();
        for (held, name) in [(hotkey.ctrl, "CTRL"), (hotkey.alt, "ALT"), (hotkey.shift, "SHIFT"), (hotkey.win, "LOGO")] {
            if held {
                text.push_str(name);
                text.push('+');
            }
        }
        match hotkey.key {
            Key::Letter(c) => text.push(c.to_ascii_lowercase()),
            Key::Digit(d) => text.push_str(&d.to_string()),
            Key::F(n) => text.push_str(&format!("F{n}")),
            Key::Num(d) => text.push_str(&format!("KP_{d}")),
            Key::Named(named) => text.push_str(match named {
                Named::Space => "space",
                Named::Insert => "Insert",
                Named::Delete => "Delete",
                Named::Home => "Home",
                Named::End => "End",
                Named::PageUp => "Page_Up",
                Named::PageDown => "Page_Down",
                Named::PrintScreen => "Print",
                Named::ScrollLock => "Scroll_Lock",
                Named::Pause => "Pause",
                Named::NumPlus => "KP_Add",
                Named::NumMinus => "KP_Subtract",
                Named::NumMultiply => "KP_Multiply",
                Named::NumDivide => "KP_Divide",
                Named::NumDecimal => "KP_Decimal",
            }),
        }
        text
    }

    /// Where the portal will answer a request made with `token` by the
    /// connection named `sender` (`:1.42`): known beforehand, so that the
    /// answer can be listened for before the request is made.
    pub(super) fn request_path(sender: &str, token: &str) -> String {
        format!("{AT}/request/{}/{token}", sender.trim_start_matches(':').replace('.', "_"))
    }

    /// Why there is no hotkey from the desktop.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) enum End {
        /// It cannot be asked for here: no session bus, no portal, a
        /// portal that refuses.
        Unavailable(String),
        /// The user was asked and said no.
        Declined,
        /// It was bound, and then the desktop's side of it went away.
        Lost(String),
    }

    impl End {
        /// In words, for the log and the status.
        pub(super) fn why(&self) -> String {
            match self {
                Self::Unavailable(why) => format!("the desktop cannot be asked for one ({why})"),
                Self::Declined => "the desktop was asked for one and it was declined".to_string(),
                Self::Lost(why) => format!("the one the desktop gave was lost ({why})"),
            }
        }

        /// How long until it is asked for again, after `tries` in a row
        /// that got nowhere. A no from the user stands until the setting
        /// changes: asking again would put the dialog in front of them
        /// again. Otherwise the wait doubles each time, up to half an hour.
        pub(super) fn retry_after(&self, tries: u32) -> Option<Duration> {
            match self {
                Self::Declined => None,
                Self::Unavailable(_) | Self::Lost(_) => Some((FIRST_RETRY * 2u32.pow(tries.min(8))).min(LONGEST_RETRY)),
            }
        }
    }

    /// What the number the portal answers a request with means: 0 is yes,
    /// 1 is the user saying no, anything else is the portal giving up.
    pub(super) fn answered(code: u32, what: &str) -> Result<(), End> {
        match code {
            0 => Ok(()),
            1 => Err(End::Declined),
            other => Err(End::Unavailable(format!("the portal ended {what} with {other}"))),
        }
    }

    fn unavailable(what: &'static str) -> impl Fn(zbus::Error) -> End {
        move |e| End::Unavailable(format!("{what}: {e}"))
    }

    /// A name for a request or a session that no other of this daemon's has.
    fn token() -> String {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        format!("zenith_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// Start listening for the portal's answer to the request about to be
    /// made with `token`.
    async fn answers(bus: &Connection, token: &str) -> Result<zbus::proxy::SignalStream<'static>, End> {
        let sender = bus.unique_name().map(|name| name.to_string()).unwrap_or_default();
        let request = Proxy::new(bus, DESKTOP, request_path(&sender, token), "org.freedesktop.portal.Request").await.map_err(unavailable("the portal's answer cannot be listened for"))?;
        request.receive_signal("Response").await.map_err(unavailable("the portal's answer cannot be listened for"))
    }

    /// Wait for that answer: what the portal gives back, when it says yes.
    /// This is where a dialog put before the user is waited on.
    async fn answer(mut answers: zbus::proxy::SignalStream<'static>, what: &str) -> Result<HashMap<String, OwnedValue>, End> {
        let Some(message) = answers.next().await else {
            return Err(End::Unavailable(format!("the session bus went away during {what}")));
        };
        let (code, results) = message.body().deserialize::<(u32, HashMap<String, OwnedValue>)>().map_err(unavailable("the portal's answer was not understood"))?;
        answered(code, what).map(|_| results)
    }

    /// What is held once the desktop has bound the shortcut.
    struct Bound {
        portal: Proxy<'static>,
        session: OwnedObjectPath,
        /// The key as the desktop describes it, which may not be the one
        /// asked for.
        key: String,
    }

    /// Ask the desktop for the shortcut, with `hotkey` as the key preferred.
    async fn bind(hotkey: Hotkey) -> Result<Bound, End> {
        let bus = Connection::session().await.map_err(unavailable("no session bus"))?;
        // Newer portals want to be told who is asking, when it is not a
        // sandboxed app; older ones do not know the call, which is fine.
        let _ = bus.call_method(Some(DESKTOP), AT, Some("org.freedesktop.host.portal.Registry"), "Register", &(APP_ID, HashMap::<&str, Value<'_>>::new())).await;
        let portal: Proxy<'static> = Proxy::new(&bus, DESKTOP, AT, SHORTCUTS).await.map_err(unavailable("no portal"))?;
        portal.get_property::<u32>("version").await.map_err(unavailable("this desktop's portal has no global shortcuts"))?;

        let (request, session) = (token(), token());
        let listening = answers(&bus, &request).await?;
        let options: HashMap<&str, Value<'_>> = HashMap::from([("handle_token", Value::from(request.as_str())), ("session_handle_token", Value::from(session.as_str()))]);
        portal.call::<_, _, OwnedObjectPath>("CreateSession", &(options,)).await.map_err(unavailable("no session for shortcuts was made"))?;
        let made = answer(listening, "making a session").await?;
        // The one place the portal gives a path as plain text.
        let session = made
            .get("session_handle")
            .and_then(|handle| handle.downcast_ref::<&str>().ok())
            .and_then(|handle| ObjectPath::try_from(handle).ok())
            .map(OwnedObjectPath::from)
            .ok_or_else(|| End::Unavailable("the portal did not say which session it made".into()))?;

        let request = token();
        let listening = answers(&bus, &request).await?;
        let shortcut: HashMap<&str, Value<'_>> = HashMap::from([("description", Value::from("Save a clip")), ("preferred_trigger", Value::from(trigger(hotkey)))]);
        let options: HashMap<&str, Value<'_>> = HashMap::from([("handle_token", Value::from(request.as_str()))]);
        portal.call::<_, _, OwnedObjectPath>("BindShortcuts", &(&session, vec![(SAVE, shortcut)], "", options)).await.map_err(unavailable("the shortcut was not taken"))?;
        let bound = answer(listening, "binding the shortcut").await?;
        // What it bound, in the desktop's own words; without them, what
        // was asked for is the best that can be said.
        let described = bound
            .get("shortcuts")
            .and_then(|all| <Vec<(String, HashMap<String, OwnedValue>)>>::try_from(all.try_clone().ok()?).ok())
            .and_then(|all| all.into_iter().find(|(id, _)| id == SAVE))
            .and_then(|(_, about)| about.get("trigger_description")?.downcast_ref::<&str>().ok().map(str::to_string))
            .filter(|key| !key.trim().is_empty());
        let key = described.unwrap_or_else(|| format!("{hotkey}, if the desktop agreed to it"));
        Ok(Bound { portal, session, key })
    }

    impl Bound {
        /// Save a clip each time the desktop says the shortcut was
        /// pressed, until its side of this goes away.
        async fn listen(self, tx: &mpsc::UnboundedSender<Msg>) -> End {
            let lost = |e: zbus::Error| End::Lost(e.to_string());
            let bus = self.portal.connection();
            let mut pressed = match self.portal.receive_signal("Activated").await {
                Ok(pressed) => pressed,
                Err(e) => return lost(e),
            };
            let mut closed = match Proxy::new(bus, DESKTOP, self.session.as_str().to_string(), "org.freedesktop.portal.Session").await {
                Ok(session) => match session.receive_signal("Closed").await {
                    Ok(closed) => closed,
                    Err(e) => return lost(e),
                },
                Err(e) => return lost(e),
            };
            let mut owners = match self.portal.receive_owner_changed().await {
                Ok(owners) => owners,
                Err(e) => return lost(e),
            };
            // Whoever answers as the portal now; a later one never heard
            // of this session.
            let mut portal_is = None;
            loop {
                tokio::select! {
                    press = pressed.next() => {
                        let Some(press) = press else { return End::Lost("the session bus went away".into()) };
                        let Ok((session, id, _when, _more)) = press.body().deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>() else { continue };
                        if session == self.session && id == SAVE {
                            // A save request, as if `zenithd save` had been
                            // run; what comes of it reaches the user as a
                            // notification.
                            let (reply, _) = oneshot::channel();
                            if tx.send(Msg::Ipc(Request::Save, reply)).is_err() {
                                return End::Lost("the daemon is stopping".into());
                            }
                        }
                    }
                    _ = closed.next() => return End::Lost("the desktop closed the shortcut's session".into()),
                    owner = owners.next() => {
                        let owner = owner.flatten().map(|name| name.to_string());
                        if owner.is_none() || portal_is.as_ref().is_some_and(|was| Some(was) != owner.as_ref()) {
                            return End::Lost("the desktop's portal was restarted".into());
                        }
                        portal_is = owner;
                    }
                }
            }
        }
    }

    /// Keep the shortcut bound to whatever the settings last named, for
    /// as long as the daemon runs, telling it each time that changes how
    /// things stand. Everything that goes wrong here ends the same way:
    /// said once, and `zenithd save` bound by hand works as before.
    pub(super) async fn keep(mut settings: watch::Receiver<Option<String>>, tx: mpsc::UnboundedSender<Msg>) {
        let mut tries = 0;
        loop {
            let Some(setting) = settings.borrow_and_update().clone() else {
                if settings.changed().await.is_err() {
                    return;
                }
                continue;
            };
            let (hotkey, unreadable) = parse_or_default(&setting);
            if let Some(why) = unreadable {
                crate::warn(format_args!("zenithd: the save hotkey in the settings cannot be used: {why}"));
            }
            let end = tokio::select! {
                end = async {
                    match bind(hotkey).await {
                        Ok(bound) => {
                            tries = 0;
                            let _ = tx.send(Msg::Hotkey(Ok(format!("{} (bound through the desktop)", bound.key))));
                            bound.listen(&tx).await
                        }
                        Err(end) => end,
                    }
                } => end,
                // A new setting: what was asked for, or bound, is let go
                // (the session ends with its connection) and it starts over.
                changed = settings.changed() => match changed {
                    Ok(()) => continue,
                    Err(_) => return,
                },
            };
            if tx.send(Msg::NoHotkey(end.why())).is_err() {
                return;
            }
            let wait = end.retry_after(tries);
            tries += 1;
            tokio::select! {
                _ = async {
                    match wait {
                        Some(wait) => tokio::time::sleep(wait).await,
                        None => std::future::pending().await,
                    }
                } => {}
                changed = settings.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    tries = 0;
                }
            }
        }
    }
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

/// How often a combination that another program had is asked for again:
/// that program may have closed, or let go of it.
#[cfg(windows)]
const RETRY_EVERY: std::time::Duration = std::time::Duration::from_secs(20);

/// Hold whichever combination was set last, until the daemon is gone, and
/// tell it how each setting went: the combination now held, or what is
/// wrong, in words for the user. One that could not be taken is asked for
/// again every `retry_every`, and the daemon told when that works.
#[cfg(any(windows, test))]
fn hold(mut binder: impl Binder, settings: std::sync::mpsc::Receiver<String>, tx: mpsc::UnboundedSender<Msg>, retry_every: std::time::Duration) {
    use std::sync::mpsc::RecvTimeoutError;
    use zenith_core::hotkey::parse_or_default;
    let mut held = None;
    // The combination that is wanted and could not be taken, and when it
    // was last asked for.
    let mut missing: Option<(zenith_core::hotkey::Hotkey, std::time::Instant)> = None;
    loop {
        binder.pump();
        let setting = match settings.recv_timeout(PUMP_EVERY) {
            Ok(setting) => setting,
            Err(RecvTimeoutError::Timeout) => {
                if let Some((hotkey, asked)) = missing
                    && asked.elapsed() >= retry_every
                {
                    // Still taken is not news, and is not said again.
                    if binder.bind(hotkey).is_ok() {
                        (held, missing) = (Some(hotkey), None);
                        if tx.send(Msg::Hotkey(Ok(hotkey.to_string()))).is_err() {
                            break;
                        }
                    } else {
                        missing = Some((hotkey, std::time::Instant::now()));
                    }
                }
                continue;
            }
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
                Ok(()) => (held, missing) = (Some(hotkey), None),
                Err(why) => {
                    missing = Some((hotkey, std::time::Instant::now()));
                    outcome = Err(format!("The save hotkey {hotkey} does not work: {why}"));
                }
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
                hold(System { manager, bound: Vec::new() }, received, tx, super::RETRY_EVERY);
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
        /// How many times it is asked for before whoever has it lets go.
        taken_for: usize,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Binder for Fake {
        fn bind(&mut self, hotkey: Hotkey) -> Result<(), String> {
            self.asked.lock().unwrap().push(format!("bind {hotkey}"));
            if self.taken == Some(hotkey) && self.taken_for > 0 {
                self.taken_for -= 1;
                Err("another program is using it".into())
            } else {
                Ok(())
            }
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
        hold(Fake { taken: taken.map(|t| t.parse().unwrap()), taken_for: usize::MAX, asked: asked.clone() }, receive, tx, std::time::Duration::from_secs(3600));
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

    #[test]
    fn a_combination_that_was_taken_is_asked_for_again_until_it_is_free() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (send, receive) = std::sync::mpsc::channel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        send.send("Alt+F9".to_string()).unwrap();
        // Whoever has it lets go after it has been asked for three times.
        let fake = Fake { taken: Some("Alt+F9".parse().unwrap()), taken_for: 3, asked: asked.clone() };
        let holding = std::thread::spawn(move || hold(fake, receive, tx, std::time::Duration::from_millis(1)));
        let told: Vec<_> = (0..2).map(|_| rx.blocking_recv()).collect();
        drop(send);
        holding.join().unwrap();
        assert!(matches!(&told[0], Some(Msg::Hotkey(Err(why))) if why.contains("another program")), "said once that it does not work");
        assert!(matches!(&told[1], Some(Msg::Hotkey(Ok(held))) if held == "Alt+F9"), "and then that it does");
        assert_eq!(*asked.lock().unwrap(), ["bind Alt+F9", "bind Alt+F9", "bind Alt+F9", "bind Alt+F9", "unbind"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_combination_is_named_the_way_the_portal_wants_it() {
        let named = |text: &str| portal::trigger(text.parse().unwrap());
        assert_eq!(named("Alt+F9"), "ALT+F9");
        assert_eq!(named("ctrl+shift+s"), "CTRL+SHIFT+s");
        assert_eq!(named("Win+Alt+7"), "ALT+LOGO+7");
        assert_eq!(named("Ctrl+Num5"), "CTRL+KP_5");
        assert_eq!(named("Ctrl+NumPlus"), "CTRL+KP_Add");
        assert_eq!(named("Alt+PageUp"), "ALT+Page_Up");
        assert_eq!(named("PrintScreen"), "Print");
        assert_eq!(named("Ctrl+Space"), "CTRL+space");
        assert_eq!(named("F24"), "F24");
        // Every key the settings can name has a name, with no `+` in it
        // to be taken for a join.
        for key in zenith_core::hotkey::Named::ALL {
            let name = named(&format!("Ctrl+Alt+{}", key.name()));
            assert_eq!(name.matches('+').count(), 2, "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_portals_answer_is_listened_for_where_it_will_come() {
        assert_eq!(portal::request_path(":1.42", "zenith_7_0"), "/org/freedesktop/portal/desktop/request/1_42/zenith_7_0");
    }

    #[cfg(unix)]
    #[test]
    fn a_desktop_without_the_portal_is_asked_again_later_and_a_no_is_taken_for_one() {
        use portal::{End, answered};
        use std::time::Duration;
        assert_eq!(answered(0, "binding"), Ok(()));
        assert_eq!(answered(1, "binding"), Err(End::Declined));
        assert!(matches!(answered(2, "binding"), Err(End::Unavailable(why)) if why.contains("binding")));

        assert_eq!(End::Declined.retry_after(0), None, "not asked again until the setting changes");
        let gone = End::Unavailable("no session bus".into());
        let waits: Vec<_> = (0..12).filter_map(|tries| gone.retry_after(tries)).collect();
        assert_eq!(waits.len(), 12);
        assert_eq!(waits[0], Duration::from_secs(30));
        assert!(waits.windows(2).all(|w| w[1] >= w[0]) && waits[11] == Duration::from_secs(1800), "{waits:?}");
        assert!(End::Lost("the portal was restarted".into()).retry_after(0).is_some());
        for end in [gone, End::Declined, End::Lost("x".into())] {
            assert!(end.why().contains("desktop"), "{}", end.why());
        }
    }
}
