//! Crosshair description and a CPU rasteriser that turns it into a [`Canvas`].
//!
//! This runs in the editor only. The pixels it produces are baked into
//! `overlay.bin`, so the overlay never rasterises anything itself.
//!
//! The shape settings follow Valorant's crosshair menu: one colour, optional
//! outlines and centre dot, and two sets of lines (inner and outer), each with
//! its own opacity, length, thickness and offset. A line set's vertical length
//! can be set separately from its horizontal length. Alternatively a PNG can
//! be used instead of the shapes.

use crate::canvas::Canvas;
use crate::color::Color;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Crosshair {
    /// Display name, used when cycling profiles.
    pub name: String,
    /// Colour of the lines and the centre dot.
    pub color: Color,

    /// Draw a border around every part.
    pub outlines: bool,
    pub outline_opacity: f32,
    /// Border width in pixels.
    pub outline_thickness: u32,
    pub outline_color: Color,

    pub center_dot: bool,
    pub center_dot_opacity: f32,
    /// Side length of the square dot in pixels.
    pub center_dot_thickness: u32,

    /// The lines nearest the centre.
    pub inner_lines: Lines,
    /// A second set of lines, usually further out and fainter.
    pub outer_lines: Lines,

    /// PNG file to use instead of the shapes above ("" = draw shapes).
    /// Relative paths are resolved against the config file's folder.
    pub image: String,
    /// Scale factor for `image` (nearest-neighbour, keeps pixel art crisp).
    pub image_scale: f32,
    /// Opacity of the whole crosshair, 0.0 to 1.0, on top of the per-part
    /// opacities.
    pub opacity: f32,
    /// Shift the crosshair away from the screen centre, in pixels.
    pub offset_x: i32,
    pub offset_y: i32,
}

/// One set of four lines around the centre.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lines {
    pub show: bool,
    pub opacity: f32,
    /// Length of the left and right lines in pixels.
    pub length: u32,
    /// Set the vertical length separately from `length`.
    pub separate_vertical: bool,
    /// Length of the top and bottom lines when `separate_vertical` is on.
    pub vertical_length: u32,
    /// Line width in pixels (0 hides the lines).
    pub thickness: u32,
    /// Distance in pixels from the centre to where each line starts. At 0 the
    /// lines meet and form one solid cross.
    pub offset: u32,
}

impl Lines {
    pub const INNER: Lines = Lines {
        show: true,
        opacity: 0.8,
        length: 6,
        separate_vertical: false,
        vertical_length: 6,
        thickness: 2,
        offset: 3,
    };

    pub const OUTER: Lines = Lines {
        show: true,
        opacity: 0.35,
        length: 2,
        separate_vertical: false,
        vertical_length: 2,
        thickness: 2,
        offset: 10,
    };

    pub const HIDDEN: Lines = Lines {
        show: false,
        ..Lines::OUTER
    };

    /// Effective vertical length.
    pub fn vertical(&self) -> u32 {
        if self.separate_vertical {
            self.vertical_length
        } else {
            self.length
        }
    }

    fn visible(&self) -> bool {
        self.show && self.thickness > 0 && self.opacity > 0.0
    }
}

impl Default for Lines {
    fn default() -> Self {
        Lines::INNER
    }
}

impl Default for Crosshair {
    /// Valorant's default crosshair.
    fn default() -> Self {
        Self {
            name: "Default".into(),
            color: Color::rgb(255, 255, 255),
            outlines: true,
            outline_opacity: 0.5,
            outline_thickness: 1,
            outline_color: Color::rgb(0, 0, 0),
            center_dot: false,
            center_dot_opacity: 1.0,
            center_dot_thickness: 2,
            inner_lines: Lines::INNER,
            outer_lines: Lines::OUTER,
            image: String::new(),
            image_scale: 1.0,
            opacity: 1.0,
            offset_x: 0,
            offset_y: 0,
        }
    }
}

