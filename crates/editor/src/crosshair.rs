//! A crosshair: Valorant-style settings (or a PNG), its placement and its
//! optional right-click crosshair.
//!
//! This runs in the editor only. The pixels it produces are baked into
//! `overlay.bin`, so the overlay never rasterises anything itself.
//!
//! crosslay 0.3 and 0.4 built crosshairs from free-form shape layers. Those
//! still load: layers that only make up Valorant-style parts become settings
//! again, and anything else is kept as it is and still drawn.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::layer::Layer;
pub use crate::valorant::Lines;
use crate::valorant::Settings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "CrosshairFile")]
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

    /// PNG file to use instead of the settings ("" = draw from settings).
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

    /// Shapes from crosslay 0.3/0.4 that the settings above can't describe.
    /// When there are any, they are drawn instead of the settings.
    #[serde(rename = "layer", skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,

    /// A different crosshair to show while the right mouse button is held
    /// (or toggled, see `right_click` in the config), for aiming down sights.
    /// `None` keeps this one; the overlay then doesn't watch the mouse at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_click: Option<Box<Crosshair>>,
}

/// What a `[[crosshair]]` may contain, from any crosslay version.
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CrosshairFile {
    name: String,
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
    image: String,
    image_scale: f32,
    opacity: f32,
    offset_x: i32,
    offset_y: i32,
    /// crosslay 0.3/0.4 shapes.
    layer: Option<Vec<Layer>>,
    right_click: Option<Box<Crosshair>>,
}

impl Default for CrosshairFile {
    fn default() -> Self {
        let d = Crosshair::default();
        CrosshairFile {
            name: d.name,
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
            image: d.image,
            image_scale: d.image_scale,
            opacity: d.opacity,
            offset_x: d.offset_x,
            offset_y: d.offset_y,
            layer: None,
            right_click: None,
        }
    }
}

impl From<CrosshairFile> for Crosshair {
    fn from(f: CrosshairFile) -> Self {
        let d = Settings::default();
        let mut settings = Settings {
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
        };
        let mut layers = Vec::new();
        if let Some(shapes) = f.layer {
            // Shapes saved by 0.3/0.4: back to settings when they fit.
            match Settings::from_layers(&shapes) {
                Ok(s) => settings = s,
                Err(_) => layers = shapes,
            }
        }
        let mut ch = Crosshair {
            name: f.name,
            image: f.image,
            image_scale: f.image_scale,
            opacity: f.opacity,
            offset_x: f.offset_x,
            offset_y: f.offset_y,
            layers,
            right_click: f.right_click,
            ..Crosshair::default()
        };
        ch.set_settings(&settings);
        ch
    }
}

impl Default for Crosshair {
    /// Valorant's default crosshair.
    fn default() -> Self {
        let s = Settings::default();
        Self {
            name: "Default".into(),
            color: s.color,
            outlines: s.outlines,
            outline_opacity: s.outline_opacity,
            outline_thickness: s.outline_thickness,
            outline_color: s.outline_color,
            center_dot: s.center_dot,
            center_dot_opacity: s.center_dot_opacity,
            center_dot_thickness: s.center_dot_thickness,
            inner_lines: s.inner_lines,
            outer_lines: s.outer_lines,
            image: String::new(),
            image_scale: 1.0,
            opacity: 1.0,
            offset_x: 0,
            offset_y: 0,
            layers: Vec::new(),
            right_click: None,
        }
    }
}

impl Crosshair {
    /// The Valorant-style settings of this crosshair.
    pub fn settings(&self) -> Settings {
        Settings {
            color: self.color,
            outlines: self.outlines,
            outline_opacity: self.outline_opacity,
            outline_thickness: self.outline_thickness,
            outline_color: self.outline_color,
            center_dot: self.center_dot,
            center_dot_opacity: self.center_dot_opacity,
            center_dot_thickness: self.center_dot_thickness,
            inner_lines: self.inner_lines,
            outer_lines: self.outer_lines,
        }
    }

