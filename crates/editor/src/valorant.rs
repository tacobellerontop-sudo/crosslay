//! Valorant crosshair codes, such as `0;P;c;5;h;0;0l;4;0o;0;0a;1;0f;0;1b;0`.
//!
//! A code is a `;`-separated list of `key;value` pairs, split into sections:
//! general settings first, then `P` (primary), `A` (aim down sights) and `S`
//! (sniper). Only the primary crosshair maps onto crosslay. Keys that are left
//! out take Valorant's defaults, which are also [`Crosshair::default`].
//! Movement and firing error don't apply to a static overlay and are ignored.

use crate::color::Color;
use crate::crosshair::Crosshair;

/// Valorant's preset colours, by index (8 = custom, given with `u`).
const PALETTE: [Color; 8] = [
    Color::rgb(255, 255, 255), // white
    Color::rgb(0, 255, 0),     // green
    Color::rgb(127, 255, 0),   // yellowish green
    Color::rgb(223, 255, 0),   // greenish yellow
    Color::rgb(255, 255, 0),   // yellow
    Color::rgb(0, 255, 255),   // cyan
    Color::rgb(255, 0, 255),   // pink
    Color::rgb(255, 0, 0),     // red
];

/// Apply a Valorant code's primary crosshair to `base`, keeping its name,
/// image, placement and outline colour.
pub fn import(code: &str, base: &Crosshair) -> Result<Crosshair, String> {
    let code = code.trim();
    let mut tokens = code.split(';').map(str::trim);
    if tokens.next() != Some("0") {
        return Err("not a Valorant crosshair code (it should start with \"0;\")".into());
    }
    let defaults = Crosshair::default();
    let mut ch = Crosshair {
        name: base.name.clone(),
        image: base.image.clone(),
        image_scale: base.image_scale,
        opacity: base.opacity,
        offset_x: base.offset_x,
        offset_y: base.offset_y,
        outline_color: base.outline_color,
        ..defaults
    };
    let mut color_index = 0usize;
    let mut custom: Option<Color> = None;
    let mut section = ' ';
    // Whether the code gave a vertical length, for inner and outer lines.
    let mut vertical_given = [false; 2];
    while let Some(key) = tokens.next() {
        if key.is_empty() {
            continue;
        }
        if matches!(key, "P" | "A" | "S") {
            section = key.chars().next().unwrap_or(' ');
            continue;
        }
        let value = tokens
            .next()
            .ok_or_else(|| format!("the code ends after {key:?} without a value"))?;
        if section != 'P' {
            continue;
        }
        let num = || -> Result<f32, String> {
            value
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| format!("{key} has an invalid value {value:?}"))
        };
        let int = || num().map(|v| v.round() as u32);
        let flag = || num().map(|v| v != 0.0);
        let (lines, field) = match key.split_at(key.len().min(1)) {
            ("0", f) if !f.is_empty() => (Some(0), f),
            ("1", f) if !f.is_empty() => (Some(1), f),
            _ => (None, key),
        };
        if field == "v"
            && let Some(i) = lines
        {
            vertical_given[i] = true;
        }
        let lines = lines.map(|i| {
            if i == 0 {
                &mut ch.inner_lines
            } else {
                &mut ch.outer_lines
            }
        });
        match (lines, field) {
            (Some(l), "b") => l.show = flag()?,
            (Some(l), "t") => l.thickness = int()?,
            (Some(l), "l") => l.length = int()?,
            (Some(l), "v") => l.vertical_length = int()?,
            (Some(l), "g") => l.separate_vertical = flag()?,
            (Some(l), "o") => l.offset = int()?,
            (Some(l), "a") => l.opacity = num()?.min(1.0),
            (Some(_), _) => {} // movement / firing error and friends
            (None, "c") => color_index = int()? as usize,
            (None, "u") => {
                custom = Some(
                    value
                        .parse()
                        .map_err(|_| format!("u has an invalid colour {value:?}"))?,
                )
            }
            (None, "h") => ch.outlines = flag()?,
            (None, "t") => ch.outline_thickness = int()?,
            (None, "o") => ch.outline_opacity = num()?.min(1.0),
            (None, "d") => ch.center_dot = flag()?,
            (None, "z") => ch.center_dot_thickness = int()?,
            (None, "a") => ch.center_dot_opacity = num()?.min(1.0),
            _ => {}
        }
    }
    // Until "Adjust independently" is on, Valorant uses the horizontal length
    // both ways; keep the stored vertical length in step for the editor.
    for (l, given) in [&mut ch.inner_lines, &mut ch.outer_lines]
        .into_iter()
        .zip(vertical_given)
    {
        if !given {
            l.vertical_length = l.length;
        }
    }
    ch.color = match (color_index, custom) {
        (8, Some(c)) => c,
        (8, None) => return Err("custom colour (c;8) without a u value".into()),
        (i, _) => *PALETTE
            .get(i)
            .ok_or_else(|| format!("unknown colour index {i}"))?,
    };
    Ok(ch)
}

