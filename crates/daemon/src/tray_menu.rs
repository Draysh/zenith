//! What a tray icon says and offers, whichever desktop draws it: the
//! status in words, the menu's entries, and what choosing one does. The
//! Linux tray (`tray`) and the Windows one (`tray_windows`) only put this
//! on screen, so the two cannot drift apart and all of it is tested here.

use crate::daemon::Msg;
use std::path::Path;
use tokio::sync::{mpsc, oneshot};
use zenith_core::ipc::{Request, Status};

/// Something the user can choose in the tray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Save,
    /// Open the app's window.
    OpenApp,
    OpenClipFolder,
    /// Capture on, off, or decided by the settings (`None`).
    Capture(Option<bool>),
    Quit,
}

/// Every action there is, for telling one from its name.
#[cfg(any(windows, test))]
const ACTIONS: [Action; 7] =
    [Action::Save, Action::OpenApp, Action::OpenClipFolder, Action::Capture(None), Action::Capture(Some(true)), Action::Capture(Some(false)), Action::Quit];

impl Action {
    /// The name a menu entry carries where a menu reports what was chosen
    /// by name (Windows).
    #[cfg(any(windows, test))]
    pub fn id(self) -> &'static str {
        match self {
            Action::Save => "save",
            Action::OpenApp => "open-app",
            Action::OpenClipFolder => "open-clips",
            Action::Capture(None) => "capture-auto",
            Action::Capture(Some(true)) => "capture-on",
            Action::Capture(Some(false)) => "capture-off",
            Action::Quit => "quit",
        }
    }

    /// The action with this name, if it is one of ours.
    #[cfg(any(windows, test))]
    pub fn from_id(id: &str) -> Option<Action> {
        ACTIONS.into_iter().find(|action| action.id() == id)
    }

    /// What the daemon is asked, for the actions that are the daemon's to
    /// carry out. The others only start a program.
    pub fn request(self) -> Option<Request> {
        match self {
            Action::Save => Some(Request::Save),
            Action::Capture(force) => Some(Request::Capture { force }),
            Action::Quit => Some(Request::Quit),
            Action::OpenApp | Action::OpenClipFolder => None,
        }
    }

    /// Carry it out. A request goes to the daemon the way one from
    /// `zenithd save` does; its answer reaches the user as a notification
    /// or as the icon changing, so it is not waited for here.
    pub fn perform(self, clip_dir: &Path, tx: &mpsc::UnboundedSender<Msg>) {
        match (self.request(), self) {
            (Some(request), _) => {
                let (reply, _) = oneshot::channel();
                let _ = tx.send(Msg::Ipc(request, reply));
            }
            (None, Action::OpenClipFolder) => crate::feedback::open_folder(clip_dir),
            (None, _) => crate::feedback::open_app(),
        }
    }
}

/// A line of the menu, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// What the recorder is doing: shown greyed out, not to be chosen.
    Status(String),
    Separator,
    Item { label: &'static str, action: Action, enabled: bool },
    /// One of a group of which exactly one is `checked`.
    Choice { label: &'static str, action: Action, checked: bool },
}

fn capturing(status: &Status) -> bool {
    matches!(status.state.as_str(), "capturing" | "saving")
}

/// The status in a few words.
pub fn headline(status: &Status) -> String {
    match (status.state.as_str(), &status.game) {
        ("capturing" | "saving", Some(game)) => format!("Capturing {game}"),
        ("capturing" | "saving", None) => "Capturing".into(),
        ("restarting", _) => "Restarting the recorder".into(),
        ("failed", _) => format!("Capture failed: {}", status.detail.as_deref().unwrap_or("the recorder stopped")),
        _ if status.forced == Some(false) => "Capture is off".into(),
        _ => "Waiting for a game".into(),
    }
}

/// The menu for this status. Its shape never changes (the same entries in
/// the same order, whatever the status), only what they say and which are
/// enabled or checked: a tray may build it once and update it in place.
pub fn entries(status: &Status) -> Vec<Entry> {
    let choice = |label, force| Entry::Choice { label, action: Action::Capture(force), checked: status.forced == force };
    vec![
        Entry::Status(headline(status)),
        Entry::Separator,
        // With nothing being captured there is nothing to save.
        Entry::Item { label: "Save clip", action: Action::Save, enabled: capturing(status) },
        Entry::Item { label: "Open Zenith", action: Action::OpenApp, enabled: true },
        Entry::Item { label: "Open the clip folder", action: Action::OpenClipFolder, enabled: true },
        Entry::Separator,
        choice("Capture: automatic", None),
        choice("Capture: always on", Some(true)),
        choice("Capture: off", Some(false)),
        Entry::Separator,
        Entry::Item { label: "Quit recorder", action: Action::Quit, enabled: true },
    ]
}

/// The longest tooltip Windows shows, in the units it counts in (UTF-16):
/// its room is 128 with the end mark.
#[cfg(any(windows, test))]
const TOOLTIP_UNITS: usize = 127;

/// What hovering over the icon says: the name and the status, cut to what
/// Windows has room for. (The tray library copies a longer one without its
/// end mark, so it must not be given one.)
#[cfg(any(windows, test))]
pub fn tooltip(status: &Status) -> String {
    let full = format!("Zenith · {}", headline(status));
    if full.encode_utf16().count() <= TOOLTIP_UNITS {
        return full;
    }
    let mut cut = String::new();
    let mut units = 1; // the "…" that says it was cut
    for c in full.chars() {
        units += c.len_utf16();
        if units > TOOLTIP_UNITS {
            break;
        }
        cut.push(c);
    }
    cut.push('…');
    cut
}

/// Everything a tray shows that follows from the status. Two statuses
/// with the same view (say, one more clip saved) need no redrawing.
#[cfg(any(windows, test))]
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub tooltip: String,
    pub entries: Vec<Entry>,
}

