//! Shape layers and the CPU rasteriser that turns them into a [`Canvas`].
//!
//! A crosshair is a stack of layers, bottom first. Each layer is one shape
//! (a cross, line, rectangle, circle, ring, arc or triangle) with its own
//! position, size, rotation, colour, opacity and outline, and can be repeated
//! around the centre or mirrored.
//!
//! This runs in the editor only. The pixels are baked into `overlay.bin`, so
//! the overlay never rasterises anything and costs the same whatever the
//! crosshair is made of.
//!
//! # Coordinates
//!
//! Positions are in screen pixels relative to the centre of the anchor pixel
//! (the pixel on the screen centre); x grows to the right and y downwards.
//! Rotation is in degrees, clockwise. A pixel is filled when its centre is
//! inside the shape. A shape edge that lands exactly on pixel centres (an even
//! size centred on 0, for example) gives the pixel to the top and left, as
//! Valorant does, so a 2 px line centred on 0 covers pixels -1 and 0.
//!
//! That is what centres even sizes on the screen: the overlay puts the anchor
//! pixel at `width / 2`, so on an even-sized screen the true centre is the
//! anchor pixel's top-left corner, and pixels -1 and 0 sit evenly around it.
//! Odd sizes have a middle pixel, which lands on the anchor pixel, half a
//! pixel below and right of the true centre; no whole-pixel shape can do
//! better there.
//!
//! # Outlines
//!
//! A layer's outline is the shape grown by `outline` pixels, minus the shape.
//! Outlines are drawn behind every shape and never cover one, so touching
//! shapes share one clean outline, like Valorant's.

use crate::canvas::Canvas;
use crate::color::Color;
use serde::{Deserialize, Serialize};

/// The kinds of shape a layer can be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Four lines around the layer's centre, like Valorant's inner and
    /// outer lines. `width` is the left/right line length, `height` the
    /// top/bottom length, `gap` the space between the centre band and the
    /// lines (0 joins them into one solid cross).
    #[default]
    Cross,
    /// A straight line: `width` long and `thickness` wide, centred on the
    /// layer position. Rotate it to point anywhere.
    Line,
    /// A `width` × `height` rectangle; a small square is a dot.
    Rect,
    /// A filled circle `width` across.
    Circle,
    /// A circle outline `width` across and `thickness` wide.
    Ring,
    /// Part of a ring: `sweep` degrees of it, centred on "up" before
    /// rotation.
    Arc,
    /// A triangle `width` wide at the base and `height` tall, pointing up
    /// before rotation.
    Triangle,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Cross => "Cross",
            Kind::Line => "Line",
            Kind::Rect => "Rectangle",
            Kind::Circle => "Circle",
            Kind::Ring => "Ring",
            Kind::Arc => "Arc",
            Kind::Triangle => "Triangle",
        }
    }
}

/// One shape in a crosshair.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layer {
    /// Label shown in the editor's layer list.
    pub name: String,
    pub shape: Kind,
    pub visible: bool,
    /// Centre of the shape, relative to the screen centre, in pixels.
    pub x: f32,
    pub y: f32,
    /// Main size: length (cross, line), width (rectangle, triangle) or
    /// diameter (circle, ring, arc).
    pub width: f32,
    /// Second size: vertical line length (cross), height (rectangle,
    /// triangle).
    pub height: f32,
    /// Line width (cross, line, ring, arc).
    pub thickness: f32,
    /// Space between the centre and the lines (cross).
    pub gap: f32,
    /// How much of the circle an arc covers, in degrees.
    pub sweep: f32,
    /// Clockwise, in degrees.
    pub rotation: f32,
    /// Draw this many copies evenly spaced around the screen centre
    /// (1 = just this one, 4 = every 90°).
    pub copies: u32,
    /// Also draw it mirrored left-right / top-bottom across the centre.
    pub mirror_x: bool,
    pub mirror_y: bool,
    pub color: Color,
    /// 0.0 to 1.0.
    pub opacity: f32,
    /// Outline width in pixels (0 = no outline).
    pub outline: f32,
    pub outline_color: Color,
    pub outline_opacity: f32,
    /// Anti-aliased edges. Off keeps every pixel fully on or off, which
    /// looks sharpest for straight shapes.
    pub smooth: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Layer::new(Kind::Cross)
    }
}

