//! The monitors: their names and sizes for `--list-capture-options`, and
//! what ffmpeg needs to capture the one `-w` names.

use crate::ffmpeg::Grab;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    /// `DISPLAY1`: Windows' device name without the `\\.\` in front.
    pub name: String,
    /// In pixels, as the monitor is turned.
    pub size: (u32, u32),
    /// Where its top left corner is on the desktop, in pixels.
    pub origin: (i32, i32),
    pub primary: bool,
    /// Its place among the outputs of the first graphics adapter, which is
    /// what Desktop Duplication through ffmpeg can reach. `None` for a
    /// monitor plugged into another adapter.
    pub output_idx: Option<u32>,
    /// Windows' handle for it, which Windows.Graphics.Capture takes.
    pub handle: u64,
}

/// The name of the pretend monitor the test picture stands for.
pub const TEST_NAME: &str = "test";

/// Whether the test picture is recorded in place of a screen: asked for,
/// or there is no Windows to capture from.
pub fn test_source() -> bool {
    cfg!(not(windows)) || std::env::var_os("ZENITH_REC_TEST_SOURCE").is_some()
}

/// The monitors, the primary one first.
pub fn list() -> Vec<Monitor> {
    if test_source() {
        return vec![Monitor { name: TEST_NAME.into(), size: (1280, 720), origin: (0, 0), primary: true, output_idx: None, handle: 0 }];
    }
    #[cfg(windows)]
    {
        let mut found = windows_impl::enumerate();
        found.sort_by_key(|m| (!m.primary, display_number(&m.name), m.name.clone()));
        found
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// The number at the end of `DISPLAY12`, so monitors sort as people count.
pub fn display_number(name: &str) -> u32 {
    let digits = name.trim_end_matches(|c: char| !c.is_ascii_digit());
    let start = digits.rfind(|c: char| !c.is_ascii_digit()).map_or(0, |at| at + 1);
    digits[start..].parse().unwrap_or(u32::MAX)
}

/// Windows' device name without its `\\.\` prefix.
pub fn short_name(device: &str) -> &str {
    device.strip_prefix(r"\\.\").unwrap_or(device)
}

/// The monitor `-w` asks for: `screen` is the largest, the first of them
/// when several are as large (the same choice the daemon makes from the
/// same list); anything else is a monitor's name.
pub fn pick<'a>(window: &str, monitors: &'a [Monitor]) -> Option<&'a Monitor> {
    if window == "screen" {
        // `max_by_key` keeps the last of equals; looked through backwards, that is the first.
        return monitors.iter().rev().max_by_key(|m| m.size.0 as u64 * m.size.1 as u64);
    }
    let wanted = short_name(window);
    monitors.iter().find(|m| m.name.eq_ignore_ascii_case(wanted))
}

/// The ways ffmpeg can capture `monitor`, of those this ffmpeg has, the
/// best first. `ZENITH_REC_CAPTURE=ddagrab|gfxcapture|gdigrab` leaves only
/// the one named.
pub fn grabs(monitor: &Monitor, filters: &std::collections::HashSet<String>, only: Option<&str>) -> Vec<Grab> {
    let mut out = Vec::new();
    if let Some(output_idx) = monitor.output_idx
        && filters.contains("ddagrab")
    {
        out.push(Grab::Ddagrab { output_idx });
    }
    if filters.contains("gfxcapture") {
        out.push(Grab::Gfxcapture { hmonitor: monitor.handle });
    }
    // Last: it works where there is no graphics card to ask (a virtual
    // machine, a remote session), at the price of speed.
    out.push(Grab::Gdigrab { origin: monitor.origin, size: monitor.size });
    if let Some(only) = only {
        out.retain(|grab| grab.name() == only);
    }
    out
}

#[cfg(windows)]
mod windows_impl {
    use super::{Monitor, short_name};
    use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    use windows::Win32::Graphics::Gdi::{
        DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };
    use windows::core::{BOOL, PCWSTR};

    /// `MONITORINFOF_PRIMARY` in `MONITORINFO::dwFlags`.
    const PRIMARY: u32 = 1;

    fn text(wide: &[u16]) -> String {
        let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        String::from_utf16_lossy(&wide[..end])
    }

    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> BOOL {
        // SAFETY: `list` is the `Vec` `handles` passed in, alive for the
        // whole of `EnumDisplayMonitors`, which calls this on its thread.
        let list = unsafe { &mut *(list.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        TRUE
    }

    fn handles() -> Vec<HMONITOR> {
        let mut list: Vec<HMONITOR> = Vec::new();
        // SAFETY: the callback only touches `list`, which outlives the call.
        let _ = unsafe { EnumDisplayMonitors(None, None, Some(collect), LPARAM(&raw mut list as isize)) };
        list
    }

    /// The device names of the first adapter's outputs, in output order:
    /// the adapter ffmpeg's `ddagrab` opens when it is not told another.
    fn first_adapter_outputs() -> Vec<String> {
        let mut names = Vec::new();
        // SAFETY: plain COM calls; every interface is released when dropped.
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return names };
            let Ok(adapter) = factory.EnumAdapters1(0) else { return names };
            // The list ends at the first index that is not an output.
            while let Ok(output) = adapter.EnumOutputs(names.len() as u32) {
                names.push(output.GetDesc().map(|desc| text(&desc.DeviceName)).unwrap_or_default());
            }
        }
        names
    }

    fn describe(handle: HMONITOR, outputs: &[String]) -> Option<Monitor> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` is a MONITORINFOEXW with its size set, which is
        // what GetMonitorInfoW fills when given that size.
        if !unsafe { GetMonitorInfoW(handle, (&raw mut info).cast::<MONITORINFO>()) }.as_bool() {
            return None;
        }
        let device = text(&info.szDevice);
        let area = info.monitorInfo.rcMonitor;
        // The mode's size is in real pixels whatever the display scaling;
        // the rectangle is only that for a DPI-aware process.
        let mut mode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        // SAFETY: `szDevice` is NUL-terminated and `mode` has its size set.
        let known = unsafe { EnumDisplaySettingsW(PCWSTR(info.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &raw mut mode) }.as_bool();
        let (size, origin) = if known && mode.dmPelsWidth > 0 && mode.dmPelsHeight > 0 {
            // SAFETY: for a display, this part of the mode holds its position.
            let at = unsafe { mode.Anonymous1.Anonymous2.dmPosition };
            ((mode.dmPelsWidth, mode.dmPelsHeight), (at.x, at.y))
        } else {
            (((area.right - area.left).max(0) as u32, (area.bottom - area.top).max(0) as u32), (area.left, area.top))
        };
        Some(Monitor {
            name: short_name(&device).to_string(),
            size,
            origin,
            primary: info.monitorInfo.dwFlags & PRIMARY != 0,
            output_idx: outputs.iter().position(|name| *name == device).map(|at| at as u32),
            handle: handle.0 as usize as u64,
        })
    }

    pub fn enumerate() -> Vec<Monitor> {
        let outputs = first_adapter_outputs();
        handles().into_iter().filter_map(|handle| describe(handle, &outputs)).filter(|m| m.size.0 > 0 && m.size.1 > 0).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn monitor(name: &str, w: u32, h: u32, output_idx: Option<u32>) -> Monitor {
        Monitor { name: name.into(), size: (w, h), origin: (0, 0), primary: false, output_idx, handle: 65537 }
    }

    #[test]
    fn screen_is_the_largest_and_the_first_of_equals() {
        let list = [monitor("DISPLAY1", 1920, 1080, Some(0)), monitor("DISPLAY2", 2560, 1440, Some(1)), monitor("DISPLAY3", 2560, 1440, None)];
        assert_eq!(pick("screen", &list).unwrap().name, "DISPLAY2");
        assert_eq!(pick("screen", &list[..1]).unwrap().name, "DISPLAY1");
        assert_eq!(pick("screen", &[]), None);
    }

    #[test]
    fn a_monitor_is_found_by_its_name_in_either_spelling() {
        let list = [monitor("DISPLAY1", 1920, 1080, Some(0)), monitor("DISPLAY2", 2560, 1440, Some(1))];
        assert_eq!(pick("DISPLAY1", &list).unwrap().size, (1920, 1080));
        assert_eq!(pick(r"\\.\DISPLAY2", &list).unwrap().size, (2560, 1440));
        assert_eq!(pick("display2", &list).unwrap().size, (2560, 1440));
        assert_eq!(pick("DISPLAY7", &list), None);
    }

    #[test]
    fn names_sort_as_people_count() {
        assert_eq!(short_name(r"\\.\DISPLAY10"), "DISPLAY10");
        assert_eq!(short_name("DISPLAY1"), "DISPLAY1");
        assert!(display_number("DISPLAY2") < display_number("DISPLAY10"));
        assert_eq!(display_number("DISPLAY12"), 12);
        assert_eq!(display_number("test"), u32::MAX);
    }

    #[test]
    fn a_monitor_is_captured_by_what_this_ffmpeg_has() {
        let both: HashSet<String> = ["ddagrab", "gfxcapture"].into_iter().map(String::from).collect();
        let dda_only: HashSet<String> = ["ddagrab".to_string()].into();
        let first = monitor("DISPLAY1", 1920, 1080, Some(2));
        // Copying the desktop's pixels is always there, and always last.
        let pixels = Grab::Gdigrab { origin: (0, 0), size: (1920, 1080) };
        assert_eq!(grabs(&first, &both, None), [Grab::Ddagrab { output_idx: 2 }, Grab::Gfxcapture { hmonitor: 65537 }, pixels.clone()]);
        assert_eq!(grabs(&first, &dda_only, None), [Grab::Ddagrab { output_idx: 2 }, pixels.clone()]);
        assert_eq!(grabs(&first, &both, Some("gfxcapture")), [Grab::Gfxcapture { hmonitor: 65537 }]);
        assert_eq!(grabs(&first, &both, Some("gdigrab")), std::slice::from_ref(&pixels));
        // On another adapter: Desktop Duplication through ffmpeg cannot reach it.
        let other = monitor("DISPLAY3", 1920, 1080, None);
        assert_eq!(grabs(&other, &both, None), [Grab::Gfxcapture { hmonitor: 65537 }, pixels.clone()]);
        assert_eq!(grabs(&other, &dda_only, None), [pixels]);
    }

    #[test]
    fn without_a_screen_there_is_the_test_monitor() {
        if test_source() {
            let list = list();
            assert_eq!(list.len(), 1);
            assert_eq!((list[0].name.as_str(), list[0].size), (TEST_NAME, (1280, 720)));
        }
    }
}
