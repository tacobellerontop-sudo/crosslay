//! Crosshair description and a CPU rasteriser that turns it into a [`Canvas`].
//!
//! Rendering happens only when the crosshair changes (startup, profile switch,
//! config reload). The result is handed to the compositor once and never
//! redrawn, so the overlay costs nothing per frame.
//!
//! A crosshair is either drawn from shapes (arms, dot, circle, outline) or,
//! when `image` is set, taken from a PNG file.

use crate::canvas::Canvas;
use crate::color::Color;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Crosshair {
    /// Display name, used when cycling profiles.
    pub name: String,
    /// Fill colour of every part of the crosshair.
    pub color: Color,
    /// Thickness of the arms in pixels.
    pub thickness: u32,
    /// Length of each arm in pixels (0 = no arms).
    pub length: u32,
    /// Distance in pixels between the centre block and each arm.
    pub gap: u32,
    /// Hide the top arm ("T" crosshair).
    pub t_style: bool,
    /// Draw a square dot in the centre.
    pub center_dot: bool,
    /// Side length of the centre dot in pixels.
    pub dot_size: u32,
    /// Radius of an optional circle around the centre (0 = no circle).
    pub circle_radius: u32,
    /// Stroke width of the circle in pixels.
    pub circle_thickness: u32,
    /// Outline width in pixels drawn around every part (0 = no outline).
    pub outline: u32,
    pub outline_color: Color,
    /// PNG file to use instead of the shapes above ("" = draw shapes).
    /// Relative paths are resolved against the config file's folder.
    pub image: String,
    /// Scale factor for `image` (nearest-neighbour, keeps pixel art crisp).
    pub image_scale: f32,
    /// Opacity of the whole crosshair, 0.0 to 1.0.
    pub opacity: f32,
    /// Shift the crosshair away from the screen centre, in pixels.
    pub offset_x: i32,
    pub offset_y: i32,
}

impl Default for Crosshair {
    fn default() -> Self {
        Self {
            name: "Classic".into(),
            color: Color::rgb(0, 255, 0),
            thickness: 2,
            length: 6,
            gap: 3,
            t_style: false,
            center_dot: false,
            dot_size: 2,
            circle_radius: 0,
            circle_thickness: 1,
            outline: 1,
            outline_color: Color::rgb(0, 0, 0),
            image: String::new(),
            image_scale: 1.0,
            opacity: 1.0,
            offset_x: 0,
            offset_y: 0,
        }
    }
}

/// Upper bound for any single size parameter, to keep a typo in the config
/// (e.g. `length = 100000`) from allocating gigabytes.
const MAX_DIM: u32 = 1024;

impl Crosshair {
    /// Whether this crosshair is drawn from a PNG instead of shapes.
    pub fn uses_image(&self) -> bool {
        !self.image.trim().is_empty()
    }

    /// Absolute path of `image`, resolving relative paths against `base_dir`.
    pub fn image_path(&self, base_dir: &Path) -> PathBuf {
        let p = Path::new(self.image.trim());
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            base_dir.join(p)
        }
    }

    /// Render the crosshair. `base_dir` is the folder relative image paths are
    /// resolved against (the config file's folder).
    pub fn render(&self, base_dir: &Path) -> Result<Canvas, String> {
        let mut canvas = if self.uses_image() {
            let img = crate::image::load_png(&self.image_path(base_dir))?;
            crate::image::scale(&img, self.image_scale)
        } else {
            self.render_shapes()
        };
        canvas.apply_opacity(self.opacity);
        Ok(canvas)
    }

    /// Like [`Self::render`], but falls back to the shapes if the image can't
    /// be loaded, so the overlay always shows something.
    pub fn render_or_shapes(&self, base_dir: &Path) -> Canvas {
        self.render(base_dir).unwrap_or_else(|_| {
            let mut c = self.render_shapes();
            c.apply_opacity(self.opacity);
            c
        })
    }

    /// Render the shape-based crosshair, ignoring `image` and `opacity`.
    pub fn render_shapes(&self) -> Canvas {
        let t = self.thickness.clamp(1, MAX_DIM) as i32;
        let len = self.length.min(MAX_DIM) as i32;
        let gap = self.gap.min(MAX_DIM) as i32;
        let dot = self.dot_size.clamp(1, MAX_DIM) as i32;
        let ring_r = self.circle_radius.min(MAX_DIM) as f32;
        let ring_t = self.circle_thickness.clamp(1, MAX_DIM) as f32;
        let outline = self.outline.min(64) as i32;

        // Half-extent of the drawing around the anchor pixel.
        let arms = if len > 0 { t / 2 + 1 + gap + len } else { 0 };
        let ring = if ring_r > 0.0 {
            (ring_r + ring_t).ceil() as i32 + 1
        } else {
            0
        };
        let dot_extent = if self.center_dot { dot / 2 + 1 } else { 0 };
        let r = arms.max(ring).max(dot_extent).max(1) + outline + 1;
        let size = (2 * r + 1) as usize;

        // Coverage of the fill shape, 0.0..=1.0 per pixel.
        let mut fill = vec![0f32; size * size];
        let mut rect = |x0: i32, y0: i32, x1: i32, y1: i32| {
            for y in y0.max(0)..y1.min(size as i32) {
                for x in x0.max(0)..x1.min(size as i32) {
                    fill[y as usize * size + x as usize] = 1.0;
                }
            }
        };

        // The centre block the arms are aligned to: `t` pixels wide, placed so
        // that the anchor pixel is inside it (odd thickness = perfectly centred,
        // even thickness = the extra pixel goes to the top/left).
        let lo = r - t / 2;
        let hi = lo + t;
        if len > 0 {
            if !self.t_style {
                rect(lo, lo - gap - len, hi, lo - gap); // top
            }
            rect(lo, hi + gap, hi, hi + gap + len); // bottom
            rect(lo - gap - len, lo, lo - gap, hi); // left
            rect(hi + gap, lo, hi + gap + len, hi); // right
        }
        if self.center_dot {
            let d0 = r - dot / 2;
            rect(d0, d0, d0 + dot, d0 + dot);
        }
        if ring_r > 0.0 {
            // Anti-aliased ring centred on the middle of the centre block.
            let c = lo as f32 + t as f32 / 2.0;
            for y in 0..size {
                for x in 0..size {
                    let dx = x as f32 + 0.5 - c;
                    let dy = y as f32 + 0.5 - c;
                    let d = (dx * dx + dy * dy).sqrt();
                    let cov = (0.5 + ring_t / 2.0 - (d - ring_r).abs()).clamp(0.0, 1.0);
                    let px = &mut fill[y * size + x];
                    *px = px.max(cov);
                }
            }
        }

        // Outline = fill shape dilated by `outline` pixels (square kernel,
        // done separably), drawn underneath the fill.
        let border = if outline > 0 {
            dilate(&fill, size, outline as usize)
        } else {
            vec![0.0; size * size]
        };

        let mut canvas = Canvas::new(size as u32, size as u32, (r, r));
        let fc = premul(self.color);
        let oc = premul(self.outline_color);
        for (i, px) in canvas.pixels.iter_mut().enumerate() {
            let f = fill[i];
            let o = border[i];
            // `fill over outline`, all in premultiplied space.
            let mut out = [0f32; 4];
            for k in 0..4 {
                let src = fc[k] * f;
                let dst = oc[k] * o;
                out[k] = src + dst * (1.0 - fc[3] * f);
            }
            let q = |v: f32| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u32;
            *px = q(out[3]) << 24 | q(out[0]) << 16 | q(out[1]) << 8 | q(out[2]);
        }
        canvas
    }
}