/// Largest size or distance a layer may use, in pixels.
pub const MAX_SIZE: f32 = 500.0;
/// Largest outline width.
pub const MAX_OUTLINE: f32 = 16.0;
/// Most copies around the centre.
pub const MAX_COPIES: u32 = 12;
/// Largest distance from the anchor that gets drawn. Keeps the baked frame
/// (2 * MAX_RADIUS + 1 px square) well inside the overlay's 4096 px limit.
const MAX_RADIUS: i32 = 1100;

impl Layer {
    /// A new layer with sensible sizes for `kind`.
    pub fn new(kind: Kind) -> Self {
        let mut l = Layer {
            name: kind.label().into(),
            shape: kind,
            visible: true,
            x: 0.0,
            y: 0.0,
            width: 6.0,
            height: 6.0,
            thickness: 2.0,
            gap: 3.0,
            sweep: 90.0,
            rotation: 0.0,
            copies: 1,
            mirror_x: false,
            mirror_y: false,
            color: Color::rgb(255, 255, 255),
            opacity: 1.0,
            outline: 1.0,
            outline_color: Color::rgb(0, 0, 0),
            outline_opacity: 0.5,
            smooth: false,
        };
        match kind {
            Kind::Cross => {}
            Kind::Line => {
                l.width = 8.0;
                l.x = 8.0;
            }
            Kind::Rect => {
                l.name = "Dot".into();
                l.width = 2.0;
                l.height = 2.0;
            }
            Kind::Circle => {
                l.width = 4.0;
                l.smooth = true;
            }
            Kind::Ring => {
                l.width = 20.0;
                l.thickness = 1.5;
                l.smooth = true;
            }
            Kind::Arc => {
                l.width = 24.0;
                l.thickness = 2.0;
                l.smooth = true;
            }
            Kind::Triangle => {
                l.width = 6.0;
                l.height = 5.0;
                l.y = 10.0;
                l.smooth = true;
            }
        }
        l
    }

    /// The area this layer covers (its fill, without the outline), as
    /// `(min_x, min_y, max_x, max_y)` around the anchor pixel's centre, or
    /// `None` when it draws nothing.
    #[cfg(test)]
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let p = Prepared::new(self)?;
        let (hx, hy) = p.geo.half_extent(0.0);
        Some(p.world_box(hx, hy))
    }
}

fn finite(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { 0.0 }
}

/// A shape's size in its own frame, cleaned up.
#[derive(Clone, Copy)]
struct Geo {
    kind: Kind,
    w: f32,
    h: f32,
    t: f32,
    gap: f32,
    /// Half the arc's sweep, in radians.
    half_sweep: f32,
}

impl Geo {
    fn new(l: &Layer) -> Self {
        let size = |v: f32| finite(v, 0.0, MAX_SIZE);
        Geo {
            kind: l.shape,
            w: size(l.width),
            h: size(l.height),
            t: size(l.thickness),
            gap: size(l.gap),
            half_sweep: finite(l.sweep, 0.0, 360.0).to_radians() / 2.0,
        }
    }

    fn has_area(&self) -> bool {
        let Geo { w, h, t, .. } = *self;
        match self.kind {
            Kind::Cross => t > 0.0 && (w > 0.0 || h > 0.0),
            Kind::Line => w > 0.0 && t > 0.0,
            Kind::Rect | Kind::Triangle => w > 0.0 && h > 0.0,
            Kind::Circle => w > 0.0,
            Kind::Ring => w > 0.0 && t > 0.0,
            Kind::Arc => w > 0.0 && t > 0.0 && self.half_sweep > 0.0,
        }
    }

    /// Half width and height of the box around the shape grown by `g`.
    fn half_extent(&self, g: f32) -> (f32, f32) {
        let Geo { w, h, t, gap, .. } = *self;
        match self.kind {
            Kind::Cross => {
                let b = t / 2.0;
                let arm = |len: f32| if len > 0.0 { gap + len } else { 0.0 };
                (b + arm(w) + g, b + arm(h) + g)
            }
            Kind::Line => (w / 2.0 + g, t / 2.0 + g),
            Kind::Rect | Kind::Triangle => (w / 2.0 + g, h / 2.0 + g),
            Kind::Circle | Kind::Ring | Kind::Arc => (w / 2.0 + g, w / 2.0 + g),
        }
    }

