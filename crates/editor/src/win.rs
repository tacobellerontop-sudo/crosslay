//! The few Win32 calls the editor needs.

use std::ptr::null_mut;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

/// Must match the overlay's single-instance mutex.
const OVERLAY_MUTEX: &str = "Local\\crosslay-overlay";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn show_error(msg: &str) {
    let text = wide(msg);
    let title = wide("crosslay-editor");
    unsafe {
        MessageBoxW(
            null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

/// Release builds use the GUI subsystem; reattach to the launching terminal
/// (if any) so `--help` and friends can print.
pub fn attach_console() {
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// Whether the overlay is running.
pub fn overlay_running() -> bool {
    let name = wide(OVERLAY_MUTEX);
    let h = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if h.is_null() {
        return false;
    }
    unsafe { CloseHandle(h) };
    true
}
