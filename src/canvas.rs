//! A tiny premultiplied-alpha image that every overlay layer renders into.
//!
//! Crosshairs render into a `Canvas` today; image overlays will produce one too,
//! so the platform backends only ever need to know how to show a `Canvas`.

/// Premultiplied BGRA pixels, stored as `0xAARRGGBB` words.
///
/// On little-endian machines (every Windows target) the in-memory byte order is
/// B, G, R, A — exactly what a 32-bit top-down DIB section expects, so a backend
/// can blit the buffer without any per-pixel conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    /// The pixel that should sit exactly on the target point (e.g. screen centre).
    pub anchor: (i32, i32),
}

impl Canvas {
    pub fn new(width: u32, height: u32, anchor: (i32, i32)) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width as usize) * (height as usize)],
            anchor,
        }
    }

    /// Straight (non-premultiplied) RGBA bytes, handy for exporting previews.
    pub fn to_straight_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &px in &self.pixels {
            let a = (px >> 24) & 0xFF;
            let unpremul =
                |c: u32| -> u8 { (c * 255 + a / 2).checked_div(a).unwrap_or(0).min(255) as u8 };
            out.push(unpremul((px >> 16) & 0xFF));
            out.push(unpremul((px >> 8) & 0xFF));
            out.push(unpremul(px & 0xFF));
            out.push(a as u8);
        }
        out
    }
}

impl Canvas {
    /// Multiply every pixel (all premultiplied channels) by `opacity`.
    pub fn apply_opacity(&mut self, opacity: f32) {
        let o = opacity.clamp(0.0, 1.0);
        if o >= 1.0 {
            return;
        }
        let k = (o * 256.0) as u32;
        for px in &mut self.pixels {
            let c = *px;
            let ch = |shift: u32| ((((c >> shift) & 0xFF) * k) >> 8) << shift;
            *px = ch(24) | ch(16) | ch(8) | ch(0);
        }
    }
}