    /// Whether the point `(x, y)` in the shape's own frame is inside the
    /// shape grown by `g` pixels. Edges count as inside.
    fn inside(&self, x: f32, y: f32, g: f32) -> bool {
        let Geo { w, h, t, gap, .. } = *self;
        match self.kind {
            Kind::Cross => {
                let b = t / 2.0;
                let (ax, ay) = (x.abs(), y.abs());
                let arm = |along: f32, across: f32, len: f32| {
                    len > 0.0
                        && across <= b + g
                        && along >= b + gap - g
                        && along <= b + gap + len + g
                };
                // With no gap the lines meet: fill the middle as well, so
                // the cross is solid instead of having a hole in it.
                let middle = gap == 0.0 && ax <= b + g && ay <= b + g;
                arm(ax, ay, w) || arm(ay, ax, h) || middle
            }
            Kind::Line => x.abs() <= w / 2.0 + g && y.abs() <= t / 2.0 + g,
            Kind::Rect => x.abs() <= w / 2.0 + g && y.abs() <= h / 2.0 + g,
            Kind::Circle => {
                let r = w / 2.0 + g;
                x * x + y * y <= r * r
            }
            Kind::Ring | Kind::Arc => {
                let outer = w / 2.0;
                let inner = (outer - t).max(0.0);
                let d2 = x * x + y * y;
                let ri = inner - g;
                let ro = outer + g;
                let in_ring = d2 <= ro * ro && (ri <= 0.0 || d2 >= ri * ri);
                if self.kind == Kind::Ring || !in_ring {
                    return in_ring;
                }
                if self.half_sweep >= std::f32::consts::PI {
                    return true;
                }
                // Angle from "up", clockwise.
                let a = x.atan2(-y).abs();
                let mid = ((outer + inner) / 2.0).max(0.5);
                a <= self.half_sweep + g / mid
            }
            Kind::Triangle => {
                // Tip at (0, -h/2), base corners at (±w/2, h/2). Each side
                // is pushed out by g; the box keeps sharp tips from growing
                // far past the shape.
                let (hw, hh) = (w / 2.0, h / 2.0);
                if x.abs() > hw + g || y < -hh - g || y > hh + g {
                    return false;
                }
                // Distance from the slanted sides, outward positive.
                let len = (h * h + hw * hw).sqrt();
                let side = (x.abs() * h - (y + hh) * hw) / len;
                side <= g
            }
        }
    }
}

/// A row-major 2x2 matrix and offset: `local = m * world + t`.
#[derive(Clone, Copy)]
struct Affine {
    m: [f32; 4],
    t: [f32; 2],
}

impl Affine {
    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.m[0] * x + self.m[1] * y + self.t[0],
            self.m[2] * x + self.m[3] * y + self.t[1],
        )
    }
}

/// Cosine and sine of `deg` degrees, exact for multiples of 90° so that
/// pixel edges don't wobble when a shape is turned on its side.
fn cos_sin(deg: f32) -> (f32, f32) {
    let d = deg.rem_euclid(360.0);
    let quarter = d / 90.0;
    if (quarter - quarter.round()).abs() < 1e-5 {
        return match quarter.round() as i32 % 4 {
            0 => (1.0, 0.0),
            1 => (0.0, 1.0),
            2 => (-1.0, 0.0),
            _ => (0.0, -1.0),
        };
    }
    let r = d.to_radians();
    (r.cos(), r.sin())
}

/// 2x2 row-major product `a * b`.
fn mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
    ]
}

fn transpose(a: [f32; 4]) -> [f32; 4] {
    [a[0], a[2], a[1], a[3]]
}

/// A layer ready to draw.
struct Prepared {
    geo: Geo,
    grow: f32,
    /// World-to-shape transforms, one per copy.
    copies: Vec<Affine>,
    /// Shape-to-world transforms, for bounding boxes.
    forward: Vec<Affine>,
    fill: [f32; 4],
    outline: [f32; 4],
    smooth: bool,
}