/// A Valorant code for `ch`'s shapes. Only settings that differ from
/// Valorant's defaults are written, like the game does. Colours outside the
/// preset list are written as custom colours.
pub fn export(ch: &Crosshair) -> String {
    let d = Crosshair::default();
    let mut out = vec!["0".to_string(), "P".to_string()];
    let mut push = |k: &str, v: String| {
        out.push(k.to_string());
        out.push(v);
    };
    let fmt = |v: f32| {
        let s = format!("{:.3}", v.clamp(0.0, 1.0));
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s.is_empty() {
            "0".into()
        } else {
            s.to_string()
        }
    };
    let opaque = Color { a: 255, ..ch.color };
    match PALETTE.iter().position(|&c| c == ch.color) {
        Some(0) => {}
        Some(i) => push("c", i.to_string()),
        None => {
            push("c", "8".into());
            let hex = opaque.to_string();
            let a = ch.color.a;
            push("u", format!("{}{a:02X}", &hex[1..]));
        }
    }
    if ch.outlines != d.outlines {
        push("h", (ch.outlines as u8).to_string());
    }
    if ch.outline_thickness != d.outline_thickness {
        push("t", ch.outline_thickness.to_string());
    }
    if ch.outline_opacity != d.outline_opacity {
        push("o", fmt(ch.outline_opacity));
    }
    if ch.center_dot != d.center_dot {
        push("d", (ch.center_dot as u8).to_string());
    }
    if ch.center_dot_thickness != d.center_dot_thickness {
        push("z", ch.center_dot_thickness.to_string());
    }
    if ch.center_dot_opacity != d.center_dot_opacity {
        push("a", fmt(ch.center_dot_opacity));
    }
    for (p, l, dl) in [
        ("0", &ch.inner_lines, &d.inner_lines),
        ("1", &ch.outer_lines, &d.outer_lines),
    ] {
        if l.show != dl.show {
            push(&format!("{p}b"), (l.show as u8).to_string());
        }
        if l.thickness != dl.thickness {
            push(&format!("{p}t"), l.thickness.to_string());
        }
        if l.length != dl.length {
            push(&format!("{p}l"), l.length.to_string());
        }
        if l.separate_vertical {
            push(&format!("{p}v"), l.vertical_length.to_string());
            push(&format!("{p}g"), "1".into());
        }
        if l.offset != dl.offset {
            push(&format!("{p}o"), l.offset.to_string());
        }
        if l.opacity != dl.opacity {
            push(&format!("{p}a"), fmt(l.opacity));
        }
    }
    out.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_code_is_default_crosshair() {
        let base = Crosshair::default();
        assert_eq!(import("0", &base).unwrap(), base);
        assert_eq!(import("0;P", &base).unwrap(), base);
        assert_eq!(export(&base), "0;P");
    }

    #[test]
    fn imports_a_typical_code() {
        let base = Crosshair {
            name: "Mine".into(),
            ..Default::default()
        };
        let ch = import(
            "0;s;1;P;c;5;h;0;f;0;0l;4;0v;9;0g;1;0o;0;0a;1;0f;0;1b;0;A;c;1;S;c;7",
            &base,
        )
        .unwrap();
        assert_eq!(ch.name, "Mine");
        assert_eq!(ch.color, Color::rgb(0, 255, 255));
        assert!(!ch.outlines);
        assert_eq!(ch.inner_lines.length, 4);
        assert!(ch.inner_lines.separate_vertical);
        assert_eq!(ch.inner_lines.vertical(), 9);
        assert_eq!(ch.inner_lines.offset, 0);
        assert_eq!(ch.inner_lines.opacity, 1.0);
        assert!(!ch.outer_lines.show);
        // Linked vertical length follows the horizontal one.
        let linked = import("0;P;0l;10", &base).unwrap();
        assert_eq!(linked.inner_lines.vertical_length, 10);
    }

    #[test]
    fn imports_custom_colour() {
        let ch = import("0;P;c;8;u;FF8000FF;d;1;z;3", &Crosshair::default()).unwrap();
        assert_eq!(ch.color, Color::rgb(255, 128, 0));
        assert!(ch.center_dot);
        assert_eq!(ch.center_dot_thickness, 3);
    }

    #[test]
    fn round_trips() {
        let mut ch = Crosshair {
            color: Color::rgba(10, 20, 30, 200),
            outline_opacity: 0.25,
            center_dot: true,
            ..Default::default()
        };
        ch.inner_lines.offset = 0;
        ch.inner_lines.separate_vertical = true;
        ch.inner_lines.vertical_length = 12;
        ch.outer_lines.show = false;
        let code = export(&ch);
        assert_eq!(import(&code, &Crosshair::default()).unwrap(), ch);
    }

    #[test]
    fn rejects_garbage() {
        let base = Crosshair::default();
        assert!(import("hello", &base).is_err());
        assert!(import("0;P;c", &base).is_err());
        assert!(import("0;P;c;9", &base).is_err());
        assert!(import("0;P;0l;-3", &base).is_err());
        assert!(import("0;P;c;8", &base).is_err());
    }
}
