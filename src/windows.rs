//! Win32 backend.
//!
//! The overlay is a single layered, click-through, top-most popup window whose
//! pixels are handed to the Desktop Window Manager once via
//! `UpdateLayeredWindow`. DWM composites it for free on every frame; crosslay
//! itself does no drawing, no polling and no per-frame work. The thread sleeps
//! in `GetMessageW` until a hotkey or a slow housekeeping timer arrives.
//!
//! Note: like every non-injecting overlay this is visible over windowed and
//! borderless-windowed games, not over exclusive fullscreen.

use crate::canvas::Canvas;
use crate::config::{Action, Config};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::time::SystemTime;

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, ReleaseDC,
    SelectObject,
};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_TOPMOST, MB_ICONERROR,
    MB_ICONWARNING, MB_OK, MSG, MessageBoxW, PostMessageW, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetTimer, SetWindowPos, ShowWindow,
    ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_DISPLAYCHANGE, WM_HOTKEY, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// Posted to ourselves when the display layout changes.
const WM_APP_REPOSITION: u32 = WM_APP + 1;
/// Housekeeping timer: re-assert top-most and check the config for edits.
const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 1500;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn show_error(msg: &str) {
    let text = wide(msg);
    let title = wide("crosslay");
    unsafe {
        MessageBoxW(
            null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

/// Release builds use the GUI subsystem; reattach to the launching terminal (if
/// any) so `--help` and friends can print.
pub fn attach_console() {
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_DISPLAYCHANGE {
        unsafe { PostMessageW(hwnd, WM_APP_REPOSITION, 0, 0) };
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

struct App {
    hwnd: HWND,
    path: PathBuf,
    cfg: Config,
    visible: bool,
    bindings: Vec<Action>,
    mtime: Option<SystemTime>,
    /// `profile` as last read from disk (the runtime one changes with hotkeys).
    file_profile: usize,
}

pub fn run(path: PathBuf, cfg: Config) -> Result<(), String> {
    unsafe {
        // Physical pixels everywhere, so a 1px line is really 1px at any scaling.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinstance = GetModuleHandleW(null());
        let class_name = wide("crosslay_overlay");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: class_name.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err("failed to register the overlay window class".into());
        }
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            wide("crosslay").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        );
        if hwnd.is_null() {
            return Err("failed to create the overlay window".into());
        }

        let mut app = App {
            hwnd,
            visible: cfg.visible,
            mtime: modified(&path),
            file_profile: cfg.profile,
            path,
            cfg,
            bindings: Vec::new(),
        };
        app.register_hotkeys()?;
        app.redraw();
        SetTimer(hwnd, TIMER_ID, TIMER_MS, None);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            match msg.message {
                WM_HOTKEY => {
                    let action = app.bindings.get(msg.wParam.wrapping_sub(1)).copied();
                    match action {
                        Some(Action::Quit) => break,
                        Some(a) => app.handle(a),
                        None => {}
                    }
                }
                WM_TIMER if msg.wParam == TIMER_ID => app.tick(),
                WM_APP_REPOSITION => app.redraw(),
                _ => {
                    DispatchMessageW(&msg);
                }
            }
        }
        app.unregister_hotkeys();
        Ok(())
    }
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl App {
    fn handle(&mut self, action: Action) {
        match action {
            Action::Toggle => {
                self.visible = !self.visible;
                self.redraw();
            }
            Action::Next | Action::Previous => {
                let n = self.cfg.crosshairs.len();
                let cur = self.cfg.profile % n;
                self.cfg.profile = if action == Action::Next {
                    (cur + 1) % n
                } else {
                    (cur + n - 1) % n
                };
                self.visible = true;
                self.redraw();
            }
            Action::Reload => {
                if !self.reload() {
                    unsafe { MessageBeep(MB_ICONWARNING) };
                }
            }
            Action::Quit => {}
        }
    }

    fn tick(&mut self) {
        // Games sometimes push themselves above other top-most windows.
        if self.visible {
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
            };
        }
        if self.cfg.live_reload {
            let m = modified(&self.path);
            if m != self.mtime {
                self.mtime = m;
                // A half-written file just fails to parse; the next save retries.
                self.reload();
            }
        }
    }

    /// Re-read the config, keeping the current profile and visibility. Returns
    /// false (and keeps the old config) if the file is invalid.
    fn reload(&mut self) -> bool {
        self.mtime = modified(&self.path);
        let Ok(mut cfg) = Config::load(&self.path) else {
            return false;
        };
        // Keep the profile picked with hotkeys unless the file's `profile` changed.
        let file_profile = cfg.profile;
        if file_profile == self.file_profile {
            cfg.profile = self.cfg.profile;
        }
        let old = std::mem::replace(&mut self.cfg, cfg);
        self.unregister_hotkeys();
        if self.register_hotkeys().is_err() {
            self.unregister_hotkeys();
            self.cfg = old;
            let _ = self.register_hotkeys();
            return false;
        }
        self.file_profile = file_profile;
        self.redraw();
        true
    }

    fn register_hotkeys(&mut self) -> Result<(), String> {
        let bindings = self.cfg.hotkeys.bindings()?;
        for (i, (action, key)) in bindings.iter().enumerate() {
            let ok = unsafe {
                RegisterHotKey(
                    self.hwnd,
                    (i + 1) as i32,
                    key.modifiers | MOD_NOREPEAT,
                    key.vk,
                )
            };
            if ok == 0 {
                self.unregister_hotkeys();
                return Err(format!(
                    "could not register the {action:?} hotkey; another program is probably using it. \
                     Pick a different one in {}",
                    self.path.display()
                ));
            }
            self.bindings.push(*action);
        }
        Ok(())
    }

    fn unregister_hotkeys(&mut self) {
        for i in 0..self.bindings.len() {
            unsafe { UnregisterHotKey(self.hwnd, (i + 1) as i32) };
        }
        self.bindings.clear();
    }

    fn redraw(&mut self) {
        if !self.visible {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            return;
        }
        let canvas = self.cfg.active().render();
        let rect = monitor_rect(self.cfg.monitor);
        let ch = self.cfg.active();
        let cx = rect.left + (rect.right - rect.left) / 2 + ch.offset_x;
        let cy = rect.top + (rect.bottom - rect.top) / 2 + ch.offset_y;
        let origin = POINT {
            x: cx - canvas.anchor.0,
            y: cy - canvas.anchor.1,
        };
        unsafe {
            present(self.hwnd, &canvas, origin);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

/// Upload `canvas` as the window's per-pixel-alpha content at `origin`.
unsafe fn present(hwnd: HWND, canvas: &Canvas, origin: POINT) {
    unsafe {
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: canvas.width as i32,
            biHeight: -(canvas.height as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut core::ffi::c_void = null_mut();
        let bitmap = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bitmap.is_null() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(
                canvas.pixels.as_ptr(),
                bits.cast::<u32>(),
                canvas.pixels.len(),
            );
            let old = SelectObject(mem, bitmap);
            let size = SIZE {
                cx: canvas.width as i32,
                cy: canvas.height as i32,
            };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd, screen, &origin, &size, mem, &src, 0, &blend, ULW_ALPHA,
            );
            SelectObject(mem, old);
            DeleteObject(bitmap);
        }
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
    }
}

/// Bounds of the requested monitor: index 0 is the primary monitor, the rest
/// follow in enumeration order. Falls back to the primary monitor.
fn monitor_rect(index: usize) -> RECT {
    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data as *mut Vec<(bool, RECT)>) };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(m, &mut info) } != 0 {
            list.push((info.dwFlags & 1 != 0, info.rcMonitor)); // MONITORINFOF_PRIMARY
        }
        1
    }
    let mut list: Vec<(bool, RECT)> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(collect),
            &mut list as *mut _ as LPARAM,
        );
    }
    // Stable sort: primary first, others keep system order.
    list.sort_by_key(|(primary, _)| !primary);
    list.get(index)
        .or_else(|| list.first())
        .map(|(_, r)| *r)
        .unwrap_or(RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        })
}
