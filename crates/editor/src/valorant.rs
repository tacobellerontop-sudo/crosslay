//! Valorant crosshair codes, such as `0;P;c;5;h;0;0l;4;0o;0;0a;1;0f;0;1b;0`.
//!
//! A code is a `;`-separated list of `key;value` pairs, split into sections:
//! general settings first, then `P` (primary), `A` (aim down sights) and `S`
//! (sniper). The primary crosshair maps onto a crosslay crosshair and the ADS
//! one onto its right-click crosshair. Keys that are left out take Valorant's
//! defaults, which are also [`Settings::default`].
//! Movement and firing error don't apply to a static overlay and are ignored.
//!
//! A code becomes shape layers: a cross for the outer lines, one for the
//! inner lines and a square for the centre dot, drawn exactly as Valorant
//! draws them. Crosshairs made only of such layers can be copied back out as
//! a code.
//!
//! [`Settings`] is also how crosslay 0.1 and 0.2 stored crosshairs, so old
//! config files load through it.

use crate::color::Color;
use crate::layer::{Kind, Layer};
use serde::Deserialize;

/// Valorant's preset colours, by index (8 = custom, given with `u`).
pub const PALETTE: [Color; 8] = [
    Color::rgb(255, 255, 255), // white
    Color::rgb(0, 255, 0),     // green
    Color::rgb(127, 255, 0),   // yellowish green
    Color::rgb(223, 255, 0),   // greenish yellow
    Color::rgb(255, 255, 0),   // yellow
    Color::rgb(0, 255, 255),   // cyan
    Color::rgb(255, 0, 255),   // pink
    Color::rgb(255, 0, 0),     // red
];

/// Valorant's crosshair settings (and crosslay 0.2's crosshair fields).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
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
}

/// One set of four lines around the centre.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, serde::Serialize)]
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
    /// Distance in pixels from the centre band to where each line starts.
    /// At 0 the lines meet and form one solid cross.
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
}

impl Default for Lines {
    fn default() -> Self {
        Lines::INNER
    }
}

impl Default for Settings {
    /// Valorant's default crosshair.
    fn default() -> Self {
        Settings {
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
        }
    }
}

/// Upper bounds Valorant-style settings were always clamped to.
const MAX_LEN: u32 = 500;
const MAX_THICKNESS: u32 = 64;
const MAX_OUTLINE: u32 = 16;

impl Settings {
    /// The crosshair as shape layers, bottom first: outer lines, inner
    /// lines, centre dot. Parts that wouldn't show are left out.
    pub fn to_layers(self) -> Vec<Layer> {
        let outline = if self.outlines {
            self.outline_thickness.min(MAX_OUTLINE) as f32
        } else {
            0.0
        };
        let part = |name: &str, shape: Kind, opacity: f32| Layer {
            name: name.into(),
            shape,
            color: self.color,
            opacity: opacity.clamp(0.0, 1.0),
            outline,
            outline_color: self.outline_color,
            outline_opacity: self.outline_opacity.clamp(0.0, 1.0),
            smooth: false,
            ..Layer::new(shape)
        };
        let mut layers = Vec::new();
        for (name, l) in [
            ("Outer lines", &self.outer_lines),
            ("Inner lines", &self.inner_lines),
        ] {
            let (h, v) = (l.length.min(MAX_LEN), l.vertical().min(MAX_LEN));
            if !l.show || l.thickness == 0 || l.opacity <= 0.0 || (h == 0 && v == 0) {
                continue;
            }
            layers.push(Layer {
                width: h as f32,
                height: v as f32,
                thickness: l.thickness.min(MAX_THICKNESS) as f32,
                gap: l.offset.min(MAX_LEN) as f32,
                ..part(name, Kind::Cross, l.opacity)
            });
        }
        if self.center_dot && self.center_dot_opacity > 0.0 {
            let size = self.center_dot_thickness.clamp(1, MAX_THICKNESS) as f32;
            layers.push(Layer {
                width: size,
                height: size,
                ..part("Center dot", Kind::Rect, self.center_dot_opacity)
            });
        }
        layers
    }

