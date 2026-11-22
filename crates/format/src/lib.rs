//! The `overlay.bin` file: the only thing the crosslay overlay ever reads.
//!
//! The editor "bakes" every crosshair profile into ready-to-show pixels plus a
//! handful of settings. The overlay copies the active frame straight into a
//! Windows DIB section, so it needs no config parser, image decoder or
//! rasteriser of its own.
//!
//! Layout (version 2, all integers little-endian):
//!
//! ```text
//! 0   [u8; 4]  magic "CLAY"
//! 4   u16      version (2)
//! 6   u16      flags (bit 0 = start visible, bit 1 = auto update)
//! 8   u16      monitor (0 = primary)
//! 10  u16      startup profile
//! 12  5 x (u16 modifiers, u16 vk)   hotkeys, see [`Action`]; vk 0 = disabled
//! 32  u16      profile count
//! 34  u16      right-click mode (0 = while held, 1 = toggle on click)
//! 36  profiles: a frame, then its right-click frame if the frame's flags say so
//!
//! frame: i32 offset_x, i32 offset_y, u16 width, u16 height,
//!        u16 anchor_x, u16 anchor_y, u16 flags (bit 0 = a right-click
//!        frame follows; always 0 on that frame), u16 reserved (0),
//!        then width*height*4 bytes of premultiplied BGRA (top-down rows)
//! ```
//!
//! Version 1 files (crosslay 0.1) are still read. They have no right-click
//! frames, their frame headers stop after the anchor, and offset 34 is 0.
//!
//! This crate has no dependencies and does no allocation when parsing, so it
//! adds almost nothing to the overlay.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

pub const MAGIC: [u8; 4] = *b"CLAY";
pub const VERSION: u16 = 2;
/// Largest frame side the overlay accepts.
pub const MAX_SIDE: u16 = 4096;
/// Largest number of frames (profiles) in one file.
pub const MAX_FRAMES: u16 = 256;

/// Name of the baked file inside the data folder.
pub const BAKED_FILE: &str = "overlay.bin";
/// Name of the editable config inside the data folder.
pub const CONFIG_FILE: &str = "config.toml";

const HEADER_LEN: usize = 36;

/// Frame header length for a file `version`.
fn frame_header_len(version: u16) -> usize {
    if version == 1 { 16 } else { 20 }
}

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

/// How the right mouse button switches to a profile's right-click frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RightClick {
    /// Shown while the button is held (aim down sights in most games).
    #[default]
    Hold,
    /// Each press switches between the two (games with toggle aim).
    Toggle,
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
    pub right_click: RightClick,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visible: true,
            auto_update: true,
            monitor: 0,
            profile: 0,
            hotkeys: [Hotkey::default(); 5],
            right_click: RightClick::Hold,
        }
    }
}

/// One baked crosshair image.
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

/// One crosshair profile: what's shown normally, and optionally something
/// else to show while the right mouse button is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile<'a> {
    pub main: Frame<'a>,
    pub right_click: Option<Frame<'a>>,
}

/// A parsed `overlay.bin`, borrowing from the file's bytes.
#[derive(Clone, Debug)]
pub struct Baked<'a> {
    pub settings: Settings,
    version: u16,
    profile_count: u16,
    profiles: &'a [u8],
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
    /// [`Baked::profile`] can't fail later.
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
        if version != 1 && version != VERSION {
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
            right_click: match u16_at(bytes, 34) {
                1 => RightClick::Toggle,
                _ => RightClick::Hold,
            },
        };
        let profile_count = u16_at(bytes, 32);
        if profile_count == 0 {
            return Err(Error::Empty);
        }
        if profile_count > MAX_FRAMES {
            return Err(Error::BadFrame);
        }
        let baked = Baked {
            settings,
            version,
            profile_count,
            profiles: &bytes[HEADER_LEN..],
        };
        // Walk all profiles once to validate sizes.
        let mut rest = baked.profiles;
        for _ in 0..profile_count {
            rest = split_profile(rest, version)?.1;
        }
        if !rest.is_empty() {
            return Err(Error::BadFrame);
        }
        Ok(baked)
    }

    pub fn profile_count(&self) -> usize {
        self.profile_count as usize
    }

    /// Profile `index`, wrapping around the profile count.
    pub fn profile(&self, index: usize) -> Profile<'a> {
        let mut rest = self.profiles;
        for _ in 0..index % self.profile_count() {
            rest = split_profile(rest, self.version)
                .expect("validated in parse")
                .1;
        }
        split_profile(rest, self.version)
            .expect("validated in parse")
            .0
    }
}

fn split_profile(b: &[u8], version: u16) -> Result<(Profile<'_>, &[u8]), Error> {
    let (main, has_alt, rest) = split_frame(b, version)?;
    if !has_alt {
        return Ok((
            Profile {
                main,
                right_click: None,
            },
            rest,
        ));
    }
    let (alt, nested, rest) = split_frame(rest, version)?;
    if nested {
        return Err(Error::BadFrame);
    }
    Ok((
        Profile {
            main,
            right_click: Some(alt),
        },
        rest,
    ))
}

