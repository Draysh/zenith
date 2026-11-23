//! The window shown while an update is put in place on Windows.
//!
//! The new version comes as a setup program, which cannot replace files
//! that are in use: the app closes, the setup program runs, the app opens
//! again. Run by itself the setup program shows its own window, which
//! looks like nothing else of Zenith's. So the app starts this instead (a
//! copy of this program, kept outside the folder being replaced):
//! `zenithd update <setup> <version> <app>` runs the setup program without
//! any window of its own and shows a small one of Zenith's meanwhile, in
//! the colours the user chose, with a bar that moves until it is done.
//! If the setup program fails, the window says so and the app that was
//! there is opened again.

use crate::overlay::{monitor_of, rgb, wide};
use std::process::ExitCode;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DT_END_ELLIPSIS,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect, GetDC, GetDeviceCaps, HDC, InvalidateRect,
    LOGPIXELSY, PAINTSTRUCT, ReleaseDC, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect, GetForegroundWindow, GetMessageW, GetWindowLongPtrW,
    KillTimer, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOW, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage, WM_APP, WM_DESTROY,
    WM_ERASEBKGND, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use zenith_core::config::{Config, Look, rgb as parse};

/// The timer that moves the bar, and the one that closes the window.
const MOVE: usize = 1;
const CLOSE: usize = 2;
/// Sent to the window by the thread running the setup program when that
/// has ended; `wparam` is 0 when it worked.
const DONE: u32 = WM_APP + 1;

/// The window's colours: Zenith's, or the user's own where they chose some.
struct Colours {
    ground: COLORREF,
    line: COLORREF,
    mark: COLORREF,
    bar: COLORREF,
    text: COLORREF,
    dim: COLORREF,
}

impl Colours {
    /// As the app's `Theme` works them out from the same settings.
    fn of(look: &Look) -> Self {
        let own = |text: &str, standard: (u8, u8, u8)| parse(text).unwrap_or(standard);
        let ground = own(&look.background, (0x06, 0x09, 0x0B));
        let luma = (0.2126 * ground.0 as f64 + 0.7152 * ground.1 as f64 + 0.0722 * ground.2 as f64) / 255.0;
        let light = luma > 0.35;
        // A step from the background towards white, or black on a light one.
        let step = |amount: f64| {
            let towards = if light { 0.0 } else { 255.0 };
            let mix = |c: u8| (c as f64 + (towards - c as f64) * amount).round() as u8;
            (mix(ground.0), mix(ground.1), mix(ground.2))
        };
        let line = match (parse(&look.line), parse(&look.background)) {
            (Some(line), _) => line,
            (None, Some(_)) => step(0.17),
            (None, None) => (0x25, 0x33, 0x3D),
        };
        let colour = |c: (u8, u8, u8)| rgb(c.0, c.1, c.2);
        Self {
            ground: colour(ground),
            line: colour(line),
            mark: colour(own(&look.accent, (0x3D, 0xE0, 0xF5))),
            bar: colour(own(&look.action, (0xFF, 0x3D, 0xA5))),
            text: if light { rgb(0x0C, 0x11, 0x14) } else { rgb(0xEE, 0xF4, 0xF6) },
            dim: if light { rgb(0x39, 0x46, 0x4D) } else { rgb(0xA9, 0xB8, 0xBF) },
        }
    }
}

/// What the window shows now.
struct Shown {
    colours: Colours,
    title: Vec<u16>,
    detail: Vec<u16>,
    /// How far the bar has moved, in steps of the timer.
    moved: u32,
    /// The setup program has ended; the bar is gone.
    ended: bool,
    scale: f64,
}