    /// The Valorant settings that draw `layers`, if they are only what
    /// [`Self::to_layers`] makes: up to two crosses and a square dot at the
    /// centre, unrotated, in one colour with one outline style.
    pub fn from_layers(layers: &[Layer]) -> Result<Settings, String> {
        let shown: Vec<&Layer> = layers
            .iter()
            .filter(|l| l.visible && l.opacity > 0.0 && l.color.a > 0)
            .collect();
        let mut s = Settings {
            inner_lines: Lines::HIDDEN,
            outer_lines: Lines::HIDDEN,
            ..Settings::default()
        };
        let Some(first) = shown.first() else {
            return Ok(s);
        };
        let whole = |v: f32| v >= 0.0 && v.fract() == 0.0 && v <= MAX_LEN as f32;
        s.color = first.color;
        s.outlines = first.outline > 0.0 && first.outline_opacity > 0.0;
        if s.outlines {
            if !whole(first.outline) {
                return Err("outlines must be whole pixels".into());
            }
            s.outline_thickness = first.outline as u32;
            s.outline_opacity = first.outline_opacity;
            s.outline_color = first.outline_color;
        }
        let mut crosses = Vec::new();
        for l in &shown {
            if l.color != s.color {
                return Err("every part must be the same colour".into());
            }
            let outlined = l.outline > 0.0 && l.outline_opacity > 0.0;
            if outlined != s.outlines
                || (outlined
                    && (l.outline != first.outline
                        || l.outline_opacity != s.outline_opacity
                        || l.outline_color != s.outline_color))
            {
                return Err("every part must have the same outline".into());
            }
            if l.x != 0.0
                || l.y != 0.0
                || l.rotation.rem_euclid(360.0) != 0.0
                || l.copies > 1
                || l.mirror_x
                || l.mirror_y
                || l.smooth
            {
                return Err(format!(
                    "{:?} must sit on the centre, unrotated, without copies or smooth edges",
                    l.name
                ));
            }
            let sizes_whole = whole(l.width) && whole(l.height) && whole(l.thickness);
            match l.shape {
                Kind::Cross if sizes_whole && whole(l.gap) && l.thickness >= 1.0 => {
                    let h = l.width as u32;
                    let v = l.height as u32;
                    crosses.push(Lines {
                        show: true,
                        opacity: l.opacity.min(1.0),
                        length: h,
                        separate_vertical: h != v,
                        vertical_length: v,
                        thickness: l.thickness as u32,
                        offset: l.gap as u32,
                    });
                }
                Kind::Rect
                    if sizes_whole && l.width == l.height && l.width >= 1.0 && !s.center_dot =>
                {
                    s.center_dot = true;
                    s.center_dot_thickness = l.width as u32;
                    s.center_dot_opacity = l.opacity.min(1.0);
                }
                _ => {
                    return Err(format!(
                        "Valorant only has two sets of lines and a square centre dot in \
                         whole pixels; {:?} doesn't fit",
                        l.name
                    ));
                }
            }
        }
        match crosses[..] {
            [] => {}
            [inner] => s.inner_lines = inner,
            [outer, inner] => {
                s.outer_lines = outer;
                s.inner_lines = inner;
            }
            _ => return Err("Valorant only has two sets of lines".into()),
        }
        Ok(s)
    }
}

/// Which crosshair of a code to read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Primary,
    /// The aim-down-sights crosshair, falling back to the primary one when
    /// the code has no ADS settings.
    Ads,
}

