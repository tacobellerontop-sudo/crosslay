//! A crosshair: a stack of shape layers (or a PNG), its placement and its
//! optional right-click crosshair.
//!
//! This runs in the editor only. The pixels it produces are baked into
//! `overlay.bin`, so the overlay never rasterises anything itself.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::layer::Layer;
use crate::valorant::{Lines, Settings};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "CrosshairFile")]
pub struct Crosshair {
    /// Display name, used when cycling profiles.
    pub name: String,
    /// PNG file to use instead of the layers ("" = draw the layers).
    /// Relative paths are resolved against the config file's folder.
    pub image: String,
    /// Scale factor for `image` (nearest-neighbour, keeps pixel art crisp).
    pub image_scale: f32,
    /// Opacity of the whole crosshair, 0.0 to 1.0, on top of each layer's.
    pub opacity: f32,
    /// Shift the crosshair away from the screen centre, in pixels.
    pub offset_x: i32,
    pub offset_y: i32,
    /// The shapes, bottom first.
    #[serde(rename = "layer")]
    pub layers: Vec<Layer>,
    /// A different crosshair to show while the right mouse button is held
    /// (or toggled, see `right_click` in the config), for aiming down sights.
    /// `None` keeps this one; the overlay then doesn't watch the mouse at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_click: Option<Box<Crosshair>>,
}

/// What a `[[crosshair]]` may contain: crosslay 0.3's layers, or crosslay
/// 0.1/0.2's Valorant-style fields, which are turned into layers.
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CrosshairFile {
    name: String,
    image: String,
    image_scale: f32,
    opacity: f32,
    offset_x: i32,
    offset_y: i32,
    layer: Option<Vec<Layer>>,
    right_click: Option<Box<Crosshair>>,
    // crosslay 0.2
    color: Option<Color>,
    outlines: Option<bool>,
    outline_opacity: Option<f32>,
    outline_thickness: Option<u32>,
    outline_color: Option<Color>,
    center_dot: Option<bool>,
    center_dot_opacity: Option<f32>,
    center_dot_thickness: Option<u32>,
    inner_lines: Option<Lines>,
    outer_lines: Option<Lines>,
}

impl Default for CrosshairFile {
    fn default() -> Self {
        let d = Crosshair::default();
        CrosshairFile {
            name: d.name,
            image: d.image,
            image_scale: d.image_scale,
            opacity: d.opacity,
            offset_x: d.offset_x,
            offset_y: d.offset_y,
            layer: None,
            right_click: None,
            color: None,
            outlines: None,
            outline_opacity: None,
            outline_thickness: None,
            outline_color: None,
            center_dot: None,
            center_dot_opacity: None,
            center_dot_thickness: None,
            inner_lines: None,
            outer_lines: None,
        }
    }
}

impl From<CrosshairFile> for Crosshair {
    fn from(f: CrosshairFile) -> Self {
        let layers = f.layer.unwrap_or_else(|| {
            let d = Settings::default();
            Settings {
                color: f.color.unwrap_or(d.color),
                outlines: f.outlines.unwrap_or(d.outlines),
                outline_opacity: f.outline_opacity.unwrap_or(d.outline_opacity),
                outline_thickness: f.outline_thickness.unwrap_or(d.outline_thickness),
                outline_color: f.outline_color.unwrap_or(d.outline_color),
                center_dot: f.center_dot.unwrap_or(d.center_dot),
                center_dot_opacity: f.center_dot_opacity.unwrap_or(d.center_dot_opacity),
                center_dot_thickness: f.center_dot_thickness.unwrap_or(d.center_dot_thickness),
                inner_lines: f.inner_lines.unwrap_or(d.inner_lines),
                outer_lines: f.outer_lines.unwrap_or(d.outer_lines),
            }
            .to_layers()
        });
        Crosshair {
            name: f.name,
            image: f.image,
            image_scale: f.image_scale,
            opacity: f.opacity,
            offset_x: f.offset_x,
            offset_y: f.offset_y,
            layers,
            right_click: f.right_click,
        }
    }
}

impl Default for Crosshair {
    /// Valorant's default crosshair.
    fn default() -> Self {
        Self::from_settings("Default", &Settings::default())
    }
}

impl Crosshair {
    /// A crosshair drawn like Valorant would draw `settings`.
    pub fn from_settings(name: &str, settings: &Settings) -> Self {
        Crosshair {
            name: name.into(),
            image: String::new(),
            image_scale: 1.0,
            opacity: 1.0,
            offset_x: 0,
            offset_y: 0,
            layers: settings.to_layers(),
            right_click: None,
        }
    }

    /// A copy of this crosshair to start its right-click crosshair from.
    pub fn right_click_template(&self) -> Crosshair {
        Crosshair {
            right_click: None,
            ..self.clone()
        }
    }

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

    /// Render the layers, ignoring `image` and `opacity`.
    pub fn render_shapes(&self) -> Canvas {
        crate::layer::render(&self.layers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::Kind;

    fn alpha(c: &Canvas, x: i32, y: i32) -> u32 {
        c.pixels[(y as u32 * c.width + x as u32) as usize] >> 24
    }

    #[test]
    fn default_has_inner_and_faint_outer_lines() {
        let ch = Crosshair::default();
        let kinds: Vec<_> = ch.layers.iter().map(|l| l.shape).collect();
        assert_eq!(kinds, [Kind::Cross, Kind::Cross]);
        let c = ch.render_shapes();
        let (ax, ay) = c.anchor;
        // Inner line at 80 %, outer at 35 %, outline at 50 % black.
        assert_eq!(alpha(&c, ax + 5, ay), (0.8f32 * 255.0 + 0.5) as u32);
        assert_eq!(alpha(&c, ax + 12, ay), (0.35f32 * 255.0 + 0.5) as u32);
        assert_eq!(alpha(&c, ax + 5, ay - 2), (0.5f32 * 255.0 + 0.5) as u32);
        assert_eq!(alpha(&c, ax, ay), 0);
    }

    #[test]
    fn reads_crosslay_0_2_crosshairs() {
        let old: Crosshair = toml::from_str(
            r##"
            name = "Old"
            color = "#00FF00"
            center_dot = true
            outlines = false
            [inner_lines]
            length = 4
            offset = 0
            [outer_lines]
            show = false
            "##,
        )
        .unwrap();
        let s = Settings {
            color: Color::rgb(0, 255, 0),
            center_dot: true,
            outlines: false,
            inner_lines: Lines {
                length: 4,
                vertical_length: 4,
                offset: 0,
                ..Lines::INNER
            },
            outer_lines: Lines::HIDDEN,
            ..Settings::default()
        };
        assert_eq!(old.name, "Old");
        assert_eq!(old.layers, s.to_layers());
        // A bare [[crosshair]] is Valorant's default.
        let bare: Crosshair = toml::from_str("name = \"Default\"").unwrap();
        assert_eq!(bare, Crosshair::default());
    }

    #[test]
    fn layers_round_trip_through_toml() {
        let mut ch = Crosshair::default();
        ch.layers.push(crate::layer::Layer {
            rotation: 45.0,
            copies: 3,
            mirror_x: true,
            ..crate::layer::Layer::new(Kind::Triangle)
        });
        ch.right_click = Some(Box::new(Crosshair {
            layers: vec![],
            ..Crosshair::default()
        }));
        let text = toml::to_string(&ch).unwrap();
        let back: Crosshair = toml::from_str(&text).unwrap();
        assert_eq!(back, ch, "{text}");
    }
}