fn premul(c: Color) -> [f32; 4] {
    let a = c.a as f32 / 255.0;
    [
        c.r as f32 / 255.0 * a,
        c.g as f32 / 255.0 * a,
        c.b as f32 / 255.0 * a,
        a,
    ]
}

fn dilate(src: &[f32], size: usize, radius: usize) -> Vec<f32> {
    let mut tmp = vec![0f32; size * size];
    let mut out = vec![0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let (a, b) = (x.saturating_sub(radius), (x + radius).min(size - 1));
            tmp[y * size + x] = src[y * size + a..=y * size + b]
                .iter()
                .copied()
                .fold(0.0, f32::max);
        }
    }
    for y in 0..size {
        for x in 0..size {
            let (a, b) = (y.saturating_sub(radius), (y + radius).min(size - 1));
            out[y * size + x] = (a..=b).map(|yy| tmp[yy * size + x]).fold(0.0, f32::max);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(c: &Canvas, x: i32, y: i32) -> u32 {
        c.pixels[(y as u32 * c.width + x as u32) as usize] >> 24
    }

    #[test]
    fn classic_crosshair_is_symmetric() {
        let ch = Crosshair {
            thickness: 1,
            outline: 0,
            ..Default::default()
        };
        let c = ch.render_shapes();
        let (ax, ay) = c.anchor;
        // Gap around the anchor is empty, arms start right after it.
        assert_eq!(alpha(&c, ax, ay), 0);
        assert_eq!(alpha(&c, ax, ay - 3), 0);
        assert_eq!(alpha(&c, ax, ay - 4), 255);
        assert_eq!(alpha(&c, ax, ay + 4), 255);
        assert_eq!(alpha(&c, ax - 4, ay), 255);
        assert_eq!(alpha(&c, ax + 4, ay), 255);
        assert_eq!(alpha(&c, ax + 9, ay), 255);
        assert_eq!(alpha(&c, ax + 10, ay), 0);
        // Mirror symmetry for odd thickness.
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                let mx = 2 * ax - x;
                assert_eq!(alpha(&c, x, y), alpha(&c, mx, y));
            }
        }
    }

    #[test]
    fn t_style_hides_top_arm() {
        let c = Crosshair {
            t_style: true,
            outline: 0,
            ..Default::default()
        }
        .render_shapes();
        let (ax, ay) = c.anchor;
        assert_eq!(alpha(&c, ax, ay - 6), 0);
        assert_eq!(alpha(&c, ax, ay + 6), 255);
    }

    #[test]
    fn outline_surrounds_fill() {
        let c = Crosshair {
            center_dot: true,
            length: 0,
            dot_size: 1,
            outline: 1,
            ..Default::default()
        }
        .render_shapes();
        let (ax, ay) = c.anchor;
        let px = |x, y| c.pixels[(y as u32 * c.width + x as u32) as usize];
        assert_eq!(px(ax, ay), 0xFF00FF00);
        assert_eq!(px(ax + 1, ay + 1), 0xFF000000);
        assert_eq!(px(ax + 2, ay), 0);
    }

    #[test]
    fn huge_values_are_clamped() {
        let c = Crosshair {
            length: u32::MAX,
            gap: u32::MAX,
            ..Default::default()
        }
        .render_shapes();
        assert!(c.width < 4 * MAX_DIM + 200);
    }
}
