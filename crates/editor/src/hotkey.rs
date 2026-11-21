//! Parsing of hotkey strings such as `"Ctrl+Alt+X"` into modifier flags and a
//! virtual-key code. The numeric values match the Win32 `MOD_*` / `VK_*`
//! constants so the Windows backend can pass them straight to `RegisterHotKey`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

impl std::str::FromStr for Hotkey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut modifiers = 0;
        let mut vk = None;
        for part in s.split('+').map(str::trim) {
            if part.is_empty() {
                return Err(format!("invalid hotkey {s:?}: empty key name"));
            }
            let lower = part.to_ascii_lowercase();
            let modifier = match lower.as_str() {
                "ctrl" | "control" => Some(MOD_CONTROL),
                "alt" => Some(MOD_ALT),
                "shift" => Some(MOD_SHIFT),
                "win" | "super" | "meta" => Some(MOD_WIN),
                _ => None,
            };
            match modifier {
                Some(m) => modifiers |= m,
                None if vk.is_some() => {
                    return Err(format!(
                        "invalid hotkey {s:?}: more than one non-modifier key"
                    ));
                }
                None => {
                    vk =
                        Some(key_code(&lower).ok_or_else(|| {
                            format!("invalid hotkey {s:?}: unknown key {part:?}")
                        })?);
                }
            }
        }
        let vk = vk.ok_or_else(|| format!("invalid hotkey {s:?}: no key given"))?;
        Ok(Hotkey { modifiers, vk })
    }
}

fn key_code(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 {
        let c = bytes[0];
        if c.is_ascii_lowercase() {
            return Some(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some(0x70 + n - 1);
    }
    if let Some(n) = name
        .strip_prefix("numpad")
        .or_else(|| name.strip_prefix("num"))
        .and_then(|n| n.parse::<u32>().ok())
    {
        return (n <= 9).then_some(0x60 + n);
    }
    Some(match name {
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "escape" | "esc" => 0x1B,
        "backspace" => 0x08,
        "pause" => 0x13,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "insert" | "ins" => 0x2D,
        "delete" | "del" => 0x2E,
        "scrolllock" => 0x91,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkeys() {
        assert_eq!(
            "Ctrl+Alt+X".parse::<Hotkey>().unwrap(),
            Hotkey {
                modifiers: MOD_CONTROL | MOD_ALT,
                vk: b'X' as u32
            }
        );
        assert_eq!(
            "f8".parse::<Hotkey>().unwrap(),
            Hotkey {
                modifiers: 0,
                vk: 0x77
            }
        );
        assert_eq!(
            "shift + PageUp".parse::<Hotkey>().unwrap(),
            Hotkey {
                modifiers: MOD_SHIFT,
                vk: 0x21
            }
        );
        assert_eq!("Numpad5".parse::<Hotkey>().unwrap().vk, 0x65);
        assert_eq!("Win+1".parse::<Hotkey>().unwrap().vk, b'1' as u32);
        assert!("Ctrl+Alt".parse::<Hotkey>().is_err());
        assert!("Ctrl+A+B".parse::<Hotkey>().is_err());
        assert!("F25".parse::<Hotkey>().is_err());
        assert!("Ctrl++".parse::<Hotkey>().is_err());
    }
}
