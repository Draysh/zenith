//! A notice drawn over the game. Windows keeps its own notifications back
//! while a game fills the screen, so "Clip saved" was never seen there.
//! This is a small window of Zenith's own that stays above every other,
//! never takes the keyboard and lets the mouse through, shown in a corner
//! of the monitor the game is on for a few seconds.
//!
//! It shows over a game that runs in a borderless window, which is what
//! nearly all do (and what Windows turns "fullscreen" into for most of the
//! rest). Over a game that has the screen to itself the old way, nothing
//! of any other program can be drawn.

use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject,
    DrawTextW, EndPaint, FillRect, FrameRect, GetDC, GetDeviceCaps, GetMonitorInfoW, LOGPIXELSY, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow,
    PAINTSTRUCT, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClassNameW, GetClientRect, GetForegroundWindow, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, LWA_ALPHA, MSG, PostQuitMessage, RegisterClassW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, WM_DESTROY, WM_PAINT,
    WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// How long the notice stays, in milliseconds.
const SHOWN_MS: u32 = 2600;

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A colour as Windows takes it.
pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// The monitor a window is on: all of it, taskbar included.
pub(crate) fn monitor_of(window: HWND) -> Option<RECT> {
    // SAFETY: both calls only read; `info` is the size it says it is.
    unsafe {
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        (GetMonitorInfoW(monitor, &mut info) != 0).then_some(info.rcMonitor)
    }
}

/// Whether the window in front fills its monitor, as a game does: Windows
/// then keeps its own notifications to itself. The desktop fills it too,
/// and is not a game.
pub fn covers_screen() -> bool {
    // SAFETY: the calls only read, into buffers of the sizes given.
    unsafe {
        let front = GetForegroundWindow();
        if front.is_null() {
            return false;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(front, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return false;
        }
        let mut at: RECT = std::mem::zeroed();
        if GetWindowRect(front, &mut at) == 0 {
            return false;
        }
        monitor_of(front).is_some_and(|m| at.left <= m.left && at.top <= m.top && at.right >= m.right && at.bottom >= m.bottom)
    }
}

/// What a notice says, and how large things are drawn on this screen.
struct Notice {
    title: Vec<u16>,
    detail: Vec<u16>,
    scale: f64,
}

/// Draw the notice: Zenith's dark box with its cyan mark down the left
/// side, the title in bold and the detail under it.
///
/// SAFETY: `window` is the notice's window, on the thread that made it.
unsafe fn paint(window: HWND, notice: &Notice) {
    unsafe {
        let px = |n: f64| (n * notice.scale).round() as i32;
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(window, &mut ps);
        let mut all: RECT = std::mem::zeroed();
        GetClientRect(window, &mut all);

        let ground = CreateSolidBrush(rgb(0x06, 0x09, 0x0B));
        let line = CreateSolidBrush(rgb(0x25, 0x33, 0x3D));
        let mark = CreateSolidBrush(rgb(0x3D, 0xE0, 0xF5));
        FillRect(dc, &all, ground);
        FrameRect(dc, &all, line);
        FillRect(dc, &RECT { left: 0, top: 0, right: px(4.0), bottom: all.bottom }, mark);

        SetBkMode(dc, TRANSPARENT as i32);
        let face = wide("Segoe UI");
        let font = |height: f64, weight: i32| CreateFontW(-px(height), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, face.as_ptr());
        let (bold, plain) = (font(15.0, 700), font(13.0, 400));
        let how = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
        let (left, right, middle) = (px(18.0), all.right - px(14.0), all.bottom / 2);

        let before = SelectObject(dc, bold);
        SetTextColor(dc, rgb(0xEE, 0xF4, 0xF6));
        let mut top = RECT { left, top: px(8.0), right, bottom: middle + px(1.0) };
        DrawTextW(dc, notice.title.as_ptr(), -1, &mut top, how);
        SelectObject(dc, plain);
        SetTextColor(dc, rgb(0xA9, 0xB8, 0xBF));
        let mut under = RECT { left, top: middle - px(1.0), right, bottom: all.bottom - px(8.0) };
        DrawTextW(dc, notice.detail.as_ptr(), -1, &mut under, how);
        SelectObject(dc, before);

        for object in [bold, plain, ground as _, line as _, mark as _] {
            DeleteObject(object);
        }
        EndPaint(window, &ps);
    }
}

unsafe extern "system" fn handle(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: called by Windows for the notice's window on its own thread.
    // What the window points at is the `Notice` `show` gave it, which
    // lives until that thread's message loop has ended.
    unsafe {
        match message {
            WM_PAINT => {
                let notice = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Notice;
                if notice.is_null() {
                    return DefWindowProcW(window, message, wparam, lparam);
                }
                paint(window, &*notice);
                0
            }
            // Its time is up.
            WM_TIMER => {
                DestroyWindow(window);
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

/// Show a notice over whatever is on screen, in the top right corner of
/// the monitor the window in front is on. Returns at once; the notice
/// goes away by itself.
pub fn show(title: &str, detail: &str) {
    let (title, detail) = (wide(title), wide(detail));
    std::thread::spawn(move || {
        // SAFETY: a window made, shown and destroyed on this one thread,
        // with this thread fetching its messages; every pointer handed
        // over outlives the call it is handed to.
        unsafe {
            let class = wide("ZenithNotice");
            let program = GetModuleHandleW(null());
            let mut kind: WNDCLASSW = std::mem::zeroed();
            kind.lpfnWndProc = Some(handle);
            kind.hInstance = program;
            kind.lpszClassName = class.as_ptr();
            // Registered already by an earlier notice: that is fine.
            RegisterClassW(&kind);

            let screen = GetDC(null_mut());
            let scale = (GetDeviceCaps(screen, LOGPIXELSY as i32) as f64 / 96.0).clamp(1.0, 4.0);
            ReleaseDC(null_mut(), screen);
            let px = |n: f64| (n * scale).round() as i32;
            let Some(monitor) = monitor_of(GetForegroundWindow()) else { return };
            let (width, height) = (px(300.0), px(60.0));

            let window = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                monitor.right - width - px(24.0),
                monitor.top + px(24.0),
                width,
                height,
                null_mut(),
                null_mut(),
                program,
                null(),
            );
            if window.is_null() {
                crate::warn("zenithd: the notice over the game could not be shown");
                return;
            }
            let notice = Box::into_raw(Box::new(Notice { title, detail, scale }));
            SetWindowLongPtrW(window, GWLP_USERDATA, notice as isize);
            SetLayeredWindowAttributes(window, 0, 238, LWA_ALPHA);
            ShowWindow(window, SW_SHOWNOACTIVATE);
            SetWindowPos(window, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            SetTimer(window, 1, SHOWN_MS, None);

            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            drop(Box::from_raw(notice));
        }
    });
}