#[cfg(any(windows, test))]
pub fn view(status: &Status) -> View {
    View { tooltip: tooltip(status), entries: entries(status) }
}

/// The desktop's side of a tray icon that has to be looked after by the
/// thread that made it (Windows).
#[cfg(any(windows, test))]
pub trait Shell {
    /// Make the icon and its menu say this.
    fn show(&mut self, view: &View);
    /// Let the desktop deliver what happened to the icon since the last
    /// call, and give what the user chose.
    fn pump(&mut self) -> Vec<Action>;
}

/// Look after an icon that shows `status` to begin with: carry out what
/// the user chooses and show each new status, until the daemon lets go of
/// `shows`. The icon is taken away by dropping `shell` on the way out.
#[cfg(any(windows, test))]
pub fn keep(
    mut shell: impl Shell,
    status: &Status,
    mut clip_dir: std::path::PathBuf,
    shows: std::sync::mpsc::Receiver<(Status, std::path::PathBuf)>,
    tx: mpsc::UnboundedSender<Msg>,
) {
    use std::sync::mpsc::RecvTimeoutError;
    let mut shown = view(status);
    loop {
        let chosen = shell.pump();
        for action in &chosen {
            action.perform(&clip_dir, &tx);
        }
        // A menu ticks the entry that was clicked by itself, but which one
        // is ticked is the daemon's to say: put it back as it was, and let
        // the new status move it if the daemon agrees.
        if chosen.iter().any(|action| matches!(action, Action::Capture(_))) {
            shell.show(&shown);
        }
        let mut latest = match shows.recv_timeout(crate::hotkey::PUMP_EVERY) {
            Ok(latest) => latest,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        // Several at once: only the last is worth showing.
        while let Ok(newer) = shows.try_recv() {
            latest = newer;
        }
        clip_dir = latest.1;
        let next = view(&latest.0);
        if next != shown {
            shell.show(&next);
            shown = next;
        }
    }
}

/// The image in an `.ico` file that suits an icon `want` pixels wide: the
/// smallest that is at least that wide, which the desktop scales down
/// cleanly, or else the widest there is.
#[cfg(any(windows, test))]
fn ico_image(ico: &[u8], want: u32) -> Option<&[u8]> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(ico.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(ico.get(at..at + 4)?.try_into().ok()?));
    // The file starts with a zero, a one for "icons", and how many.
    if (u16_at(0)?, u16_at(2)?) != (0, 1) {
        return None;
    }
    let mut best: Option<(u32, &[u8])> = None;
    for index in 0..u16_at(4)? as usize {
        // Sixteen bytes for each: the width first (0 stands for 256), the
        // image's length and where it starts last.
        let entry = 6 + 16 * index;
        let width = match *ico.get(entry)? {
            0 => 256,
            width => u32::from(width),
        };
        let start = u32_at(entry + 12)? as usize;
        let image = ico.get(start..start.checked_add(u32_at(entry + 8)? as usize)?)?;
        let better = match best {
            None => true,
            Some((have, _)) if have >= want => width >= want && width < have,
            Some((have, _)) => width > have,
        };
        if better {
            best = Some((width, image));
        }
    }
    best.map(|(_, image)| image)
}

