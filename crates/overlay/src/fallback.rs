//! A built-in crosshair shown until a valid `overlay.bin` exists, so a fresh
//! install or a damaged file never leaves the screen blank.

use crosslay_format::Frame;

const SIZE: u16 = 21;
const C: usize = SIZE as usize / 2;
const GREEN: [u8; 4] = [0, 255, 0, 255]; // premultiplied BGRA
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// Green 1 px plus with a 3 px gap, 6 px arms and a 1 px black outline.
pub fn pixels() -> Vec<u8> {
    let n = SIZE as usize;
    let arm = |d: usize| (4..=9).contains(&d);
    let fill =
        |x: usize, y: usize| (x == C && arm(y.abs_diff(C))) || (y == C && arm(x.abs_diff(C)));
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let px = if fill(x, y) {
                GREEN
            } else if (y.saturating_sub(1)..=(y + 1).min(n - 1))
                .any(|yy| (x.saturating_sub(1)..=(x + 1).min(n - 1)).any(|xx| fill(xx, yy)))
            {
                BLACK
            } else {
                continue;
            };
            out[(y * n + x) * 4..][..4].copy_from_slice(&px);
        }
    }
    out
}

pub fn frame(pixels: &[u8]) -> Frame<'_> {
    Frame {
        offset: (0, 0),
        width: SIZE,
        height: SIZE,
        anchor: (C as u16, C as u16),
        pixels,
    }
}
