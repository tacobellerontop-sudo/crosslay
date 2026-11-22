use crate::color::Color;
use crate::crosshair::Crosshair;
use crate::hotkey::Hotkey;
use crate::layer::{Kind, Layer};
use crate::valorant::{Lines, Settings};
use crosslay_format::{Action, BAKED_FILE, CONFIG_FILE};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Monitor to draw on: 0 = primary, 1.. = the other monitors in system order.
    pub monitor: usize,
    /// Index into `crosshairs` of the profile shown at startup.
    pub profile: usize,
    /// Start with the overlay visible.
    pub visible: bool,
    /// Check GitHub for a new version each time the overlay starts, and
    /// install it.
    pub auto_update: bool,
    /// How the right mouse button switches to a crosshair's `right_click`
    /// crosshair.
    pub right_click: RightClickMode,
    pub hotkeys: Hotkeys,
    #[serde(rename = "crosshair")]
    pub crosshairs: Vec<Crosshair>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RightClickMode {
    /// Show it while the button is held.
    #[default]
    Hold,
    /// Each press switches between the two.
    Toggle,
}

/// Global hotkeys. Use an empty string to disable one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hotkeys {
    pub toggle: String,
    pub editor: String,
    pub next: String,
    pub previous: String,
    pub quit: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle: "Ctrl+Alt+X".into(),
            editor: "Ctrl+Alt+M".into(),
            next: "Ctrl+Alt+Right".into(),
            previous: "Ctrl+Alt+Left".into(),
            quit: "Ctrl+Alt+Q".into(),
        }
    }
}

impl Hotkeys {
    pub fn get(&self, action: Action) -> &String {
        match action {
            Action::Toggle => &self.toggle,
            Action::Editor => &self.editor,
            Action::Next => &self.next,
            Action::Previous => &self.previous,
            Action::Quit => &self.quit,
        }
    }

