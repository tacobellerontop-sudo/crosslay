//! Win32 overlay.
//!
//! One layered, click-through, top-most popup window. Its pixels go to the
//! Desktop Window Manager once through `UpdateLayeredWindow`, and DWM
//! composites them every frame at no cost to us. The thread then blocks in
//! `MsgWaitForMultipleObjects` on two things only:
//!
//! - its message queue (hotkeys, display changes, foreground changes), and
//! - a change notification on the data folder (the editor saved).
//!
//! There are no timers, so an idle overlay never wakes up.
//!
//! Like every overlay that doesn't inject into games, it shows over windowed
//! and borderless-windowed games, not exclusive fullscreen.

use crate::fallback;
use crosslay_format::{Action, BAKED_FILE, Baked, Frame, Hotkey, Settings};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::SystemTime;

use windows_sys::Win32::Foundation::{
    BOOL, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT,
    POINT, RECT, SIZE, WAIT_OBJECT_0, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, ReleaseDC,
    SelectObject,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FindFirstChangeNotificationW,
    FindNextChangeNotification,
};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, INFINITE, SetProcessWorkingSetSize,
};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_SYSTEM_FOREGROUND, HWND_TOPMOST,
    MB_ICONERROR, MB_ICONWARNING, MB_OK, MSG, MessageBoxW, MsgWaitForMultipleObjects, PM_REMOVE,
    PeekMessageW, PostMessageW, QS_ALLINPUT, RegisterClassExW, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_DISPLAYCHANGE,
    WM_HOTKEY, WM_QUIT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// Posted to ourselves when the display layout changes.
const WM_APP_REPOSITION: u32 = WM_APP + 1;
const CLASS_NAME: &str = "crosslay_overlay";
/// Held while an overlay runs, so a second one just exits.
const MUTEX_NAME: &str = "Local\\crosslay-overlay";
const EDITOR_EXE: &str = "crosslay-editor.exe";

/// Read by the foreground-change callback, which gets no user data.
static OVERLAY: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static SHOWN: AtomicBool = AtomicBool::new(false);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn show_error(msg: &str) {
    message_box(msg, MB_ICONERROR);
}

fn message_box(msg: &str, icon: u32) {
    let text = wide(msg);
    let title = wide("crosslay");
    unsafe { MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | icon) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_DISPLAYCHANGE {
        unsafe { PostMessageW(hwnd, WM_APP_REPOSITION, 0, 0) };
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A game that takes the foreground can push itself above other top-most
/// windows. Re-assert top-most exactly then, instead of on a timer.
unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if SHOWN.load(Ordering::Relaxed) {
        bring_to_top(OVERLAY.load(Ordering::Relaxed));
    }
}

fn bring_to_top(hwnd: HWND) {
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

pub fn run(dir: PathBuf) -> Result<(), String> {
    unsafe {
        let name = wide(MUTEX_NAME);
        let mutex = CreateMutexW(null(), 1, name.as_ptr());
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(()); // already running
        }

        // Physical pixels everywhere, so a 1 px line is 1 px at any scaling.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinstance = GetModuleHandleW(null());
        let class_name = wide(CLASS_NAME);
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
        let title = wide("crosslay");
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            title.as_ptr(),
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
        OVERLAY.store(hwnd, Ordering::Relaxed);

        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            null_mut(),
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );

        // Watch the folder (the file is replaced by rename, so watch names too).
        let _ = std::fs::create_dir_all(&dir);
        let watch: HANDLE = FindFirstChangeNotificationW(
            wide_path(&dir).as_ptr(),
            0,
            FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
        );
        let watching = watch != INVALID_HANDLE_VALUE && !watch.is_null();

        let mut app = App {
            hwnd,
            file: dir.join(BAKED_FILE),
            dir,
            settings: Settings::default(),
            hotkeys: [Hotkey::default(); 5],
            stamp: None,
            profile: 0,
            visible: true,
        };
        let startup = app.read();
        if let Some(baked) = &startup {
            app.settings = baked.settings;
            app.profile = baked.settings.profile as usize;
            app.visible = baked.settings.visible;
        }
        drop(startup);
        let failed = app.register_hotkeys();
        app.redraw();
        if !failed.is_empty() {
            message_box(
                &format!(
                    "These hotkeys are already used by another program and won't work: {}.\n\n\
                     Pick different ones in crosslay-editor.",
                    failed.join(", ")
                ),
                MB_ICONWARNING,
            );
        }

        let mut msg: MSG = std::mem::zeroed();
        loop {
            let r = MsgWaitForMultipleObjects(watching as u32, &watch, 0, INFINITE, QS_ALLINPUT);
            if watching && r == WAIT_OBJECT_0 {
                FindNextChangeNotification(watch);
                app.file_changed();
                continue;
            }
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                match msg.message {
                    WM_QUIT => return Ok(()),
                    WM_HOTKEY => match Action::ALL.get(msg.wParam.wrapping_sub(1)) {
                        Some(Action::Quit) => {
                            app.unregister_hotkeys();
                            return Ok(());
                        }
                        Some(&a) => app.handle(a),
                        None => {}
                    },
                    WM_APP_REPOSITION => app.redraw(),
                    _ => {
                        DispatchMessageW(&msg);
                    }
                }
            }
        }
    }
}

struct App {
    hwnd: HWND,
    dir: PathBuf,
    file: PathBuf,
    /// Settings from the last valid file.
    settings: Settings,
    /// Hotkeys currently registered with Windows.
    hotkeys: [Hotkey; 5],
    /// Modification time and size of the file as last read.
    stamp: Option<(SystemTime, u64)>,
    /// Profile shown now (changes with the next/previous hotkeys).
    profile: usize,
    visible: bool,
}

