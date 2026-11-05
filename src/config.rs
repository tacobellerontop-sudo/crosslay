use crate::color::Color;
use crate::crosshair::Crosshair;
use crate::hotkey::Hotkey;
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
    /// Watch the config file and apply edits automatically.
    pub live_reload: bool,
    pub hotkeys: Hotkeys,
    #[serde(rename = "crosshair")]
    pub crosshairs: Vec<Crosshair>,
}

/// Global hotkeys. Use an empty string to disable one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hotkeys {
    pub toggle: String,
    pub next: String,
    pub previous: String,
    pub reload: String,
    pub quit: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle: "Ctrl+Alt+X".into(),
            next: "Ctrl+Alt+Right".into(),
            previous: "Ctrl+Alt+Left".into(),
            reload: "Ctrl+Alt+R".into(),
            quit: "Ctrl+Alt+Q".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Next,
    Previous,
    Reload,
    Quit,
}

impl Hotkeys {
    /// Parsed hotkeys, skipping disabled (empty) entries.
    pub fn bindings(&self) -> Result<Vec<(Action, Hotkey)>, String> {
        [
            (Action::Toggle, &self.toggle),
            (Action::Next, &self.next),
            (Action::Previous, &self.previous),
            (Action::Reload, &self.reload),
            (Action::Quit, &self.quit),
        ]
        .into_iter()
        .filter(|(_, s)| !s.trim().is_empty())
        .map(|(a, s)| Ok((a, s.parse()?)))
        .collect()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            monitor: 0,
            profile: 0,
            visible: true,
            live_reload: true,
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
        self.hotkeys.bindings()?;
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

    pub fn active(&self) -> &Crosshair {
        &self.crosshairs[self.profile % self.crosshairs.len()]
    }
}

/// The default config with explanatory comments, as written on first run.
pub fn default_config_text() -> String {
    let body = toml::to_string_pretty(&Config::default()).expect("default config serialises");
    format!(
        "\
# crosslay configuration
#
# Edits are applied automatically while crosslay runs (live_reload = true),
# or press the reload hotkey.
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
#   offset_x, offset_y      move the crosshair away from the screen centre

{body}"
    )
}

/// Where the config lives when `--config` is not given.
///
/// A `crosslay.toml` next to the executable wins (portable mode); otherwise the
/// per-user config directory is used.
pub fn default_path() -> PathBuf {
    if let Some(portable) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("crosslay.toml")))
        .filter(|p| p.exists())
    {
        return portable;
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("crosslay")
        .join("config.toml")
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
        assert_eq!(cfg.hotkeys.bindings().unwrap().len(), 4);
    }

    #[test]
    fn rejects_bad_config() {
        assert!(Config::parse("crosshair = []").is_err());
        assert!(Config::parse("[hotkeys]\ntoggle = \"Ctrl+Nope\"").is_err());
        assert!(Config::parse("[[crosshair]]\ncolour = \"#FFFFFF\"").is_err());
    }
}