/// Premultiplied RGBA, 0.0 to 1.0, times `opacity`.
fn premul(c: Color, opacity: f32) -> [f32; 4] {
    let a = c.a as f32 / 255.0 * opacity;
    [
        c.r as f32 / 255.0 * a,
        c.g as f32 / 255.0 * a,
        c.b as f32 / 255.0 * a,
        a,
    ]
}

impl Prepared {
    fn new(l: &Layer) -> Option<Self> {
        let opacity = finite(l.opacity, 0.0, 1.0);
        let geo = Geo::new(l);
        if !l.visible || opacity <= 0.0 || l.color.a == 0 || !geo.has_area() {
            return None;
        }
        let pos = [
            finite(l.x, -MAX_SIZE, MAX_SIZE),
            finite(l.y, -MAX_SIZE, MAX_SIZE),
        ];
        let (c, s) = cos_sin(finite(l.rotation, -1e6, 1e6));
        // Shape frame to world: rotate (clockwise on screen, y down), move.
        let rot = [c, -s, s, c];
        let n = l.copies.clamp(1, MAX_COPIES);
        let mut mirrors = vec![[1.0, 0.0, 0.0, 1.0]];
        if l.mirror_x {
            mirrors.push([-1.0, 0.0, 0.0, 1.0]);
        }
        if l.mirror_y {
            let flipped: Vec<_> = mirrors
                .iter()
                .map(|&m| mul([1.0, 0.0, 0.0, -1.0], m))
                .collect();
            mirrors.extend(flipped);
        }
        let mut copies = Vec::new();
        let mut forward = Vec::new();
        for k in 0..n {
            let (ck, sk) = cos_sin(360.0 * k as f32 / n as f32);
            for &mirror in &mirrors {
                // world = C * (rot * local + pos), C = turn * mirror.
                let cm = mul([ck, -sk, sk, ck], mirror);
                let fm = mul(cm, rot);
                let ft = [
                    cm[0] * pos[0] + cm[1] * pos[1],
                    cm[2] * pos[0] + cm[3] * pos[1],
                ];
                forward.push(Affine { m: fm, t: ft });
                // Both parts are orthogonal, so the inverse is the transpose.
                let im = transpose(fm);
                copies.push(Affine {
                    m: im,
                    t: [
                        -(im[0] * ft[0] + im[1] * ft[1]),
                        -(im[2] * ft[0] + im[3] * ft[1]),
                    ],
                });
            }
        }
        let grow = finite(l.outline, 0.0, MAX_OUTLINE);
        let outline_opacity = finite(l.outline_opacity, 0.0, 1.0);
        Some(Prepared {
            geo,
            grow: if outline_opacity > 0.0 && l.outline_color.a > 0 {
                grow
            } else {
                0.0
            },
            copies,
            forward,
            fill: premul(l.color, opacity),
            outline: premul(l.outline_color, outline_opacity),
            smooth: l.smooth,
        })
    }

    /// World box around every copy of the shape-frame box `±hx, ±hy`.
    fn world_box(&self, hx: f32, hy: f32) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for f in &self.forward {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let (x, y) = f.apply(sx * hx, sy * hy);
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
        b
    }

    /// (inside the shape, inside the shape grown by the outline) at a world
    /// point.
    fn sample(&self, x: f32, y: f32) -> (bool, bool) {
        let mut grown = false;
        for c in &self.copies {
            let (lx, ly) = c.apply(x, y);
            if self.geo.inside(lx, ly, 0.0) {
                return (true, true);
            }
            grown = grown || (self.grow > 0.0 && self.geo.inside(lx, ly, self.grow));
        }
        (false, grown)
    }
}

/// Edges that fall exactly on a pixel centre belong to the pixel's top/left
/// side: samples are taken this far right of and below the centre.
const NUDGE: f32 = 1.0 / 1024.0;
/// Sub-samples per side for smooth layers.
const SUPERSAMPLE: usize = 4;

