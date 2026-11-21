use crate::color::Color;
use crate::crosshair::Crosshair;
use crate::hotkey::Hotkey;
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
    /// Accepted (and dropped) so configs from crosslay 0.1 still load.
    #[serde(skip_serializing)]
    live_reload: Option<bool>,
    pub hotkeys: Hotkeys,
    #[serde(rename = "crosshair")]
    pub crosshairs: Vec<Crosshair>,
}

/// Global hotkeys. Use an empty string to disable one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hotkeys {
    pub toggle: String,
    #[serde(alias = "menu")]
    pub editor: String,
    pub next: String,
    pub previous: String,
    pub quit: String,
    /// crosslay 0.1 had a reload hotkey; the overlay now reloads by itself.
    #[serde(skip_serializing)]
    reload: Option<String>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle: "Ctrl+Alt+X".into(),
            editor: "Ctrl+Alt+M".into(),
            next: "Ctrl+Alt+Right".into(),
            previous: "Ctrl+Alt+Left".into(),
            quit: "Ctrl+Alt+Q".into(),
            reload: None,
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
            live_reload: None,
            hotkeys: Hotkeys::default(),
            crosshairs: vec![
                Crosshair::default(),
                Crosshair {
                    name: "Dot".into(),
                    color: Color::rgb(0, 255, 255),
                    length: 0,
                    center_dot: true,
                    dot_size: 4,
                    ..Default::default()
                },
                Crosshair {
                    name: "Circle".into(),
                    color: Color::rgb(255, 255, 255),
                    length: 0,
                    center_dot: true,
                    dot_size: 2,
                    circle_radius: 10,
                    circle_thickness: 2,
                    ..Default::default()
                },
                Crosshair {
                    name: "T".into(),
                    color: Color::rgb(255, 0, 255),
                    thickness: 1,
                    length: 8,
                    gap: 4,
                    t_style: true,
                    center_dot: true,
                    dot_size: 1,
                    ..Default::default()
                },
            ],
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
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
            monitor: self.monitor.min(u16::MAX as usize) as u16,
            profile: (self.profile % self.crosshairs.len()) as u16,
            hotkeys,
        };
        let rendered: Vec<_> = self
            .crosshairs
            .iter()
            .map(|ch| {
                let canvas = ch.render_or_shapes(dir);
                (ch, canvas.to_bgra_bytes(), canvas)
            })
            .collect();
        let mut frames = Vec::with_capacity(rendered.len());
        for (ch, bytes, canvas) in &rendered {
            let side = crosslay_format::MAX_SIDE as u32;
            if canvas.width > side || canvas.height > side {
                return Err(format!(
                    "crosshair {:?} is {}x{} px; the most is {side}x{side}",
                    ch.name, canvas.width, canvas.height
                ));
            }
            frames.push(crosslay_format::Frame {
                offset: (ch.offset_x, ch.offset_y),
                width: canvas.width as u16,
                height: canvas.height as u16,
                anchor: (canvas.anchor.0 as u16, canvas.anchor.1 as u16),
                pixels: bytes,
            });
        }
        Ok(crosslay_format::encode(&settings, &frames))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active(&self) -> &Crosshair {
        &self.crosshairs[self.profile % self.crosshairs.len()]
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
# monitor   0 = primary monitor, 1.. = other monitors in system order
# profile   index of the [[crosshair]] shown at startup (0 = first)
# hotkeys   e.g. \"Ctrl+Alt+X\", \"Shift+F8\", \"Numpad5\"; \"\" disables a hotkey
#
# Each [[crosshair]] is a profile you can cycle through. Fields:
#   color / outline_color   \"#RRGGBB\" or \"#RRGGBBAA\"
#   thickness, length, gap  arm geometry in pixels (length = 0 hides the arms)
#   t_style                 hide the top arm
#   center_dot, dot_size    square centre dot
#   circle_radius           ring around the centre (0 = none), circle_thickness
#   outline                 outline width in pixels (0 = none)
#   image                   PNG file to show instead of the shapes (\"\" = shapes);
#                           relative paths are relative to this file's folder
#   image_scale             scale factor for the image
#   opacity                 0.0 to 1.0, applies to the whole crosshair
#   offset_x, offset_y      move the crosshair away from the screen centre

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
        assert_eq!(cfg.active().color, Color::rgb(255, 0, 0));
        assert_eq!(cfg.active().thickness, 2);
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
        assert_eq!(baked.frame_count(), cfg.crosshairs.len());
        assert_eq!(baked.settings.profile, 2);
        assert_eq!(
            baked.settings.hotkeys[Action::Toggle as usize].vk,
            b'X' as u16
        );
        let dot = baked.frame(1);
        let canvas = cfg.crosshairs[1].render_shapes();
        assert_eq!(dot.pixels, canvas.to_bgra_bytes().as_slice());
        assert_eq!(dot.anchor, (canvas.anchor.0 as u16, canvas.anchor.1 as u16));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn loads_old_configs() {
        let cfg = Config::parse(
            r##"
            live_reload = true
            [hotkeys]
            menu = "F9"
            reload = "Ctrl+Alt+R"
            "##,
        )
        .unwrap();
        assert_eq!(cfg.hotkeys.editor, "F9");
        assert!(!config_text(&cfg).contains("reload"));
    }

    #[test]
    fn rejects_bad_config() {
        assert!(Config::parse("crosshair = []").is_err());
        assert!(Config::parse("[hotkeys]\ntoggle = \"Ctrl+Nope\"").is_err());
        assert!(Config::parse("[[crosshair]]\ncolour = \"#FFFFFF\"").is_err());
    }
}
