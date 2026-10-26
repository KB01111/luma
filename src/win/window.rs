use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::{
        Dwm::{
            DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
            DWMWCP_DONOTROUND, DwmSetWindowAttribute,
        },
        Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    },
    UI::WindowsAndMessaging::{
        BringWindowToTop, GetCursorPos, GetForegroundWindow, GetWindowRect, HWND_TOPMOST, IsIconic,
        IsWindowVisible, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        SetForegroundWindow, SetWindowPos, ShowWindow,
    },
};

fn to_hwnd(value: isize) -> HWND {
    HWND(value as *mut c_void)
}

pub fn hwnd_of(window: &impl HasWindowHandle) -> Option<isize> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
        _ => None,
    }
}

pub fn hide(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(to_hwnd(hwnd), SW_HIDE);
    }
}

pub fn show(hwnd: isize) {
    polish(hwnd);
    unsafe {
        let _ = ShowWindow(to_hwnd(hwnd), SW_SHOW);
    }
    set_topmost(hwnd);
}

/// Removes the Windows accent border and requests a square DWM frame.
/// The visible rounded shape is drawn by Luma itself.
pub fn polish(hwnd: isize) {
    let window = to_hwnd(hwnd);
    let dark: i32 = 1;
    let border_color: u32 = 0xFFFF_FFFE;
    let corner = DWMWCP_DONOTROUND;

    unsafe {
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const c_void,
            std::mem::size_of_val(&corner) as u32,
        );
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_BORDER_COLOR,
            &border_color as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

pub fn is_visible(hwnd: isize) -> bool {
    let window = to_hwnd(hwnd);
    unsafe { IsWindowVisible(window).as_bool() && !IsIconic(window).as_bool() }
}

pub fn is_foreground(hwnd: isize) -> bool {
    unsafe { GetForegroundWindow() == to_hwnd(hwnd) }
}

pub fn set_topmost(hwnd: isize) {
    unsafe {
        let _ = SetWindowPos(
            to_hwnd(hwnd),
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn position_top_center_on_cursor_monitor(hwnd: isize) {
    let window = to_hwnd(hwnd);

    unsafe {
        let mut cursor = POINT::default();
        if GetCursorPos(&mut cursor).is_err() {
            return;
        }

        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut monitor_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
            return;
        }

        let work = monitor_info.rcWork;
        let mut window_rect = RECT::default();
        if GetWindowRect(window, &mut window_rect).is_err() {
            return;
        }

        let width = window_rect.right.saturating_sub(window_rect.left);
        let height = window_rect.bottom.saturating_sub(window_rect.top);
        let work_width = work.right.saturating_sub(work.left);
        let work_height = work.bottom.saturating_sub(work.top);

        let x = work
            .left
            .saturating_add(work_width.saturating_sub(width) / 2);
        let y = work
            .top
            .saturating_add(work_height.saturating_mul(14) / 100);
        let max_x = work.right.saturating_sub(width).max(work.left);
        let max_y = work.bottom.saturating_sub(height).max(work.top);
        let x = x.clamp(work.left, max_x);
        let y = y.clamp(work.top, max_y);

        let _ = SetWindowPos(
            window,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }

    set_topmost(hwnd);
}

pub fn show_and_activate(hwnd: isize) {
    show(hwnd);
    unsafe {
        let _ = SetForegroundWindow(to_hwnd(hwnd));
        let _ = BringWindowToTop(to_hwnd(hwnd));
    }
}
