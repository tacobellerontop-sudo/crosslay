//! Loading PNG crosshairs into a [`Canvas`].

use crate::canvas::Canvas;
use std::path::Path;

/// Largest accepted image side, in pixels (before and after scaling).
pub const MAX_SIDE: u32 = 2048;

/// Decode a PNG into a premultiplied canvas anchored at its centre.
pub fn load_png(path: &Path) -> Result<Canvas, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    decode_png(std::io::BufReader::new(file)).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn decode_png(reader: impl std::io::BufRead + std::io::Seek) -> Result<Canvas, String> {
    let mut decoder = png::Decoder::new(reader);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("not a valid PNG: {e}"))?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!(
            "image is {w}x{h}; crosshair images must be at most {MAX_SIDE}x{MAX_SIDE}"
        ));
    }
    let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("cannot decode PNG: {e}"))?;
    let data = &buf[..frame.buffer_size()];
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("unsupported indexed PNG".into()),
    };

    let mut canvas = Canvas::new(w, h, ((w / 2) as i32, (h / 2) as i32));
    for (px, src) in canvas.pixels.iter_mut().zip(data.chunks_exact(channels)) {
        let (r, g, b, a) = match channels {
            1 => (src[0], src[0], src[0], 255),
            2 => (src[0], src[0], src[0], src[1]),
            3 => (src[0], src[1], src[2], 255),
            _ => (src[0], src[1], src[2], src[3]),
        };
        let a32 = a as u32;
        let pm = |c: u8| (c as u32 * a32 + 127) / 255;
        *px = a32 << 24 | pm(r) << 16 | pm(g) << 8 | pm(b);
    }
    Ok(canvas)
}

/// Nearest-neighbour scale, keeping the anchor on the same image point.
pub fn scale(src: &Canvas, factor: f32) -> Canvas {
    let factor = if factor.is_finite() {
        factor.clamp(0.05, 64.0)
    } else {
        1.0
    };
    if (factor - 1.0).abs() < 1e-6 {
        return src.clone();
    }
    let size = |n: u32| ((n as f32 * factor).round() as u32).clamp(1, MAX_SIDE);
    let (w, h) = (size(src.width), size(src.height));
    let mut out = Canvas::new(w, h, ((w / 2) as i32, (h / 2) as i32));
    for y in 0..h {
        let sy = ((y as f32 + 0.5) / factor) as u32;
        let sy = sy.min(src.height - 1);
        for x in 0..w {
            let sx = (((x as f32 + 0.5) / factor) as u32).min(src.width - 1);
            out.pixels[(y * w + x) as usize] = src.pixels[(sy * src.width + sx) as usize];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_what_preview_encodes() {
        let mut c = Canvas::new(3, 2, (1, 1));
        c.pixels = vec![
            0xFF00FF00, 0x80800000, 0, 0xFFFFFFFF, 0xFF000000, 0x40404040,
        ];
        let png = crate::png::encode(&c, 1);
        let d = decode_png(std::io::Cursor::new(png)).unwrap();
        assert_eq!((d.width, d.height, d.anchor), (3, 2, (1, 1)));
        assert_eq!(d.pixels, c.pixels);
    }

    #[test]
    fn scales_nearest() {
        let mut c = Canvas::new(2, 1, (1, 0));
        c.pixels = vec![1, 2];
        let s = scale(&c, 2.0);
        assert_eq!((s.width, s.height), (4, 2));
        assert_eq!(s.pixels, vec![1, 1, 2, 2, 1, 1, 2, 2]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_png(std::io::Cursor::new(b"nope".to_vec())).is_err());
    }
}