/// Rasterise `layers` (bottom first) into a square canvas anchored at its
/// centre pixel.
pub fn render(layers: &[Layer]) -> Canvas {
    let prepared: Vec<Prepared> = layers.iter().filter_map(Prepared::new).collect();

    let mut reach = 1.0f32;
    for p in &prepared {
        let (hx, hy) = p.geo.half_extent(p.grow);
        let b = p.world_box(hx, hy);
        reach = reach.max(-b.0).max(-b.1).max(b.2).max(b.3);
    }
    let r = (reach.ceil() as i32 + 1).clamp(1, MAX_RADIUS);
    let size = (2 * r + 1) as usize;

    let mut fill = vec![[0f32; 4]; size * size];
    let mut outline = vec![[0f32; 4]; size * size];
    // How much of each pixel any shape covers; outlines only show outside.
    let mut covered = vec![0f32; size * size];

    let offsets: Vec<f32> = (0..SUPERSAMPLE)
        .map(|k| (k as f32 + 0.5) / SUPERSAMPLE as f32 - 0.5)
        .collect();
    for p in &prepared {
        let (hx, hy) = p.geo.half_extent(p.grow);
        let b = p.world_box(hx, hy);
        let lo = |v: f32| ((v.floor() as i32) - 1).clamp(-r, r);
        let hi = |v: f32| ((v.ceil() as i32) + 1).clamp(-r, r);
        for py in lo(b.1)..=hi(b.3) {
            for px in lo(b.0)..=hi(b.2) {
                let (cf, cg) = if p.smooth {
                    let (mut nf, mut ng) = (0u32, 0u32);
                    for &oy in &offsets {
                        for &ox in &offsets {
                            let (f, g) = p.sample(px as f32 + ox, py as f32 + oy);
                            nf += f as u32;
                            ng += g as u32;
                        }
                    }
                    let n = (SUPERSAMPLE * SUPERSAMPLE) as f32;
                    (nf as f32 / n, ng as f32 / n)
                } else {
                    let (f, g) = p.sample(px as f32 + NUDGE, py as f32 + NUDGE);
                    (f as u8 as f32, g as u8 as f32)
                };
                if cf == 0.0 && cg == 0.0 {
                    continue;
                }
                let i = (py + r) as usize * size + (px + r) as usize;
                if cf > 0.0 {
                    let dst = &mut fill[i];
                    let src = p.fill.map(|c| c * cf);
                    let keep = 1.0 - src[3];
                    for ch in 0..4 {
                        dst[ch] = src[ch] + dst[ch] * keep;
                    }
                    covered[i] = covered[i].max(cf);
                }
                if p.grow > 0.0 && cg > 0.0 {
                    let o = p.outline.map(|c| c * cg);
                    if o[3] > outline[i][3] {
                        outline[i] = o;
                    }
                }
            }
        }
    }

    let mut canvas = Canvas::new(size as u32, size as u32, (r, r));
    let q = |v: f32| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u32;
    for (i, px) in canvas.pixels.iter_mut().enumerate() {
        let keep = 1.0 - covered[i];
        let c: [f32; 4] = std::array::from_fn(|ch| fill[i][ch] + outline[i][ch] * keep);
        *px = q(c[3]) << 24 | q(c[0]) << 16 | q(c[1]) << 8 | q(c[2]);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(c: &Canvas, x: i32, y: i32) -> u32 {
        let (ax, ay) = c.anchor;
        let (x, y) = (x + ax, y + ay);
        if x < 0 || y < 0 || x >= c.width as i32 || y >= c.height as i32 {
            return 0;
        }
        c.pixels[(y as u32 * c.width + x as u32) as usize] >> 24
    }

    fn solid(kind: Kind) -> Layer {
        Layer {
            outline: 0.0,
            smooth: false,
            ..Layer::new(kind)
        }
    }

    /// Pixels with any alpha, relative to the anchor.
    fn lit(c: &Canvas) -> Vec<(i32, i32)> {
        let (ax, ay) = c.anchor;
        let mut out = Vec::new();
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                if c.pixels[(y as u32 * c.width + x as u32) as usize] >> 24 > 0 {
                    out.push((x - ax, y - ay));
                }
            }
        }
        out
    }

    #[test]
    fn even_sizes_lean_top_left() {
        let c = render(&[Layer {
            width: 2.0,
            height: 2.0,
            ..solid(Kind::Rect)
        }]);
        assert_eq!(lit(&c), vec![(-1, -1), (0, -1), (-1, 0), (0, 0)]);
        let c = render(&[Layer {
            width: 3.0,
            height: 1.0,
            ..solid(Kind::Rect)
        }]);
        assert_eq!(lit(&c), vec![(-1, 0), (0, 0), (1, 0)]);
    }

    #[test]
    fn line_copies_make_a_symmetric_cross() {
        // A 2 px line from x = 4 to 10, repeated four times, is the same as
        // a cross with a 3 px gap.
        let line = Layer {
            x: 7.0,
            width: 6.0,
            thickness: 2.0,
            copies: 4,
            ..solid(Kind::Line)
        };
        let cross = Layer {
            width: 6.0,
            height: 6.0,
            thickness: 2.0,
            gap: 3.0,
            ..solid(Kind::Cross)
        };
        let a = lit(&render(&[line]));
        assert_eq!(a, lit(&render(&[cross])));
        assert!(a.contains(&(4, -1)) && a.contains(&(9, 0)) && !a.contains(&(10, 0)));
        assert!(a.contains(&(-10, 0)) && !a.contains(&(-11, 0)));
        assert!(a.contains(&(-1, 9)) && a.contains(&(0, -10)));
    }

    #[test]
    fn rotation_by_quarter_turns_is_exact() {
        let base = Layer {
            x: 5.0,
            y: 2.0,
            width: 5.0,
            thickness: 3.0,
            ..solid(Kind::Line)
        };
        let a = lit(&render(std::slice::from_ref(&base)));
        let b = lit(&render(&[Layer {
            rotation: 360.0,
            ..base.clone()
        }]));
        assert_eq!(a, b);
        let turned = lit(&render(&[Layer {
            rotation: 90.0,
            ..base
        }]));
        assert_eq!(turned.len(), a.len());
        assert!(turned.contains(&(5, 0)) && turned.contains(&(5, 4)));
    }

    #[test]
    fn mirror_copies() {
        let dot = Layer {
            x: 5.0,
            y: -3.0,
            width: 1.0,
            height: 1.0,
            mirror_x: true,
            mirror_y: true,
            ..solid(Kind::Rect)
        };
        let mut a = lit(&render(&[dot]));
        a.sort();
        assert_eq!(a, vec![(-5, -3), (-5, 3), (5, -3), (5, 3)]);
    }

    #[test]
    fn circle_and_ring() {
        let c = render(&[Layer {
            width: 9.0,
            ..solid(Kind::Circle)
        }]);
        assert_eq!(alpha(&c, 0, 0), 255);
        assert_eq!(alpha(&c, 4, 0), 255);
        assert_eq!(alpha(&c, 5, 0), 0);
        assert_eq!(alpha(&c, 4, 3), 0);
        assert_eq!(alpha(&c, 3, 3), 255);
        let ring = render(&[Layer {
            width: 21.0,
            thickness: 2.0,
            ..solid(Kind::Ring)
        }]);
        assert_eq!(alpha(&ring, 0, 0), 0);
        assert_eq!(alpha(&ring, 10, 0), 255);
        assert_eq!(alpha(&ring, 0, -9), 255);
        assert_eq!(alpha(&ring, 7, 0), 0);
    }

    #[test]
    fn arc_covers_its_sweep_only() {
        let c = render(&[Layer {
            width: 20.0,
            thickness: 2.0,
            sweep: 90.0,
            ..solid(Kind::Arc)
        }]);
        assert_eq!(alpha(&c, 0, -9), 255, "top");
        assert_eq!(alpha(&c, 0, 9), 0, "bottom");
        assert_eq!(alpha(&c, 9, 0), 0, "right");
        let turned = render(&[Layer {
            width: 20.0,
            thickness: 2.0,
            sweep: 90.0,
            rotation: 90.0,
            ..solid(Kind::Arc)
        }]);
        assert_eq!(alpha(&turned, 9, 0), 255, "rotated to the right");
    }

    #[test]
    fn triangle_points_up() {
        let c = render(&[Layer {
            y: 0.0,
            width: 9.0,
            height: 9.0,
            ..solid(Kind::Triangle)
        }]);
        assert_eq!(alpha(&c, 0, -4), 255, "tip");
        assert_eq!(alpha(&c, 3, -3), 0);
        assert_eq!(alpha(&c, 4, 4), 255, "base corner");
    }

    #[test]
    fn outline_goes_around_and_behind() {
        let dot = Layer {
            width: 1.0,
            height: 1.0,
            outline: 1.0,
            outline_opacity: 1.0,
            ..solid(Kind::Rect)
        };
        let c = render(std::slice::from_ref(&dot));
        let px = |x: i32, y: i32| {
            c.pixels[((y + c.anchor.1) as u32 * c.width + (x + c.anchor.0) as u32) as usize]
        };
        assert_eq!(px(0, 0), 0xFFFFFFFF);
        assert_eq!(px(1, 1), 0xFF000000);
        assert_eq!(px(2, 0), 0);

        // A second shape next to the dot: its outline doesn't cover the dot.
        let beside = Layer {
            x: 2.0,
            ..dot.clone()
        };
        let c = render(&[dot, beside]);
        let px = |x: i32, y: i32| {
            c.pixels[((y + c.anchor.1) as u32 * c.width + (x + c.anchor.0) as u32) as usize]
        };
        assert_eq!(px(0, 0), 0xFFFFFFFF);
        assert_eq!(px(2, 0), 0xFFFFFFFF);
        assert_eq!(px(1, 0), 0xFF000000);
    }

    #[test]
    fn upper_layers_go_on_top() {
        let red = Layer {
            width: 3.0,
            height: 3.0,
            color: Color::rgb(255, 0, 0),
            ..solid(Kind::Rect)
        };
        let blue = Layer {
            width: 1.0,
            height: 1.0,
            color: Color::rgb(0, 0, 255),
            opacity: 0.5,
            ..solid(Kind::Rect)
        };
        let c = render(&[red, blue]);
        let px = c.pixels[(c.anchor.1 as u32 * c.width + c.anchor.0 as u32) as usize];
        // Half blue over red.
        assert_eq!(px, 0xFF80_0080);
    }

    #[test]
    fn smooth_edges_are_partial() {
        let c = render(&[Layer {
            width: 7.0,
            smooth: true,
            ..solid(Kind::Circle)
        }]);
        assert_eq!(alpha(&c, 0, 0), 255);
        let edge = alpha(&c, 3, 1);
        assert!(edge > 0 && edge < 255, "{edge}");
    }

    #[test]
    fn hidden_and_empty_layers_draw_nothing() {
        let hidden = Layer {
            visible: false,
            ..Layer::new(Kind::Cross)
        };
        let empty = Layer {
            width: 0.0,
            ..Layer::new(Kind::Circle)
        };
        let c = render(&[hidden, empty]);
        assert!(c.pixels.iter().all(|&p| p == 0));
        assert_eq!((c.width, c.height), (5, 5));
    }

    #[test]
    fn huge_and_broken_values_are_clamped() {
        let c = render(&[Layer {
            x: f32::INFINITY,
            y: -1e30,
            width: f32::MAX,
            height: f32::NAN,
            thickness: 1e9,
            gap: 1e9,
            outline: 1e9,
            copies: u32::MAX,
            rotation: f32::NAN,
            ..Layer::new(Kind::Cross)
        }]);
        assert!(c.width < crosslay_format::MAX_SIDE as u32);
        let r = render(&[Layer {
            width: 1e9,
            thickness: 1e9,
            ..Layer::new(Kind::Ring)
        }]);
        assert!(r.width < crosslay_format::MAX_SIDE as u32);
    }

    #[test]
    fn bounds_cover_the_copies() {
        let b = Layer {
            x: 7.0,
            width: 6.0,
            thickness: 2.0,
            copies: 4,
            ..solid(Kind::Line)
        }
        .bounds()
        .unwrap();
        assert_eq!(b, (-10.0, -10.0, 10.0, 10.0));
        assert!(
            Layer {
                visible: false,
                ..Layer::new(Kind::Rect)
            }
            .bounds()
            .is_none()
        );
    }
}
