//! The tray icon on Windows: Zenith's mark among the icons by the clock,
//! with the status as its tooltip and a menu on a right click. What it
//! says and offers is `tray_menu`'s, shared with the Linux tray; here it
//! is only handed to Windows. The same calls as `tray`, so the daemon need
//! not know which it has.
//!
//! Windows tells of a click on the icon by a message to the thread that
//! made the icon, which has to fetch it. That is a thread of this file's
//! own and not the hotkey's, though both fetch messages the same way: the
//! hotkey must go on working when there is no tray and the other way
//! round, and while the menu is open Windows keeps its thread to itself,
//! which must not hold up a new hotkey setting.

use crate::daemon::Msg;
use crate::tray_menu::{self, Action, Entry, Shell, View};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use zenith_core::ipc::Status;

/// Zenith's mark, in every size a tray asks for.
const ICO: &[u8] = include_bytes!("../assets/zenith.ico");

/// The daemon's hold on the icon, which lives on a thread of its own.
pub struct Icon {
    shows: std::sync::mpsc::Sender<(Status, PathBuf)>,
    gone: oneshot::Receiver<()>,
}

/// A line of the menu as Windows has it, kept to change it in place.
enum Row {
    Text(MenuItem),
    Tick(CheckMenuItem),
    Line(PredefinedMenuItem),
}

impl Row {
    fn item(&self) -> &dyn IsMenuItem {
        match self {
            Row::Text(item) => item,
            Row::Tick(item) => item,
            Row::Line(item) => item,
        }
    }
}

/// A label as a Windows menu shows it: there a single `&` underlines the
/// next letter instead of being shown, and a game may well have one.
fn label(text: &str) -> String {
    text.replace('&', "&&")
}

/// How long after a click on the icon another is taken as the second
/// half of a double click, which should open the app once and not twice.
const DOUBLE_CLICK: Duration = Duration::from_millis(600);

/// The icon in Windows' tray.
struct System {
    /// Dropped to take the icon out of the tray.
    tray: TrayIcon,
    rows: Vec<Row>,
    clicked: Option<Instant>,
    /// Whether it was already said that the tooltip cannot be changed.
    told: bool,
}

/// Zenith's mark at the size the tray draws icons.
fn mark() -> Result<tray_icon::Icon, String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    // SAFETY: only asks a number of the system. 0 says it is not known;
    // then the usual 16 is as good a guess as any.
    let small = match unsafe { GetSystemMetrics(SM_CXSMICON) } {
        size if size > 0 => size as u32,
        _ => 16,
    };
    let (pixels, width, height) = tray_menu::icon_pixels(ICO, small).ok_or("its image could not be read")?;
    tray_icon::Icon::from_rgba(pixels, width, height).map_err(|e| e.to_string())
}

impl System {
    /// Put the icon in the tray, saying `view`. Must be called on the
    /// thread that goes on to fetch its messages.
    fn new(view: &View) -> Result<Self, String> {
        let menu = Menu::new();
        let mut rows = Vec::new();
        for entry in &view.entries {
            let row = match entry {
                Entry::Status(text) => Row::Text(MenuItem::new(label(text), false, None)),
                Entry::Separator => Row::Line(PredefinedMenuItem::separator()),
                Entry::Item { label: text, action, enabled } => Row::Text(MenuItem::with_id(action.id(), label(text), *enabled, None)),
                Entry::Choice { label: text, action, checked } => Row::Tick(CheckMenuItem::with_id(action.id(), label(text), true, *checked, None)),
            };
            menu.append(row.item()).map_err(|e| e.to_string())?;
            rows.push(row);
        }
        let tray = TrayIconBuilder::new()
            .with_icon(mark()?)
            .with_tooltip(&view.tooltip)
            .with_menu(Box::new(menu))
            // A left click opens the app; the menu is for the right one.
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { tray, rows, clicked: None, told: false })
    }
}

impl Shell for System {
    fn show(&mut self, view: &View) {
        for (row, entry) in self.rows.iter().zip(&view.entries) {
            match (row, entry) {
                (Row::Text(item), Entry::Status(text)) => item.set_text(label(text)),
                (Row::Text(item), Entry::Item { enabled, .. }) => item.set_enabled(*enabled),
                (Row::Tick(item), Entry::Choice { checked, .. }) => item.set_checked(*checked),
                // The menu's shape does not change (`tray_menu::entries`).
                _ => {}
            }
        }
        match self.tray.set_tooltip(Some(view.tooltip.as_str())) {
            Ok(()) => self.told = false,
            // As when Explorer is not running; it is asked again with the
            // next status, so once is enough to say.
            Err(e) if !self.told => {
                self.told = true;
                crate::warn(format_args!("zenithd: the tray icon's tooltip could not be changed: {e}"));
            }
            Err(_) => {}
        }
    }

    /// A click arrives as a message for the icon's hidden window, which
    /// only reaches it when this thread fetches and hands it on; what the
    /// window makes of it is then waiting in the library's two queues.
    fn pump(&mut self) -> Vec<Action> {
        crate::hotkey::pump_messages();
        let mut chosen = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            chosen.extend(Action::from_id(event.id.as_ref()));
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            let left_click = matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. });
            if left_click && self.clicked.is_none_or(|before| before.elapsed() > DOUBLE_CLICK) {
                self.clicked = Some(Instant::now());
                chosen.push(Action::OpenApp);
            }
        }
        chosen
    }
}

/// Put the icon in the tray. `None` when not even its thread could be
/// started; the recorder works the same without.
pub async fn start(status: Status, clip_dir: PathBuf, tx: mpsc::UnboundedSender<Msg>) -> Option<Icon> {
    let (shows, received) = std::sync::mpsc::channel();
    let (left, gone) = oneshot::channel();
    let thread = std::thread::Builder::new().name("tray".into()).spawn(move || {
        match System::new(&tray_menu::view(&status)) {
            // Returns, and with that takes the icon away, when the daemon
            // lets go of `shows`.
            Ok(system) => tray_menu::keep(system, &status, clip_dir, received, tx),
            Err(e) => crate::warn(format_args!("zenithd: no tray icon: {e}")),
        }
        let _ = left.send(());
    });
    match thread {
        Ok(_) => Some(Icon { shows, gone }),
        Err(e) => {
            crate::warn(format_args!("zenithd: no tray icon: {e}"));
            None
        }
    }
}

/// Show a new status. Whether it looks any different from the one shown
/// is for the icon's thread to say.
pub async fn show(icon: &Option<Icon>, status: Status, clip_dir: &std::path::Path) {
    if let Some(icon) = icon {
        // Fails when there is no icon after all, which was said then.
        let _ = icon.shows.send((status, clip_dir.to_path_buf()));
    }
}

/// Take the icon out of the tray again, so that none is left behind
/// pointing at a recorder that is gone.
pub async fn stop(icon: Option<Icon>) {
    let Some(Icon { shows, gone }) = icon else { return };
    drop(shows);
    // Not waited for longer than this: with its menu open, Windows keeps
    // the icon's thread until the menu is closed.
    if tokio::time::timeout(Duration::from_secs(1), gone).await.is_err() {
        crate::warn("zenithd: the tray icon was not taken away in time; it goes when the mouse next passes over it");
    }
}
