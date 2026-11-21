//! The `overlay.bin` file: the only thing the crosslay overlay ever reads.
//!
//! The editor "bakes" every crosshair profile into ready-to-show pixels plus a
//! handful of settings. The overlay copies the active frame straight into a
//! Windows DIB section, so it needs no config parser, image decoder or
//! rasteriser of its own.
//!
//! Layout (version 1, all integers little-endian):
//!
//! ```text
//! 0   [u8; 4]  magic "CLAY"
//! 4   u16      version (1)
//! 6   u16      flags (bit 0 = start visible, bit 1 = auto update)
//! 8   u16      monitor (0 = primary)
//! 10  u16      startup profile
//! 12  5 x (u16 modifiers, u16 vk)   hotkeys, see [`Action`]; vk 0 = disabled
//! 32  u16      frame count
//! 34  u16      reserved (0)
//! 36  frames:  i32 offset_x, i32 offset_y, u16 width, u16 height,
//!              u16 anchor_x, u16 anchor_y, then width*height*4 bytes of
//!              premultiplied BGRA (top-down rows)
//! ```
//!
//! This crate has no dependencies and does no allocation when parsing, so it
//! adds almost nothing to the overlay.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

pub const MAGIC: [u8; 4] = *b"CLAY";
pub const VERSION: u16 = 1;
/// Largest frame side the overlay accepts.
pub const MAX_SIDE: u16 = 4096;
/// Largest number of frames (profiles) in one file.
pub const MAX_FRAMES: u16 = 256;

/// Name of the baked file inside the data folder.
pub const BAKED_FILE: &str = "overlay.bin";
/// Name of the editable config inside the data folder.
pub const CONFIG_FILE: &str = "config.toml";

const HEADER_LEN: usize = 36;
const FRAME_HEADER_LEN: usize = 16;

/// What each hotkey slot does, in file order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Action {
    Toggle = 0,
    Editor = 1,
    Next = 2,
    Previous = 3,
    Quit = 4,
}

impl Action {
    pub const ALL: [Action; 5] = [
        Action::Toggle,
        Action::Editor,
        Action::Next,
        Action::Previous,
        Action::Quit,
    ];
}

/// A global hotkey: Win32 `MOD_*` flags and a virtual-key code. `vk == 0`
/// means the slot is disabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u16,
    pub vk: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub visible: bool,
    /// Start `crosslay-editor --update` when the overlay starts.
    pub auto_update: bool,
    pub monitor: u16,
    pub profile: u16,
    /// Indexed by `Action as usize`.
    pub hotkeys: [Hotkey; 5],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visible: true,
            auto_update: true,
            monitor: 0,
            profile: 0,
            hotkeys: [Hotkey::default(); 5],
        }
    }
}

/// One baked crosshair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    /// Shift from the monitor centre, in physical pixels.
    pub offset: (i32, i32),
    pub width: u16,
    pub height: u16,
    /// The pixel that sits on the monitor centre (plus `offset`).
    pub anchor: (u16, u16),
    /// `width * height * 4` bytes of premultiplied BGRA, top-down.
    pub pixels: &'a [u8],
}

/// A parsed `overlay.bin`, borrowing from the file's bytes.
#[derive(Clone, Debug)]
pub struct Baked<'a> {
    pub settings: Settings,
    frame_count: u16,
    frames: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BadMagic,
    UnsupportedVersion(u16),
    Truncated,
    BadFrame,
    Empty,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadMagic => f.write_str("not a crosslay overlay file"),
            Error::UnsupportedVersion(v) => write!(f, "unsupported overlay file version {v}"),
            Error::Truncated => f.write_str("overlay file is truncated"),
            Error::BadFrame => f.write_str("overlay file has an invalid frame"),
            Error::Empty => f.write_str("overlay file has no crosshairs"),
        }
    }
}

impl std::error::Error for Error {}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl<'a> Baked<'a> {
    /// Parse and fully validate a baked file. Every frame is checked here, so
    /// [`Baked::frame`] can't fail later.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_LEN {
            return Err(if bytes.starts_with(&MAGIC) || bytes.len() < 4 {
                Error::Truncated
            } else {
                Error::BadMagic
            });
        }
        if bytes[..4] != MAGIC {
            return Err(Error::BadMagic);
        }
        let version = u16_at(bytes, 4);
        if version != VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let mut hotkeys = [Hotkey::default(); 5];
        for (i, hk) in hotkeys.iter_mut().enumerate() {
            let at = 12 + i * 4;
            *hk = Hotkey {
                modifiers: u16_at(bytes, at),
                vk: u16_at(bytes, at + 2),
            };
        }
        let settings = Settings {
            visible: u16_at(bytes, 6) & 1 != 0,
            auto_update: u16_at(bytes, 6) & 2 != 0,
            monitor: u16_at(bytes, 8),
            profile: u16_at(bytes, 10),
            hotkeys,
        };
        let frame_count = u16_at(bytes, 32);
        if frame_count == 0 {
            return Err(Error::Empty);
        }
        if frame_count > MAX_FRAMES {
            return Err(Error::BadFrame);
        }
        let frames = &bytes[HEADER_LEN..];
        let baked = Baked {
            settings,
            frame_count,
            frames,
        };
        // Walk all frames once to validate sizes.
        let mut rest = frames;
        for _ in 0..frame_count {
            let (_, next) = split_frame(rest)?;
            rest = next;
        }
        if !rest.is_empty() {
            return Err(Error::BadFrame);
        }
        Ok(baked)
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count as usize
    }

    /// Frame `index`, wrapping around the frame count.
    pub fn frame(&self, index: usize) -> Frame<'a> {
        let mut rest = self.frames;
        for _ in 0..index % self.frame_count() {
            rest = split_frame(rest).expect("validated in parse").1;
        }
        split_frame(rest).expect("validated in parse").0
    }
}