/// An `overlay.bin` read from disk. Only lives while drawing.
struct Loaded {
    bytes: Vec<u8>,
    settings: Settings,
}

impl Loaded {
    fn baked(&self) -> Baked<'_> {
        Baked::parse(&self.bytes).expect("validated on load")
    }
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

impl App {
    /// Read and validate the baked file. `None` if it's missing or invalid.
    fn read(&mut self) -> Option<Loaded> {
        self.stamp = stamp(&self.file);
        let bytes = std::fs::read(&self.file).ok()?;
        let settings = Baked::parse(&bytes).ok()?.settings;
        Some(Loaded { bytes, settings })
    }

    /// The folder changed. Reload if `overlay.bin` itself did.
    fn file_changed(&mut self) {
        if stamp(&self.file) == self.stamp {
            return;
        }
        // A file that fails to parse is ignored: the window keeps showing the
        // last good crosshair (DWM still has its pixels).
        let Some(loaded) = self.read() else { return };
        let old = std::mem::replace(&mut self.settings, loaded.settings);
        // The editor writes the selected crosshair as `profile`. Follow it when
        // it changes; otherwise keep what was picked with the hotkeys.
        if loaded.settings.profile != old.profile {
            self.profile = loaded.settings.profile as usize;
        }
        if loaded.settings.hotkeys != self.hotkeys && !self.register_hotkeys().is_empty() {
            unsafe { MessageBeep(MB_ICONWARNING) };
        }
        self.draw(Some(&loaded));
    }

    fn handle(&mut self, action: Action) {
        match action {
            Action::Toggle => {
                self.visible = !self.visible;
                self.redraw();
            }
            Action::Next | Action::Previous => {
                self.profile = if action == Action::Next {
                    self.profile.wrapping_add(1)
                } else {
                    self.profile.wrapping_sub(1)
                };
                self.visible = true;
                self.redraw();
            }
            Action::Editor => self.open_editor(),
            Action::Quit => {}
        }
    }

    fn open_editor(&self) {
        let ok = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join(EDITOR_EXE)))
            .and_then(|editor| {
                std::process::Command::new(editor)
                    .arg("--dir")
                    .arg(&self.dir)
                    .spawn()
                    .ok()
            })
            .is_some();
        if !ok {
            unsafe { MessageBeep(MB_ICONWARNING) };
        }
    }

    /// Register the hotkeys from `settings`. Returns the ones that failed.
    fn register_hotkeys(&mut self) -> Vec<&'static str> {
        self.unregister_hotkeys();
        let mut failed = Vec::new();
        for action in Action::ALL {
            let hk = self.settings.hotkeys[action as usize];
            if hk.vk == 0 {
                continue;
            }
            let ok = unsafe {
                RegisterHotKey(
                    self.hwnd,
                    action as i32 + 1,
                    hk.modifiers as u32 | MOD_NOREPEAT,
                    hk.vk as u32,
                )
            };
            if ok == 0 {
                failed.push(match action {
                    Action::Toggle => "show/hide",
                    Action::Editor => "open editor",
                    Action::Next => "next crosshair",
                    Action::Previous => "previous crosshair",
                    Action::Quit => "quit",
                });
            } else {
                self.hotkeys[action as usize] = hk;
            }
        }
        failed
    }

    fn unregister_hotkeys(&mut self) {
        for action in Action::ALL {
            if self.hotkeys[action as usize].vk != 0 {
                unsafe { UnregisterHotKey(self.hwnd, action as i32 + 1) };
            }
        }
        self.hotkeys = [Hotkey::default(); 5];
    }

    /// Redraw from the file on disk (or the built-in crosshair).
    fn redraw(&mut self) {
        if !self.visible {
            self.draw(None);
            return;
        }
        let loaded = self.read();
        self.draw(loaded.as_ref());
    }

    fn draw(&mut self, loaded: Option<&Loaded>) {
        if !self.visible {
            SHOWN.store(false, Ordering::Relaxed);
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            return;
        }
        let fallback_pixels;
        let baked;
        let frame: Frame<'_> = match loaded {
            Some(l) => {
                baked = l.baked();
                self.profile %= baked.frame_count();
                baked.frame(self.profile)
            }
            None => {
                fallback_pixels = fallback::pixels();
                fallback::frame(&fallback_pixels)
            }
        };
        let rect = monitor_rect(self.settings.monitor as usize);
        let cx = rect.left + (rect.right - rect.left) / 2 + frame.offset.0;
        let cy = rect.top + (rect.bottom - rect.top) / 2 + frame.offset.1;
        let origin = POINT {
            x: cx - frame.anchor.0 as i32,
            y: cy - frame.anchor.1 as i32,
        };
        unsafe {
            present(self.hwnd, &frame, origin);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
        bring_to_top(self.hwnd);
        SHOWN.store(true, Ordering::Relaxed);
        // Everything above was temporary. Hand the pages back so the idle
        // footprint is what's really in use.
        unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
    }
}

/// Upload `frame` as the window's per-pixel-alpha content at `origin`.
unsafe fn present(hwnd: HWND, frame: &Frame<'_>, origin: POINT) {
    unsafe {
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: frame.width as i32,
            biHeight: -(frame.height as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bitmap.is_null() && !bits.is_null() {
            // The file already holds premultiplied BGRA rows: a straight copy.
            std::ptr::copy_nonoverlapping(
                frame.pixels.as_ptr(),
                bits.cast::<u8>(),
                frame.pixels.len(),
            );
            let old = SelectObject(mem, bitmap);
            let size = SIZE {
                cx: frame.width as i32,
                cy: frame.height as i32,
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

/// Bounds of the requested monitor: 0 is the primary monitor, the rest follow
/// in system order. Falls back to the primary monitor.
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