    /// Take over every Valorant-style setting from `s`, and drop any 0.3/0.4
    /// shapes so the settings are what's drawn.
    pub fn set_settings(&mut self, s: &Settings) {
        self.color = s.color;
        self.outlines = s.outlines;
        self.outline_opacity = s.outline_opacity;
        self.outline_thickness = s.outline_thickness;
        self.outline_color = s.outline_color;
        self.center_dot = s.center_dot;
        self.center_dot_opacity = s.center_dot_opacity;
        self.center_dot_thickness = s.center_dot_thickness;
        self.inner_lines = s.inner_lines;
        self.outer_lines = s.outer_lines;
    }

    /// Whether this crosshair is drawn from 0.3/0.4 shapes.
    pub fn uses_shapes(&self) -> bool {
        !self.layers.is_empty()
    }

    /// A copy of this crosshair to start its right-click crosshair from.
    pub fn right_click_template(&self) -> Crosshair {
        Crosshair {
            right_click: None,
            ..self.clone()
        }
    }

    /// Whether this crosshair is drawn from a PNG instead of settings.
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

    /// Like [`Self::render`], but falls back to the settings if the image
    /// can't be loaded, so the overlay always shows something.
    pub fn render_or_shapes(&self, base_dir: &Path) -> Canvas {
        self.render(base_dir).unwrap_or_else(|_| {
            let mut c = self.render_shapes();
            c.apply_opacity(self.opacity);
            c
        })
    }

    /// Render the settings (or 0.3/0.4 shapes), ignoring `image` and
    /// `opacity`.
    pub fn render_shapes(&self) -> Canvas {
        if self.uses_shapes() {
            crate::layer::render(&self.layers)
        } else {
            crate::layer::render(&self.settings().to_layers())
        }
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
        let c = Crosshair::default().render_shapes();
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
    fn settings_round_trip_through_toml() {
        let mut ch = Crosshair {
            name: "Mine".into(),
            color: Color::rgb(0, 255, 0),
            center_dot: true,
            ..Default::default()
        };
        ch.inner_lines.separate_vertical = true;
        ch.inner_lines.vertical_length = 9;
        ch.right_click = Some(Box::new(Crosshair::default()));
        let text = toml::to_string(&ch).unwrap();
        assert!(!text.contains("layer"), "{text}");
        let back: Crosshair = toml::from_str(&text).unwrap();
        assert_eq!(back, ch);
        // A bare [[crosshair]] is Valorant's default.
        let bare: Crosshair = toml::from_str("name = \"Default\"").unwrap();
        assert_eq!(bare, Crosshair::default());
    }

    #[test]
    fn shapes_from_0_4_become_settings_when_they_fit() {
        let settings = Settings {
            color: Color::rgb(255, 0, 0),
            center_dot: true,
            ..Default::default()
        };
        let shaped = Crosshair {
            layers: settings.to_layers(),
            ..Default::default()
        };
        let text = toml::to_string(&shaped).unwrap();
        assert!(text.contains("[[layer]]"), "{text}");
        let back: Crosshair = toml::from_str(&text).unwrap();
        assert!(!back.uses_shapes());
        assert_eq!(
            back.settings(),
            Settings::from_layers(&shaped.layers).unwrap()
        );
        assert_eq!(back.color, Color::rgb(255, 0, 0));
        assert!(back.center_dot);
        assert_eq!(back.render_shapes(), shaped.render_shapes());
    }

    #[test]
    fn other_shapes_from_0_4_are_kept_and_drawn() {
        let ring = Layer {
            width: 15.0,
            ..Layer::new(Kind::Ring)
        };
        let shaped = Crosshair {
            layers: vec![ring.clone()],
            ..Default::default()
        };
        let back: Crosshair = toml::from_str(&toml::to_string(&shaped).unwrap()).unwrap();
        assert_eq!(back.layers, vec![ring.clone()]);
        assert_eq!(back.render_shapes(), crate::layer::render(&[ring]));
    }
}