/// The pixels of the tray icon at about `want` wide, out of an `.ico`
/// file whose images are PNGs (ours are): four bytes each, red, green,
/// blue, alpha, row by row, with the width and height.
#[cfg(any(windows, test))]
pub fn icon_pixels(ico: &[u8], want: u32) -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(ico_image(ico, want)?));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    if frame.color_type != png::ColorType::Rgba {
        return None;
    }
    pixels.truncate(frame.buffer_size());
    Some((pixels, frame.width, frame.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const ICO: &[u8] = include_bytes!("../assets/zenith.ico");

    fn status(state: &str, game: Option<&str>, forced: Option<bool>, detail: Option<&str>) -> Status {
        Status { state: state.into(), game: game.map(Into::into), forced, detail: detail.map(Into::into), buffer_secs: 30, ..Default::default() }
    }

    #[test]
    fn the_status_is_said_in_words() {
        for (status, says) in [
            (status("capturing", Some("STRAFE"), None, None), "Capturing STRAFE"),
            (status("saving", Some("STRAFE"), None, None), "Capturing STRAFE"),
            (status("capturing", None, Some(true), None), "Capturing"),
            (status("idle", None, None, None), "Waiting for a game"),
            (status("idle", None, Some(false), None), "Capture is off"),
            (status("idle", None, None, Some("The save hotkey Alt+F9 does not work")), "Waiting for a game"),
            (status("restarting", None, None, Some("attempt 2")), "Restarting the recorder"),
            (status("failed", None, None, Some("no encoder")), "Capture failed: no encoder"),
            (status("failed", None, None, None), "Capture failed: the recorder stopped"),
        ] {
            assert_eq!(headline(&status), says);
            assert_eq!(tooltip(&status), format!("Zenith · {says}"));
        }
    }

    #[test]
    fn a_long_tooltip_is_cut_to_what_windows_shows() {
        let long = status("failed", None, None, Some(&"é𝄞".repeat(200)));
        let tip = tooltip(&long);
        let units = tip.encode_utf16().count();
        assert!((TOOLTIP_UNITS - 1..=TOOLTIP_UNITS).contains(&units), "{units}");
        assert!(tip.starts_with("Zenith · Capture failed: é𝄞") && tip.ends_with('…'), "{tip}");
    }

    #[test]
    fn the_menu_offers_the_same_entries_whatever_the_status() {
        let idle = entries(&status("idle", None, None, None));
        let labels: Vec<&str> = idle
            .iter()
            .map(|entry| match entry {
                Entry::Status(label) => label.as_str(),
                Entry::Separator => "-",
                Entry::Item { label, .. } | Entry::Choice { label, .. } => label,
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Waiting for a game",
                "-",
                "Save clip",
                "Open Zenith",
                "Open the clip folder",
                "-",
                "Capture: automatic",
                "Capture: always on",
                "Capture: off",
                "-",
                "Quit recorder"
            ]
        );
        let shape = |entries: &[Entry]| entries.iter().map(std::mem::discriminant).collect::<Vec<_>>();
        for other in [status("capturing", Some("STRAFE"), Some(true), None), status("failed", None, Some(false), Some("no encoder"))] {
            assert_eq!(shape(&entries(&other)), shape(&idle));
        }
    }

    #[test]
    fn a_clip_can_only_be_saved_while_capturing() {
        let save_enabled = |state| {
            entries(&status(state, None, None, None)).into_iter().find_map(|entry| match entry {
                Entry::Item { action: Action::Save, enabled, .. } => Some(enabled),
                _ => None,
            })
        };
        assert_eq!(save_enabled("capturing"), Some(true));
        assert_eq!(save_enabled("saving"), Some(true));
        assert_eq!(save_enabled("idle"), Some(false));
        assert_eq!(save_enabled("failed"), Some(false));
    }

    #[test]
    fn exactly_the_current_capture_choice_is_checked() {
        for forced in [None, Some(true), Some(false)] {
            let checked: Vec<Action> = entries(&status("idle", None, forced, None))
                .into_iter()
                .filter_map(|entry| match entry {
                    Entry::Choice { action, checked: true, .. } => Some(action),
                    _ => None,
                })
                .collect();
            assert_eq!(checked, [Action::Capture(forced)]);
        }
    }

    #[test]
    fn each_action_asks_the_daemon_for_the_right_thing() {
        assert_eq!(Action::Save.request(), Some(Request::Save));
        assert_eq!(Action::Quit.request(), Some(Request::Quit));
        for force in [None, Some(true), Some(false)] {
            assert_eq!(Action::Capture(force).request(), Some(Request::Capture { force }));
        }
        assert_eq!(Action::OpenApp.request(), None, "opening the app is not the daemon's business");
        assert_eq!(Action::OpenClipFolder.request(), None);
    }

    #[test]
    fn every_entry_that_can_be_chosen_is_told_apart_by_its_name() {
        let mut seen = Vec::new();
        for entry in entries(&status("capturing", None, None, None)) {
            if let Entry::Item { action, .. } | Entry::Choice { action, .. } = entry {
                assert_eq!(Action::from_id(action.id()), Some(action));
                assert!(!seen.contains(&action.id()), "{} is there twice", action.id());
                seen.push(action.id());
            }
        }
        assert_eq!(seen.len(), ACTIONS.len(), "every action is in the menu");
        assert_eq!(Action::from_id("something else"), None);
    }

    /// A desktop that writes down what it is shown. Each time it is asked
    /// what happened, the next step of `script` does: the user makes these
    /// choices and the daemon has this new status. After the last step the
    /// daemon lets go.
    struct Fake {
        script: Vec<(Vec<Action>, Option<Status>)>,
        daemon: Option<std::sync::mpsc::Sender<(Status, std::path::PathBuf)>>,
        shown: Arc<Mutex<Vec<String>>>,
    }

    impl Shell for Fake {
        fn show(&mut self, view: &View) {
            let checked = view.entries.iter().find_map(|entry| match entry {
                Entry::Choice { label, checked: true, .. } => Some(*label),
                _ => None,
            });
            self.shown.lock().unwrap().push(format!("{} / {}", view.tooltip, checked.unwrap_or("nothing")));
        }

        fn pump(&mut self) -> Vec<Action> {
            if self.script.is_empty() {
                self.daemon = None;
                return Vec::new();
            }
            let (chosen, status) = self.script.remove(0);
            if let (Some(status), Some(daemon)) = (status, &self.daemon) {
                daemon.send((status, "clips".into())).unwrap();
            }
            chosen
        }
    }

    /// Run `keep` from an idle status over this script, and give what it
    /// showed and what it asked the daemon.
    fn kept(script: Vec<(Vec<Action>, Option<Status>)>) -> (Vec<String>, Vec<Request>) {
        let shown = Arc::new(Mutex::new(Vec::new()));
        let (daemon, shows) = std::sync::mpsc::channel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let fake = Fake { script, daemon: Some(daemon), shown: shown.clone() };
        keep(fake, &status("idle", None, None, None), "clips".into(), shows, tx);
        let mut asked = Vec::new();
        while let Ok(Msg::Ipc(request, _)) = rx.try_recv() {
            asked.push(request);
        }
        let shown = shown.lock().unwrap().clone();
        (shown, asked)
    }

    #[test]
    fn a_new_status_is_shown_only_when_it_looks_different() {
        let capturing = status("capturing", Some("STRAFE"), None, None);
        let one_more_clip = Status { clips_saved: 1, ..capturing.clone() };
        let script = [status("idle", None, None, None), capturing, one_more_clip, status("idle", None, Some(false), None)];
        let (shown, asked) = kept(script.into_iter().map(|status| (Vec::new(), Some(status))).collect());
        assert_eq!(shown, ["Zenith · Capturing STRAFE / Capture: automatic", "Zenith · Capture is off / Capture: off"]);
        assert!(asked.is_empty());
    }

    #[test]
    fn what_is_chosen_is_asked_of_the_daemon() {
        let (shown, asked) = kept(vec![(vec![Action::Save, Action::Quit], None)]);
        assert_eq!(asked, [Request::Save, Request::Quit]);
        assert!(shown.is_empty(), "nothing changed, nothing is redrawn");
    }

    #[test]
    fn a_capture_choice_is_ticked_by_the_daemon_not_by_the_click() {
        let (shown, asked) = kept(vec![(vec![Action::Capture(Some(false))], None), (Vec::new(), Some(status("idle", None, Some(false), None)))]);
        assert_eq!(asked, [Request::Capture { force: Some(false) }]);
        assert_eq!(
            shown,
            ["Zenith · Waiting for a game / Capture: automatic", "Zenith · Capture is off / Capture: off"],
            "the tick is put back, and moves when the status says so"
        );
    }

    #[test]
    fn the_icon_file_has_an_image_for_every_tray_size() {
        // 16 pixels at normal scale, 24 at 150 %, 32 at 200 %; 20 (125 %)
        // takes the next one up, and past the largest the largest is used.
        for (want, gets) in [(16, 16), (20, 24), (24, 24), (32, 32), (40, 48), (1000, 256)] {
            let (pixels, width, height) = icon_pixels(ICO, want).expect("the icon is read");
            assert_eq!((width, height), (gets, gets), "for {want}");
            assert_eq!(pixels.len(), (gets * gets * 4) as usize);
            assert!(pixels.chunks(4).any(|pixel| pixel[3] != 0), "something is drawn");
        }
    }

    #[test]
    fn a_file_that_is_not_an_icon_gives_none() {
        assert_eq!(icon_pixels(b"", 16), None);
        assert_eq!(icon_pixels(b"not an icon at all, just words", 16), None);
        assert_eq!(icon_pixels(&ICO[..40], 16), None, "cut short");
        let mut no_images = ICO[..6].to_vec();
        no_images[4] = 0;
        assert_eq!(icon_pixels(&no_images, 16), None);
    }
}