    /// Parsed hotkeys in [`Action::ALL`] order; `None` for disabled entries.
    pub fn parsed(&self) -> Result<[Option<Hotkey>; 5], String> {
        let mut out = [None; 5];
        for (slot, action) in out.iter_mut().zip(Action::ALL) {
            let s = self.get(action);
            if !s.trim().is_empty() {
                *slot = Some(s.parse()?);
            }
        }
        Ok(out)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            monitor: 0,
            profile: 0,
            visible: true,
            auto_update: true,
            right_click: RightClickMode::Hold,
            hotkeys: Hotkeys::default(),
            crosshairs: vec![
                Crosshair::default(),
                Crosshair::from_settings(
                    "Solid cross",
                    &Settings {
                        color: Color::rgb(0, 255, 0),
                        inner_lines: Lines {
                            opacity: 1.0,
                            length: 5,
                            offset: 0,
                            ..Lines::INNER
                        },
                        outer_lines: Lines::HIDDEN,
                        ..Default::default()
                    },
                ),
                Crosshair::from_settings(
                    "Dot",
                    &Settings {
                        color: Color::rgb(0, 255, 255),
                        center_dot: true,
                        center_dot_thickness: 4,
                        inner_lines: Lines::HIDDEN,
                        outer_lines: Lines::HIDDEN,
                        ..Default::default()
                    },
                ),
                Crosshair::from_settings(
                    "Wide",
                    &Settings {
                        color: Color::rgb(255, 255, 0),
                        center_dot: true,
                        inner_lines: Lines {
                            opacity: 1.0,
                            length: 10,
                            separate_vertical: true,
                            vertical_length: 3,
                            thickness: 2,
                            offset: 4,
                            ..Lines::INNER
                        },
                        outer_lines: Lines::HIDDEN,
                        ..Default::default()
                    },
                ),
                Crosshair {
                    name: "Circle".into(),
                    layers: vec![
                        Layer {
                            name: "Ring".into(),
                            width: 21.0,
                            thickness: 1.5,
                            ..Layer::new(Kind::Ring)
                        },
                        Layer {
                            name: "Dot".into(),
                            width: 3.0,
                            outline: 0.0,
                            ..Layer::new(Kind::Circle)
                        },
                        Layer {
                            name: "Ticks".into(),
                            x: 13.5,
                            width: 4.0,
                            thickness: 1.0,
                            copies: 4,
                            opacity: 0.8,
                            ..Layer::new(Kind::Line)
                        },
                    ],
                    ..Crosshair::from_settings("Circle", &Settings::default())
                },
            ],
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        // Notepad and Windows PowerShell like to add a UTF-8 byte order mark.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let cfg: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.crosshairs.is_empty() {
            return Err("config must contain at least one [[crosshair]]".into());
        }
        if self.crosshairs.len() > crosslay_format::MAX_FRAMES as usize {
            return Err(format!(
                "at most {} crosshairs are supported",
                crosslay_format::MAX_FRAMES
            ));
        }
        if let Some(ch) = self.crosshairs.iter().find(|c| {
            c.right_click
                .as_ref()
                .is_some_and(|rc| rc.right_click.is_some())
        }) {
            return Err(format!(
                "crosshair {:?}: a right_click crosshair can't have its own right_click",
                ch.name
            ));
        }
        self.hotkeys.parsed()?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Load the config at `path`, writing the default one there first if the
    /// file does not exist yet.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
            std::fs::write(path, default_config_text())
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        Self::load(path)
    }

    /// Validate and save the config into `dir`: `config.toml` for people and
    /// the baked `overlay.bin` for the overlay. Both are written to a temporary
    /// file and renamed into place, so the overlay never sees a half-written
    /// file.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let baked = self.bake(dir)?;
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        write_atomic(&dir.join(CONFIG_FILE), config_text(self).as_bytes())?;
        write_atomic(&dir.join(BAKED_FILE), &baked)
    }

    /// Render every crosshair and pack them into the `overlay.bin` format.
    /// Image paths are resolved against `dir`. A crosshair whose image can't
    /// be loaded is baked from its shapes, so the overlay always shows
    /// something.
    pub fn bake(&self, dir: &Path) -> Result<Vec<u8>, String> {
        self.validate()?;
        let hotkeys = self.hotkeys.parsed()?.map(|hk| {
            hk.map(|h| crosslay_format::Hotkey {
                modifiers: h.modifiers as u16,
                vk: h.vk as u16,
            })
            .unwrap_or_default()
        });
        let settings = crosslay_format::Settings {
            visible: self.visible,
            auto_update: self.auto_update,
            monitor: self.monitor.min(u16::MAX as usize) as u16,
            profile: (self.profile % self.crosshairs.len()) as u16,
            hotkeys,
            right_click: match self.right_click {
                RightClickMode::Hold => crosslay_format::RightClick::Hold,
                RightClickMode::Toggle => crosslay_format::RightClick::Toggle,
            },
        };
        let render = |ch: &Crosshair| -> Result<Baked, String> {
            let canvas = ch.render_or_shapes(dir);
            let side = crosslay_format::MAX_SIDE as u32;
            if canvas.width > side || canvas.height > side {
                return Err(format!(
                    "crosshair {:?} is {}x{} px; the most is {side}x{side}",
                    ch.name, canvas.width, canvas.height
                ));
            }
            Ok(Baked {
                offset: (ch.offset_x, ch.offset_y),
                width: canvas.width as u16,
                height: canvas.height as u16,
                anchor: (canvas.anchor.0 as u16, canvas.anchor.1 as u16),
                pixels: canvas.to_bgra_bytes(),
            })
        };
        let rendered = self
            .crosshairs
            .iter()
            .map(|ch| {
                Ok((
                    render(ch)?,
                    ch.right_click.as_deref().map(render).transpose()?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let profiles: Vec<_> = rendered
            .iter()
            .map(|(main, alt)| crosslay_format::Profile {
                main: main.frame(),
                right_click: alt.as_ref().map(Baked::frame),
            })
            .collect();
        Ok(crosslay_format::encode(&settings, &profiles))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active(&self) -> &Crosshair {
        &self.crosshairs[self.profile % self.crosshairs.len()]
    }
}

/// A rendered frame that owns its pixels.
struct Baked {
    offset: (i32, i32),
    width: u16,
    height: u16,
    anchor: (u16, u16),
    pixels: Vec<u8>,
}

impl Baked {
    fn frame(&self) -> crosslay_format::Frame<'_> {
        crosslay_format::Frame {
            offset: self.offset,
            width: self.width,
            height: self.height,
            anchor: self.anchor,
            pixels: &self.pixels,
        }
    }
}

/// The default config with explanatory comments, as written on first run.
pub fn default_config_text() -> String {
    config_text(&Config::default())
}

/// `cfg` serialised as TOML, with the explanatory header comment.
pub fn config_text(cfg: &Config) -> String {
    let body = toml::to_string_pretty(cfg).expect("config serialises");
    format!(
        "\
# crosslay configuration
#
# The overlay doesn't read this file. It reads overlay.bin, which
# crosslay-editor bakes from this file. Easiest: edit crosshairs in
# crosslay-editor. If you edit this file by hand, run
# `crosslay-editor --bake` afterwards (opening the editor does it too).
#
# monitor      0 = primary monitor, 1.. = other monitors in system order
# profile      index of the [[crosshair]] shown at startup (0 = first)
# auto_update  check for a new version when the overlay starts
# right_click  \"hold\": show a crosshair's right_click crosshair while the
#              right mouse button is held; \"toggle\": each click switches
# hotkeys      e.g. \"Ctrl+Alt+X\", \"Shift+F8\", \"Numpad5\"; \"\" disables one
#
# Each [[crosshair]] is a profile you can cycle through, built from shape
# layers. Easiest to make in crosslay-editor. Crosshair fields:
#   name                    shown when cycling crosshairs
#   image                   PNG file to show instead of the layers (\"\" = layers);
#                           relative paths are relative to this file's folder
#   image_scale             scale factor for the image
#   opacity                 0.0 to 1.0, applies to the whole crosshair
#   offset_x, offset_y      move the crosshair away from the screen centre
#   [[crosshair.layer]]     one shape, bottom first. Fields:
#       shape               cross, line, rect, circle, ring, arc or triangle
#       visible             false hides the layer
#       x, y                centre, in pixels from the screen centre (y down)
#       width               length (cross, line), width (rect, triangle) or
#                           diameter (circle, ring, arc)
#       height              vertical line length (cross), height (rect, triangle)
#       thickness           line width (cross, line, ring, arc)
#       gap                 space between the centre and the lines (cross)
#       sweep               degrees of the circle an arc covers
#       rotation            degrees, clockwise
#       copies              repeat evenly around the centre (4 = every 90°)
#       mirror_x, mirror_y  also draw it mirrored left-right / top-bottom
#       color, opacity      \"#RRGGBB\" or \"#RRGGBBAA\", and 0.0 to 1.0
#       outline             outline width in pixels (0 = none), with
#                           outline_color and outline_opacity
#       smooth              anti-aliased edges (off = sharp pixels)
#   [crosshair.right_click] optional: a second crosshair with the same fields,
#                           shown while the right mouse button is held (e.g.
#                           aiming down sights). Leave it out to keep one
#                           crosshair; the overlay then ignores the mouse.
#
# Crosshairs from crosslay 0.2 (color, inner_lines, ...) are still read and
# turned into layers.

{body}"
    )
}

/// Write `bytes` to `path` through a temporary file and a rename. The rename
/// is retried briefly because Windows refuses it while the overlay has the
/// old file open for its (very short) read.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    let mut tries = 0;
    loop {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(_) if tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("cannot write {}: {e}", path.display()));
            }
        }
    }
}