/// Upper bounds for sizes, so a typo in the config (e.g. `length = 100000`)
/// can't allocate gigabytes. The baked frame stays well under 4096 px.
const MAX_LEN: u32 = 500;
const MAX_THICKNESS: u32 = 64;
const MAX_OUTLINE: u32 = 16;

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
        let outline = if self.outlines {
            self.outline_thickness.min(MAX_OUTLINE) as i32
        } else {
            0
        };
        let dot = self.center_dot_thickness.clamp(1, MAX_THICKNESS) as i32;
        let extent = |l: &Lines| {
            if !l.visible() {
                return 0;
            }
            let reach = l.length.max(l.vertical()).min(MAX_LEN) + l.offset.min(MAX_LEN);
            (l.thickness.min(MAX_THICKNESS) / 2 + 1 + reach) as i32
        };
        let dot_extent = if self.center_dot { dot / 2 + 1 } else { 0 };
        // Half-size of the canvas around the anchor pixel.
        let r = extent(&self.inner_lines)
            .max(extent(&self.outer_lines))
            .max(dot_extent)
            .max(1)
            + outline
            + 1;
        let size = (2 * r + 1) as usize;

        // Per pixel: fill opacity (0 = empty). Parts are the same colour, so
        // where they overlap the most opaque one wins.
        let mut fill = vec![0f32; size * size];
        let mut rect = |x0: i32, y0: i32, x1: i32, y1: i32, a: f32| {
            for y in y0.max(0)..y1.min(size as i32) {
                for x in x0.max(0)..x1.min(size as i32) {
                    let px = &mut fill[y as usize * size + x as usize];
                    *px = px.max(a);
                }
            }
        };

        for lines in [&self.outer_lines, &self.inner_lines] {
            if !lines.visible() {
                continue;
            }
            let a = lines.opacity.clamp(0.0, 1.0);
            let t = lines.thickness.min(MAX_THICKNESS) as i32;
            let off = lines.offset.min(MAX_LEN) as i32;
            let h = lines.length.min(MAX_LEN) as i32;
            let v = lines.vertical().min(MAX_LEN) as i32;
            // The `t`-wide block the lines line up on. The anchor pixel is
            // inside it: odd thickness is exactly centred, even thickness puts
            // the extra pixel top/left.
            let lo = r - t / 2;
            let hi = lo + t;
            if h > 0 {
                rect(lo - off - h, lo, lo - off, hi, a); // left
                rect(hi + off, lo, hi + off + h, hi, a); // right
            }
            if v > 0 {
                rect(lo, lo - off - v, hi, lo - off, a); // top
                rect(lo, hi + off, hi, hi + off + v, a); // bottom
            }
            // With no offset the lines meet: fill the middle so the cross is
            // solid instead of having a hole in it.
            if off == 0 && (h > 0 || v > 0) {
                rect(lo, lo, hi, hi, a);
            }
        }
        if self.center_dot {
            let d0 = r - dot / 2;
            rect(
                d0,
                d0,
                d0 + dot,
                d0 + dot,
                self.center_dot_opacity.clamp(0.0, 1.0),
            );
        }

        // Outline = everything drawn, grown by `outline` pixels (square
        // kernel, done separably), minus the parts themselves.
        let border = if outline > 0 {
            let solid: Vec<f32> = fill.iter().map(|&a| (a > 0.0) as u8 as f32).collect();
            dilate(&solid, size, outline as usize)
        } else {
            vec![0.0; size * size]
        };

        let mut canvas = Canvas::new(size as u32, size as u32, (r, r));
        let fc = premul(self.color);
        let oc = premul(self.outline_color);
        let oa = self.outline_opacity.clamp(0.0, 1.0);
        for (i, px) in canvas.pixels.iter_mut().enumerate() {
            let out = if fill[i] > 0.0 {
                fc.map(|c| c * fill[i])
            } else {
                oc.map(|c| c * border[i] * oa)
            };
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

    /// Solid, outline-free inner lines only.
    fn plain(inner: Lines) -> Crosshair {
        Crosshair {
            outlines: false,
            inner_lines: Lines {
                opacity: 1.0,
                ..inner
            },
            outer_lines: Lines::HIDDEN,
            ..Default::default()
        }
    }

    #[test]
    fn lines_are_symmetric() {
        let c = plain(Lines {
            thickness: 1,
            ..Lines::INNER
        })
        .render_shapes();
        let (ax, ay) = c.anchor;
        // Offset 3 around the 1 px centre, then 6 px lines.
        assert_eq!(alpha(&c, ax, ay), 0);
        assert_eq!(alpha(&c, ax, ay - 3), 0);
        assert_eq!(alpha(&c, ax, ay - 4), 255);
        assert_eq!(alpha(&c, ax + 4, ay), 255);
        assert_eq!(alpha(&c, ax + 9, ay), 255);
        assert_eq!(alpha(&c, ax + 10, ay), 0);
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                assert_eq!(alpha(&c, x, y), alpha(&c, 2 * ax - x, y));
                assert_eq!(alpha(&c, x, y), alpha(&c, x, 2 * ay - y));
            }
        }
    }

    #[test]
    fn zero_offset_has_no_hole() {
        for thickness in 1..=4 {
            let c = plain(Lines {
                thickness,
                offset: 0,
                ..Lines::INNER
            })
            .render_shapes();
            let (ax, ay) = c.anchor;
            // A continuous line through the centre in both directions.
            for d in -6..=6 {
                assert_eq!(alpha(&c, ax + d, ay), 255, "t={thickness} x {d}");
                assert_eq!(alpha(&c, ax, ay + d), 255, "t={thickness} y {d}");
            }
        }
    }

    #[test]
    fn vertical_length_is_separate() {
        let c = plain(Lines {
            thickness: 1,
            offset: 0,
            length: 3,
            separate_vertical: true,
            vertical_length: 8,
            ..Lines::INNER
        })
        .render_shapes();
        let (ax, ay) = c.anchor;
        assert_eq!(alpha(&c, ax + 3, ay), 255);
        assert_eq!(alpha(&c, ax + 4, ay), 0);
        assert_eq!(alpha(&c, ax, ay + 8), 255);
        assert_eq!(alpha(&c, ax, ay - 8), 255);
        assert_eq!(alpha(&c, ax, ay + 9), 0);
    }

    #[test]
    fn vertical_length_zero_hides_vertical_lines() {
        let c = plain(Lines {
            thickness: 1,
            separate_vertical: true,
            vertical_length: 0,
            ..Lines::INNER
        })
        .render_shapes();
        let (ax, ay) = c.anchor;
        assert_eq!(alpha(&c, ax + 5, ay), 255);
        assert!((0..c.height as i32).all(|y| alpha(&c, ax, y) == 0));
    }

    #[test]
    fn default_has_inner_and_faint_outer_lines() {
        let ch = Crosshair::default();
        let c = ch.render_shapes();
        let (ax, ay) = c.anchor;
        // Inner line at 80 %, outer at 35 %, outline at 50 % black.
        assert_eq!(alpha(&c, ax + 5, ay), (0.8f32 * 255.0 + 0.5) as u32);
        assert_eq!(alpha(&c, ax + 12, ay), (0.35f32 * 255.0 + 0.5) as u32);
        assert_eq!(alpha(&c, ax + 5, ay - 2), (0.5f32 * 255.0 + 0.5) as u32);
        assert_eq!(alpha(&c, ax, ay), 0);
    }

    #[test]
    fn outline_surrounds_dot() {
        let c = Crosshair {
            center_dot: true,
            center_dot_thickness: 1,
            outline_opacity: 1.0,
            inner_lines: Lines::HIDDEN,
            outer_lines: Lines::HIDDEN,
            ..Default::default()
        }
        .render_shapes();
        let (ax, ay) = c.anchor;
        let px = |x, y| c.pixels[(y as u32 * c.width + x as u32) as usize];
        assert_eq!(px(ax, ay), 0xFFFFFFFF);
        assert_eq!(px(ax + 1, ay + 1), 0xFF000000);
        assert_eq!(px(ax + 2, ay), 0);
    }

    #[test]
    fn huge_values_are_clamped() {
        let c = plain(Lines {
            length: u32::MAX,
            vertical_length: u32::MAX,
            separate_vertical: true,
            offset: u32::MAX,
            thickness: u32::MAX,
            ..Lines::INNER
        })
        .render_shapes();
        assert!(c.width < crosslay_format::MAX_SIDE as u32);
    }
}