/// Draw the window: its box, the mark down the left side, the two lines
/// of words, and under them the bar going from left to right. Drawn off
/// screen first and put on in one go, so the bar moves without a flicker.
///
/// SAFETY: `window` is this file's window, on the thread that made it.
unsafe fn paint(window: HWND, shown: &Shown) {
    unsafe {
        let px = |n: f64| (n * shown.scale).round() as i32;
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let screen = BeginPaint(window, &mut ps);
        let mut all: RECT = std::mem::zeroed();
        GetClientRect(window, &mut all);
        let dc: HDC = CreateCompatibleDC(screen);
        let picture = CreateCompatibleBitmap(screen, all.right, all.bottom);
        let before_picture = SelectObject(dc, picture);

        let c = &shown.colours;
        let brushes = [c.ground, c.line, c.mark, c.bar].map(|colour| CreateSolidBrush(colour));
        let [ground, line, mark, bar] = brushes;
        FillRect(dc, &all, ground);
        FrameRect(dc, &all, line);
        FillRect(dc, &RECT { left: 0, top: 0, right: px(4.0), bottom: all.bottom }, mark);

        SetBkMode(dc, TRANSPARENT as i32);
        let face = wide("Segoe UI");
        let font = |height: f64, weight: i32| CreateFontW(-px(height), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, face.as_ptr());
        let (bold, plain) = (font(17.0, 700), font(13.0, 400));
        let how = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
        let (left, right) = (px(26.0), all.right - px(24.0));
        let before_font = SelectObject(dc, bold);
        SetTextColor(dc, c.text);
        let mut top = RECT { left, top: px(22.0), right, bottom: px(48.0) };
        DrawTextW(dc, shown.title.as_ptr(), -1, &mut top, how);
        SelectObject(dc, plain);
        SetTextColor(dc, c.dim);
        let mut under = RECT { left, top: px(50.0), right, bottom: px(72.0) };
        DrawTextW(dc, shown.detail.as_ptr(), -1, &mut under, how);
        SelectObject(dc, before_font);

        if !shown.ended {
            // A piece of the bar's colour running along its groove, in at
            // one end and out at the other, for as long as it takes.
            let groove = RECT { left, top: all.bottom - px(30.0), right, bottom: all.bottom - px(24.0) };
            FillRect(dc, &groove, line);
            let (length, piece) = (groove.right - groove.left, px(96.0));
            let at = groove.left - piece + (shown.moved as i32 * px(5.0)) % (length + piece);
            let lit = RECT { left: at.max(groove.left), top: groove.top, right: (at + piece).min(groove.right), bottom: groove.bottom };
            if lit.right > lit.left {
                FillRect(dc, &lit, bar);
            }
        }

        BitBlt(screen, 0, 0, all.right, all.bottom, dc, 0, 0, SRCCOPY);
        SelectObject(dc, before_picture);
        for object in [bold, plain, picture as _] {
            DeleteObject(object);
        }
        for brush in brushes {
            DeleteObject(brush as _);
        }
        DeleteDC(dc);
        EndPaint(window, &ps);
    }
}