/// Read one of a Valorant code's crosshairs.
pub fn import(code: &str, section: Section) -> Result<Settings, String> {
    let code = code.trim();
    // An ADS section with at least one setting in it.
    let has_ads = code
        .split(';')
        .map(str::trim)
        .skip_while(|t| *t != "A")
        .nth(1)
        .is_some_and(|t| !matches!(t, "P" | "S"));
    let wanted = if section == Section::Ads && has_ads {
        'A'
    } else {
        'P'
    };
    let mut tokens = code.split(';').map(str::trim);
    if tokens.next() != Some("0") {
        return Err("not a Valorant crosshair code (it should start with \"0;\")".into());
    }
    let mut ch = Settings::default();
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
        if section != wanted {
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

/// A Valorant code for `ch`. Only settings that differ from Valorant's
/// defaults are written, like the game does. Colours outside the preset list
/// are written as custom colours.
pub fn export(ch: &Settings) -> String {
    let d = Settings::default();
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
    use crate::canvas::Canvas;

    #[test]
    fn default_code_is_default_crosshair() {
        let d = Settings::default();
        assert_eq!(import("0", Section::Primary).unwrap(), d);
        assert_eq!(import("0;P", Section::Primary).unwrap(), d);
        assert_eq!(export(&d), "0;P");
    }

    #[test]
    fn imports_a_typical_code() {
        let ch = import(
            "0;s;1;P;c;5;h;0;f;0;0l;4;0v;9;0g;1;0o;0;0a;1;0f;0;1b;0;A;c;1;S;c;7",
            Section::Primary,
        )
        .unwrap();
        assert_eq!(ch.color, Color::rgb(0, 255, 255));
        assert!(!ch.outlines);
        assert_eq!(ch.inner_lines.length, 4);
        assert!(ch.inner_lines.separate_vertical);
        assert_eq!(ch.inner_lines.vertical(), 9);
        assert_eq!(ch.inner_lines.offset, 0);
        assert_eq!(ch.inner_lines.opacity, 1.0);
        assert!(!ch.outer_lines.show);
        // Linked vertical length follows the horizontal one.
        let linked = import("0;P;0l;10", Section::Primary).unwrap();
        assert_eq!(linked.inner_lines.vertical_length, 10);
    }

    #[test]
    fn imports_custom_colour() {
        let ch = import("0;P;c;8;u;FF8000FF;d;1;z;3", Section::Primary).unwrap();
        assert_eq!(ch.color, Color::rgb(255, 128, 0));
        assert!(ch.center_dot);
        assert_eq!(ch.center_dot_thickness, 3);
    }

    #[test]
    fn round_trips() {
        let mut ch = Settings {
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
        assert_eq!(import(&code, Section::Primary).unwrap(), ch);
    }

    #[test]
    fn imports_ads_section() {
        let code = "0;P;c;5;0l;4;A;c;1;d;1;0b;0";
        let primary = import(code, Section::Primary).unwrap();
        assert_eq!(primary.color, Color::rgb(0, 255, 255));
        let ads = import(code, Section::Ads).unwrap();
        assert_eq!(ads.color, Color::rgb(0, 255, 0));
        assert!(ads.center_dot);
        assert!(!ads.inner_lines.show);
        // No ADS settings in the code: use the primary crosshair.
        for code in ["0;P;c;5", "0;P;c;5;A;S;c;1"] {
            let ads = import(code, Section::Ads).unwrap();
            assert_eq!(ads.color, Color::rgb(0, 255, 255), "{code}");
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(import("hello", Section::Primary).is_err());
        assert!(import("0;P;c", Section::Primary).is_err());
        assert!(import("0;P;c;9", Section::Primary).is_err());
        assert!(import("0;P;0l;-3", Section::Primary).is_err());
        assert!(import("0;P;c;8", Section::Primary).is_err());
    }

    /// crosslay 0.2's rasteriser for Valorant-style settings, kept to check
    /// that the layers draw the same pixels.
    fn legacy_render(s: &Settings) -> Canvas {
        let visible = |l: &Lines| l.show && l.thickness > 0 && l.opacity > 0.0;
        let outline = if s.outlines {
            s.outline_thickness.min(MAX_OUTLINE) as i32
        } else {
            0
        };
        let dot = s.center_dot_thickness.clamp(1, MAX_THICKNESS) as i32;
        let extent = |l: &Lines| {
            if !visible(l) {
                return 0;
            }
            let reach = l.length.max(l.vertical()).min(MAX_LEN) + l.offset.min(MAX_LEN);
            (l.thickness.min(MAX_THICKNESS) / 2 + 1 + reach) as i32
        };
        let dot_extent = if s.center_dot { dot / 2 + 1 } else { 0 };
        let r = extent(&s.inner_lines)
            .max(extent(&s.outer_lines))
            .max(dot_extent)
            .max(1)
            + outline
            + 1;
        let size = (2 * r + 1) as usize;
        let mut fill = vec![0f32; size * size];
        let mut rect = |x0: i32, y0: i32, x1: i32, y1: i32, a: f32| {
            for y in y0.max(0)..y1.min(size as i32) {
                for x in x0.max(0)..x1.min(size as i32) {
                    let px = &mut fill[y as usize * size + x as usize];
                    *px = px.max(a);
                }
            }
        };
        for lines in [&s.outer_lines, &s.inner_lines] {
            if !visible(lines) {
                continue;
            }
            let a = lines.opacity.clamp(0.0, 1.0);
            let t = lines.thickness.min(MAX_THICKNESS) as i32;
            let off = lines.offset.min(MAX_LEN) as i32;
            let h = lines.length.min(MAX_LEN) as i32;
            let v = lines.vertical().min(MAX_LEN) as i32;
            let lo = r - t / 2;
            let hi = lo + t;
            if h > 0 {
                rect(lo - off - h, lo, lo - off, hi, a);
                rect(hi + off, lo, hi + off + h, hi, a);
            }
            if v > 0 {
                rect(lo, lo - off - v, hi, lo - off, a);
                rect(lo, hi + off, hi, hi + off + v, a);
            }
            if off == 0 && (h > 0 || v > 0) {
                rect(lo, lo, hi, hi, a);
            }
        }
        if s.center_dot {
            let d0 = r - dot / 2;
            rect(
                d0,
                d0,
                d0 + dot,
                d0 + dot,
                s.center_dot_opacity.clamp(0.0, 1.0),
            );
        }
        let solid: Vec<bool> = fill.iter().map(|&a| a > 0.0).collect();
        let mut border = vec![false; size * size];
        for y in 0..size as i32 {
            for x in 0..size as i32 {
                'n: for dy in -outline..=outline {
                    for dx in -outline..=outline {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx >= 0
                            && ny >= 0
                            && (nx as usize) < size
                            && (ny as usize) < size
                            && solid[ny as usize * size + nx as usize]
                        {
                            border[y as usize * size + x as usize] = true;
                            break 'n;
                        }
                    }
                }
            }
        }
        let pm = |c: Color| {
            let a = c.a as f32 / 255.0;
            [
                c.r as f32 / 255.0 * a,
                c.g as f32 / 255.0 * a,
                c.b as f32 / 255.0 * a,
                a,
            ]
        };
        let (fc, oc) = (pm(s.color), pm(s.outline_color));
        let oa = s.outline_opacity.clamp(0.0, 1.0);
        let mut canvas = Canvas::new(size as u32, size as u32, (r, r));
        for (i, px) in canvas.pixels.iter_mut().enumerate() {
            let out = if fill[i] > 0.0 {
                fc.map(|c| c * fill[i])
            } else if border[i] {
                oc.map(|c| c * oa)
            } else {
                [0.0; 4]
            };
            let q = |v: f32| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u32;
            *px = q(out[3]) << 24 | q(out[0]) << 16 | q(out[1]) << 8 | q(out[2]);
        }
        canvas
    }

    /// Non-empty pixels relative to the anchor.
    fn pixels(c: &Canvas) -> Vec<(i32, i32, u32)> {
        let mut out = Vec::new();
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                let p = c.pixels[(y as u32 * c.width + x as u32) as usize];
                if p != 0 {
                    out.push((x - c.anchor.0, y - c.anchor.1, p));
                }
            }
        }
        out
    }

    #[test]
    fn layers_draw_what_valorant_settings_drew() {
        // A small deterministic pseudo-random walk over the settings.
        let mut seed = 0x2545_f491_u32;
        let mut next = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % n
        };
        let mut checked = 0;
        for _ in 0..600 {
            let lines = |next: &mut dyn FnMut(u32) -> u32| Lines {
                show: next(5) > 0,
                opacity: [1.0, 0.8, 0.35, 0.5][next(4) as usize],
                length: next(12),
                separate_vertical: next(2) == 0,
                vertical_length: next(12),
                thickness: next(6),
                offset: next(6),
            };
            let s = Settings {
                color: [Color::rgb(255, 255, 255), Color::rgb(0, 255, 0)][next(2) as usize],
                outlines: next(2) == 0,
                outline_opacity: [0.5, 1.0][next(2) as usize],
                outline_thickness: 1 + next(3),
                outline_color: Color::rgb(0, 0, 0),
                center_dot: next(2) == 0,
                center_dot_opacity: 1.0,
                center_dot_thickness: 1 + next(5),
                inner_lines: lines(&mut next),
                outer_lines: lines(&mut next),
            };
            // Where parts overlap, 0.2 kept the most opaque one while layers
            // blend, which only differs when see-through inner lines cross
            // the outer ones. The dot is opaque and on top.
            if s.inner_lines.show && s.outer_lines.show && s.inner_lines.opacity < 1.0 {
                continue;
            }
            let new = crate::layer::render(&s.to_layers());
            assert_eq!(pixels(&new), pixels(&legacy_render(&s)), "{s:?}");
            checked += 1;
        }
        assert!(checked > 100, "only {checked} cases compared");
    }

    #[test]
    fn layers_export_back_to_the_code() {
        for code in [
            "0;P",
            "0;P;c;5;h;0;0l;4;0o;0;0a;1;1b;0",
            "0;P;c;8;u;FF8000FF;d;1;z;3;0b;0;1b;0",
            "0;P;o;1;d;1;0l;3;0v;7;0g;1;0o;0;1t;4",
        ] {
            let s = import(code, Section::Primary).unwrap();
            let back = Settings::from_layers(&s.to_layers()).unwrap();
            assert_eq!(back.to_layers(), s.to_layers(), "{code}");
            let again = import(&export(&back), Section::Primary).unwrap();
            assert_eq!(again.to_layers(), s.to_layers(), "{code}");
        }
        // Free-form shapes have no Valorant code.
        let mut layers = Settings::default().to_layers();
        layers.push(Layer::new(Kind::Ring));
        assert!(Settings::from_layers(&layers).is_err());
        layers.pop();
        layers[0].rotation = 45.0;
        assert!(Settings::from_layers(&layers).is_err());
    }
}