/// Whether `overlay.bin` in `dir` is missing or older than `config.toml`.
pub fn needs_bake(dir: &Path) -> bool {
    let modified = |name: &str| {
        std::fs::metadata(dir.join(name))
            .and_then(|m| m.modified())
            .ok()
    };
    match (modified(CONFIG_FILE), modified(BAKED_FILE)) {
        (Some(cfg), Some(bin)) => cfg > bin,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_text_round_trips() {
        let cfg = Config::parse(&default_config_text()).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_config_uses_defaults() {
        let cfg = Config::parse(
            r##"
            [hotkeys]
            quit = ""

            [[crosshair]]
            name = "Mine"
            color = "#FF0000"
            "##,
        )
        .unwrap();
        assert_eq!(cfg.crosshairs.len(), 1);
        // A crosslay 0.2 crosshair: Valorant's default lines, in red.
        let layers = &cfg.active().layers;
        assert_eq!(layers.len(), 2);
        assert!(layers.iter().all(|l| l.color == Color::rgb(255, 0, 0)));
        assert_eq!(layers[1].width, Lines::INNER.length as f32);
        let parsed = cfg.hotkeys.parsed().unwrap();
        assert_eq!(parsed.iter().flatten().count(), 4);
    }

    #[test]
    fn save_round_trips() {
        let dir = std::env::temp_dir().join(format!("crosslay-test-{}", std::process::id()));
        let mut cfg = Config::default();
        cfg.crosshairs[0].image = "images/missing.png".into();
        cfg.crosshairs[0].opacity = 0.5;
        cfg.profile = 2;
        cfg.save(&dir).unwrap();
        assert_eq!(Config::load(&dir.join(CONFIG_FILE)).unwrap(), cfg);
        assert!(!needs_bake(&dir));

        // The baked file holds every crosshair, the settings and real pixels.
        let bytes = std::fs::read(dir.join(BAKED_FILE)).unwrap();
        let baked = crosslay_format::Baked::parse(&bytes).unwrap();
        assert_eq!(baked.profile_count(), cfg.crosshairs.len());
        assert_eq!(baked.settings.profile, 2);
        assert_eq!(
            baked.settings.hotkeys[Action::Toggle as usize].vk,
            b'X' as u16
        );
        let dot = baked.profile(1).main;
        let canvas = cfg.crosshairs[1].render_shapes();
        assert_eq!(dot.pixels, canvas.to_bgra_bytes().as_slice());
        assert_eq!(dot.anchor, (canvas.anchor.0 as u16, canvas.anchor.1 as u16));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn right_click_crosshair_round_trips() {
        let dir = std::env::temp_dir().join(format!("crosslay-rc-{}", std::process::id()));
        let mut cfg = Config {
            right_click: RightClickMode::Toggle,
            ..Config::default()
        };
        let mut aim = cfg.crosshairs[0].right_click_template();
        aim.layers = vec![Layer::new(Kind::Rect)];
        aim.offset_y = 3;
        cfg.crosshairs[0].right_click = Some(Box::new(aim.clone()));
        let text = config_text(&cfg);
        assert!(text.contains("[crosshair.right_click]"), "{text}");
        assert!(text.contains("right_click = \"toggle\""), "{text}");
        assert_eq!(Config::parse(&text).unwrap(), cfg);

        cfg.save(&dir).unwrap();
        let bytes = std::fs::read(dir.join(BAKED_FILE)).unwrap();
        let baked = crosslay_format::Baked::parse(&bytes).unwrap();
        assert_eq!(
            baked.settings.right_click,
            crosslay_format::RightClick::Toggle
        );
        let p = baked.profile(0);
        let alt = p.right_click.expect("profile 0 has a right-click frame");
        assert_eq!(alt.pixels, aim.render_shapes().to_bgra_bytes().as_slice());
        assert_eq!(alt.offset, (0, 3));
        assert_ne!(p.main.pixels, alt.pixels);
        assert!(baked.profile(1).right_click.is_none());
        let _ = std::fs::remove_dir_all(dir);

        // Nesting is refused.
        let mut nested = Config::default();
        let mut inner = nested.crosshairs[0].clone();
        inner.right_click = Some(Box::new(Crosshair::default()));
        nested.crosshairs[0].right_click = Some(Box::new(inner));
        assert!(nested.validate().is_err());
    }

    #[test]
    fn accepts_byte_order_mark() {
        let cfg = Config::parse("\u{feff}monitor = 1").unwrap();
        assert_eq!(cfg.monitor, 1);
    }

    #[test]
    fn rejects_bad_config() {
        assert!(Config::parse("crosshair = []").is_err());
        assert!(Config::parse("[hotkeys]\ntoggle = \"Ctrl+Nope\"").is_err());
        assert!(Config::parse("[[crosshair]]\ncolour = \"#FFFFFF\"").is_err());
    }
}