fn split_frame(b: &[u8]) -> Result<(Frame<'_>, &[u8]), Error> {
    if b.len() < FRAME_HEADER_LEN {
        return Err(Error::Truncated);
    }
    let width = u16_at(b, 8);
    let height = u16_at(b, 10);
    let anchor = (u16_at(b, 12), u16_at(b, 14));
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || anchor.0 >= width
        || anchor.1 >= height
    {
        return Err(Error::BadFrame);
    }
    let len = width as usize * height as usize * 4;
    let body = &b[FRAME_HEADER_LEN..];
    if body.len() < len {
        return Err(Error::Truncated);
    }
    let frame = Frame {
        offset: (i32_at(b, 0), i32_at(b, 4)),
        width,
        height,
        anchor,
        pixels: &body[..len],
    };
    Ok((frame, &body[len..]))
}

/// Serialise settings and frames. Panics if a frame's pixel buffer doesn't
/// match its size or a limit is exceeded; callers build frames themselves.
pub fn encode(settings: &Settings, frames: &[Frame<'_>]) -> Vec<u8> {
    assert!(!frames.is_empty() && frames.len() <= MAX_FRAMES as usize);
    let body: usize = frames
        .iter()
        .map(|f| FRAME_HEADER_LEN + f.pixels.len())
        .sum();
    let mut out = Vec::with_capacity(HEADER_LEN + body);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    let flags = settings.visible as u16 | (settings.auto_update as u16) << 1;
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&settings.monitor.to_le_bytes());
    out.extend_from_slice(&settings.profile.to_le_bytes());
    for hk in settings.hotkeys {
        out.extend_from_slice(&hk.modifiers.to_le_bytes());
        out.extend_from_slice(&hk.vk.to_le_bytes());
    }
    out.extend_from_slice(&(frames.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    for f in frames {
        assert!(f.width > 0 && f.height > 0 && f.width <= MAX_SIDE && f.height <= MAX_SIDE);
        assert!(f.anchor.0 < f.width && f.anchor.1 < f.height);
        assert_eq!(f.pixels.len(), f.width as usize * f.height as usize * 4);
        out.extend_from_slice(&f.offset.0.to_le_bytes());
        out.extend_from_slice(&f.offset.1.to_le_bytes());
        out.extend_from_slice(&f.width.to_le_bytes());
        out.extend_from_slice(&f.height.to_le_bytes());
        out.extend_from_slice(&f.anchor.0.to_le_bytes());
        out.extend_from_slice(&f.anchor.1.to_le_bytes());
        out.extend_from_slice(f.pixels);
    }
    out
}

/// The folder both apps keep their files in.
///
/// If a `config.toml` sits next to the running executable, that folder is
/// used (portable mode). Otherwise it's `%APPDATA%\crosslay` on Windows and
/// `$XDG_CONFIG_HOME/crosslay` (or `~/.config/crosslay`) elsewhere.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .filter(|d| d.join(CONFIG_FILE).is_file())
    {
        return dir;
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("crosslay")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let a = [1u8; 3 * 2 * 4];
        let b = [2u8; 4];
        let settings = Settings {
            visible: false,
            auto_update: true,
            monitor: 2,
            profile: 1,
            hotkeys: [
                Hotkey {
                    modifiers: 3,
                    vk: 0x58,
                },
                Hotkey::default(),
                Hotkey {
                    modifiers: 3,
                    vk: 0x27,
                },
                Hotkey {
                    modifiers: 3,
                    vk: 0x25,
                },
                Hotkey {
                    modifiers: 3,
                    vk: 0x51,
                },
            ],
        };
        encode(
            &settings,
            &[
                Frame {
                    offset: (-5, 7),
                    width: 3,
                    height: 2,
                    anchor: (1, 1),
                    pixels: &a,
                },
                Frame {
                    offset: (0, 0),
                    width: 1,
                    height: 1,
                    anchor: (0, 0),
                    pixels: &b,
                },
            ],
        )
    }

    #[test]
    fn round_trips() {
        let bytes = sample();
        let baked = Baked::parse(&bytes).unwrap();
        assert!(!baked.settings.visible);
        assert!(baked.settings.auto_update);
        assert_eq!(baked.settings.monitor, 2);
        assert_eq!(baked.settings.profile, 1);
        assert_eq!(baked.settings.hotkeys[Action::Quit as usize].vk, 0x51);
        assert_eq!(baked.settings.hotkeys[Action::Editor as usize].vk, 0);
        assert_eq!(baked.frame_count(), 2);
        let f = baked.frame(0);
        assert_eq!(
            (f.offset, f.width, f.height, f.anchor),
            ((-5, 7), 3, 2, (1, 1))
        );
        assert_eq!(f.pixels, &[1u8; 24]);
        assert_eq!(baked.frame(1).pixels, &[2u8; 4]);
        assert_eq!(baked.frame(2), baked.frame(0));
    }

    #[test]
    fn rejects_damage() {
        let bytes = sample();
        for len in 0..bytes.len() {
            assert!(Baked::parse(&bytes[..len]).is_err(), "accepted {len} bytes");
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(Baked::parse(&extra).unwrap_err(), Error::BadFrame);
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(Baked::parse(&magic).unwrap_err(), Error::BadMagic);
        let mut version = bytes.clone();
        version[4] = 9;
        assert_eq!(
            Baked::parse(&version).unwrap_err(),
            Error::UnsupportedVersion(9)
        );
        let mut anchor = bytes;
        anchor[36 + 12] = 3; // anchor_x == width
        assert_eq!(Baked::parse(&anchor).unwrap_err(), Error::BadFrame);
    }
}
