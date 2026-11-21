//! crosslay: the crosshair overlay.
//!
//! Reads the baked `overlay.bin` written by `crosslay-editor`, shows one
//! frame of it in a click-through, top-most layered window, and sleeps until
//! a hotkey, a file change or a display change wakes it.

// No console window in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod fallback;
#[cfg(windows)]
mod windows;

use std::path::PathBuf;

/// `crosslay [--dir <folder>]`
fn data_dir_from_args() -> Result<PathBuf, String> {
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next(), args.next()) {
        (None, _, _) => Ok(crosslay_format::data_dir()),
        (Some(flag), Some(dir), None) if flag == "--dir" => Ok(dir.into()),
        _ => Err("usage: crosslay [--dir <folder>]".into()),
    }
}

#[cfg(windows)]
fn main() {
    let result = data_dir_from_args().and_then(windows::run);
    if let Err(msg) = result {
        windows::show_error(&msg);
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    let _ = data_dir_from_args();
    eprintln!(
        "crosslay: the overlay currently runs on Windows only. \
         crosslay-editor works here for designing crosshairs."
    );
    std::process::exit(1);
}
