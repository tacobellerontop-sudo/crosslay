use serde::{Deserialize, Serialize};
use std::fmt;

/// An sRGB colour with straight alpha. Written in config files as `"#RRGGBB"`
/// or `"#RRGGBBAA"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }
}

impl std::str::FromStr for Color {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.trim().trim_start_matches('#');
        let byte = |i: usize| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| format!("invalid colour {s:?}: expected #RRGGBB or #RRGGBBAA"))
        };
        if !hex.is_ascii() {
            return Err(format!(
                "invalid colour {s:?}: expected #RRGGBB or #RRGGBBAA"
            ));
        }
        match hex.len() {
            6 => Ok(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Ok(Color::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => Err(format!(
                "invalid colour {s:?}: expected #RRGGBB or #RRGGBBAA"
            )),
        }
    }
}

impl TryFrom<String> for Color {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Color> for String {
    fn from(c: Color) -> String {
        c.to_string()
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            write!(
                f,
                "#{:02X}{:02X}{:02X}{:02X}",
                self.r, self.g, self.b, self.a
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        assert_eq!("#00ff00".parse::<Color>().unwrap(), Color::rgb(0, 255, 0));
        assert_eq!(
            "FF000080".parse::<Color>().unwrap(),
            Color::rgba(255, 0, 0, 128)
        );
        assert_eq!(Color::rgba(1, 2, 3, 4).to_string(), "#01020304");
        assert_eq!(Color::rgb(255, 255, 255).to_string(), "#FFFFFF");
        assert!("#12345".parse::<Color>().is_err());
        assert!("#GGGGGG".parse::<Color>().is_err());
        assert!("#ééé".parse::<Color>().is_err());
    }
}