unsafe extern "system" fn handle(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: called by Windows for this file's window on the thread that
    // made it. What the window points at is the `Shown` `run` gave it,
    // which lives until that thread's message loop has ended, and nothing
    // else reads or writes it.
    unsafe {
        let shown = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Shown;
        if shown.is_null() {
            return DefWindowProcW(window, message, wparam, lparam);
        }
        match message {
            WM_PAINT => {
                paint(window, &*shown);
                0
            }
            // All of it is drawn by `paint`: nothing to clear first.
            WM_ERASEBKGND => 1,
            WM_TIMER if wparam == MOVE => {
                (*shown).moved = (*shown).moved.wrapping_add(1);
                InvalidateRect(window, null(), 0);
                0
            }
            WM_TIMER => {
                DestroyWindow(window);
                0
            }
            DONE => {
                KillTimer(window, MOVE);
                (*shown).ended = true;
                let worked = wparam == 0;
                if worked {
                    (*shown).detail = wide("Done. Opening Zenith\u{2026}");
                } else {
                    (*shown).title = wide("The update did not go in");
                    (*shown).detail = wide("Zenith is as it was, and opens again in a moment.");
                }
                InvalidateRect(window, null(), 0);
                // Long enough to read, and for the app's window to come up.
                SetTimer(window, CLOSE, if worked { 1500 } else { 6000 }, None);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

/// Run the setup program at `setup` without a window of its own, showing
/// Zenith's meanwhile. The setup program opens the new app itself; if it
/// fails, the app at `app` is opened again.
pub fn run(setup: &str, version: &str, app: &str) -> ExitCode {
    // What happened to an update is kept beside the other logs: this
    // program shows no console, and is gone when it is done.
    if let Err(e) = crate::logfile::start(&zenith_core::paths::data_dir().join("logs").join("update.log")) {
        crate::warn(format_args!("zenithd: the update keeps no log: {e}"));
    }
    crate::warn(format_args!("zenithd: updating to {version} with {setup}, behind Zenith's own window"));
    let look = Config::load_lenient(&zenith_core::paths::config_file()).map(|(cfg, _)| cfg.look).unwrap_or_default();
    // SAFETY: a window made, shown and destroyed on this one thread, with
    // this thread fetching its messages; every pointer handed over
    // outlives the call it is handed to. The other thread is given the
    // window only as a number to post a message to.
    let worked = unsafe {
        let class = wide("ZenithUpdate");
        let program = GetModuleHandleW(null());
        let mut kind: WNDCLASSW = std::mem::zeroed();
        kind.lpfnWndProc = Some(handle);
        kind.hInstance = program;
        kind.lpszClassName = class.as_ptr();
        RegisterClassW(&kind);

        let screen = GetDC(null_mut());
        let scale = (GetDeviceCaps(screen, LOGPIXELSY as i32) as f64 / 96.0).clamp(1.0, 4.0);
        ReleaseDC(null_mut(), screen);
        let px = |n: f64| (n * scale).round() as i32;
        let (width, height) = (px(400.0), px(124.0));
        // In the middle of the monitor the app was on.
        let (x, y) = monitor_of(GetForegroundWindow())
            .map_or((px(200.0), px(200.0)), |m| ((m.left + m.right - width) / 2, (m.top + m.bottom - height) / 2));

        let title = wide("Zenith");
        let window =
            CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, class.as_ptr(), title.as_ptr(), WS_POPUP, x, y, width, height, null_mut(), null_mut(), program, null());
        // Without a window the update still goes in, unseen.
        let shown = Box::into_raw(Box::new(Shown {
            colours: Colours::of(&look),
            title: wide(&if version.is_empty() { "Updating Zenith".to_string() } else { format!("Updating Zenith to {version}") }),
            detail: wide("This takes a moment. Zenith opens again by itself."),
            moved: 0,
            ended: false,
            scale,
        }));
        if window.is_null() {
            crate::warn("zenithd: the update's window could not be made; the update goes in unseen");
        } else {
            SetWindowLongPtrW(window, GWLP_USERDATA, shown as isize);
            ShowWindow(window, SW_SHOW);
            SetTimer(window, MOVE, 30, None);
        }

        let (setup, to) = (setup.to_string(), window as isize);
        let installing = std::thread::spawn(move || {
            let ended = std::process::Command::new(&setup).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]).status();
            let worked = ended.as_ref().is_ok_and(|status| status.success());
            if !worked {
                crate::warn(format_args!("zenithd: the setup program {setup} did not finish: {ended:?}"));
            }
            if to != 0 {
                PostMessageW(to as HWND, DONE, usize::from(!worked), 0);
            }
            (worked, setup)
        });

        if !window.is_null() {
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let (worked, setup) = installing.join().unwrap_or((false, String::new()));
        drop(Box::from_raw(shown));
        if worked {
            // It has done its work.
            let _ = std::fs::remove_file(format!("{setup}.minisig"));
            let _ = std::fs::remove_file(setup);
        }
        worked
    };
    if worked {
        crate::warn(format_args!("zenithd: the setup program finished; Zenith {version} is in place"));
        return ExitCode::SUCCESS;
    }
    if !app.is_empty()
        && let Err(e) = std::process::Command::new(app).spawn()
    {
        crate::warn(format_args!("zenithd: {app} could not be opened again: {e}"));
    }
    ExitCode::FAILURE
}