/// One frame, whether a right-click frame follows it, and the bytes after it.
fn split_frame(b: &[u8], version: u16) -> Result<(Frame<'_>, bool, &[u8]), Error> {
    let header = frame_header_len(version);
    if b.len() < header {
        return Err(Error::Truncated);
    }
    let width = u16_at(b, 8);
    let height = u16_at(b, 10);
    let anchor = (u16_at(b, 12), u16_at(b, 14));
    let flags = if version == 1 { 0 } else { u16_at(b, 16) };
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || anchor.0 >= width
        || anchor.1 >= height
        || flags > 1
    {
        return Err(Error::BadFrame);
    }
    let len = width as usize * height as usize * 4;
    let body = &b[header..];
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
    Ok((frame, flags & 1 != 0, &body[len..]))
}

/// Serialise settings and profiles. Panics if a frame's pixel buffer doesn't
/// match its size or a limit is exceeded; callers build frames themselves.
pub fn encode(settings: &Settings, profiles: &[Profile<'_>]) -> Vec<u8> {
    assert!(!profiles.is_empty() && profiles.len() <= MAX_FRAMES as usize);
    let header = frame_header_len(VERSION);
    let body: usize = profiles
        .iter()
        .flat_map(|p| std::iter::once(&p.main).chain(&p.right_click))
        .map(|f| header + f.pixels.len())
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
    out.extend_from_slice(&(profiles.len() as u16).to_le_bytes());
    let mode = match settings.right_click {
        RightClick::Hold => 0u16,
        RightClick::Toggle => 1,
    };
    out.extend_from_slice(&mode.to_le_bytes());
    let mut frame = |f: &Frame<'_>, has_alt: bool| {
        assert!(f.width > 0 && f.height > 0 && f.width <= MAX_SIDE && f.height <= MAX_SIDE);
        assert!(f.anchor.0 < f.width && f.anchor.1 < f.height);
        assert_eq!(f.pixels.len(), f.width as usize * f.height as usize * 4);
        out.extend_from_slice(&f.offset.0.to_le_bytes());
        out.extend_from_slice(&f.offset.1.to_le_bytes());
        out.extend_from_slice(&f.width.to_le_bytes());
        out.extend_from_slice(&f.height.to_le_bytes());
        out.extend_from_slice(&f.anchor.0.to_le_bytes());
        out.extend_from_slice(&f.anchor.1.to_le_bytes());
        out.extend_from_slice(&(has_alt as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(f.pixels);
    };
    for p in profiles {
        frame(&p.main, p.right_click.is_some());
        if let Some(alt) = &p.right_click {
            frame(alt, false);
        }
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

    fn frame(
        offset: (i32, i32),
        width: u16,
        height: u16,
        anchor: (u16, u16),
        pixels: &[u8],
    ) -> Frame<'_> {
        Frame {
            offset,
            width,
            height,
            anchor,
            pixels,
        }
    }

    fn settings() -> Settings {
        Settings {
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
            right_click: RightClick::Toggle,
        }
    }

    const A: [u8; 24] = [1; 24];
    const B: [u8; 4] = [2; 4];
    const C: [u8; 8] = [3; 8];

    fn sample() -> Vec<u8> {
        encode(
            &settings(),
            &[
                Profile {
                    main: frame((-5, 7), 3, 2, (1, 1), &A),
                    right_click: Some(frame((0, 4), 2, 1, (1, 0), &C)),
                },
                Profile {
                    main: frame((0, 0), 1, 1, (0, 0), &B),
                    right_click: None,
                },
            ],
        )
    }

    #[test]
    fn round_trips() {
        let bytes = sample();
        let baked = Baked::parse(&bytes).unwrap();
        assert_eq!(baked.settings, settings());
        assert_eq!(baked.profile_count(), 2);
        let p = baked.profile(0);
        assert_eq!(p.main, frame((-5, 7), 3, 2, (1, 1), &A));
        assert_eq!(p.right_click, Some(frame((0, 4), 2, 1, (1, 0), &C)));
        let q = baked.profile(1);
        assert_eq!(q.main.pixels, &B);
        assert_eq!(q.right_click, None);
        assert_eq!(baked.profile(2), baked.profile(0));
    }

    /// A file written by crosslay 0.1: 16-byte frame headers, no right-click.
    #[test]
    fn reads_version_1() {
        let mut v1 = Vec::new();
        v1.extend_from_slice(b"CLAY");
        v1.extend_from_slice(&1u16.to_le_bytes());
        v1.extend_from_slice(&1u16.to_le_bytes()); // visible
        v1.extend_from_slice(&[0; 4]); // monitor, profile
        v1.extend_from_slice(&[0; 20]); // hotkeys
        v1.extend_from_slice(&1u16.to_le_bytes()); // frame count
        v1.extend_from_slice(&0u16.to_le_bytes());
        v1.extend_from_slice(&3i32.to_le_bytes());
        v1.extend_from_slice(&(-2i32).to_le_bytes());
        for v in [1u16, 1, 0, 0] {
            v1.extend_from_slice(&v.to_le_bytes());
        }
        v1.extend_from_slice(&B);
        let baked = Baked::parse(&v1).unwrap();
        assert!(baked.settings.visible);
        assert_eq!(baked.settings.right_click, RightClick::Hold);
        let p = baked.profile(0);
        assert_eq!(p.main, frame((3, -2), 1, 1, (0, 0), &B));
        assert_eq!(p.right_click, None);
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
        let mut anchor = bytes.clone();
        anchor[36 + 12] = 3; // anchor_x == width
        assert_eq!(Baked::parse(&anchor).unwrap_err(), Error::BadFrame);
        // A right-click frame can't have one of its own.
        let mut nested = bytes;
        let alt = 36 + 20 + A.len();
        nested[alt + 16] = 1;
        assert!(Baked::parse(&nested).is_err());
    }
}
