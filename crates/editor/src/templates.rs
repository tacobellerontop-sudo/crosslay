//! Ready-made crosshairs to start from, shown as thumbnails in the editor.

use crate::color::Color;
use crate::layer::{Kind, Layer};
use crate::valorant::{Lines, Settings};

/// A starting point: a name and its layers, bottom first.
pub struct Template {
    pub name: &'static str,
    pub layers: Vec<Layer>,
}

fn plain(kind: Kind) -> Layer {
    Layer {
        outline: 1.0,
        outline_opacity: 0.5,
        ..Layer::new(kind)
    }
}

/// Every template, in gallery order.
pub fn all() -> Vec<Template> {
    let classic = Settings::default().to_layers();
    let cross = Settings {
        inner_lines: Lines {
            opacity: 1.0,
            length: 5,
            vertical_length: 5,
            offset: 0,
            ..Lines::INNER
        },
        outer_lines: Lines::HIDDEN,
        ..Settings::default()
    }
    .to_layers();
    let gap_dot = Settings {
        center_dot: true,
        inner_lines: Lines {
            opacity: 1.0,
            length: 4,
            vertical_length: 4,
            offset: 2,
            ..Lines::INNER
        },
        outer_lines: Lines::HIDDEN,
        ..Settings::default()
    }
    .to_layers();
    let dot = vec![Layer {
        name: "Dot".into(),
        width: 3.0,
        height: 3.0,
        ..plain(Kind::Rect)
    }];
    let circle = vec![
        Layer {
            name: "Ring".into(),
            width: 17.0,
            thickness: 1.5,
            ..plain(Kind::Ring)
        },
        Layer {
            name: "Dot".into(),
            width: 3.0,
            ..plain(Kind::Circle)
        },
    ];
    let t_shape = vec![
        Layer {
            name: "Sides".into(),
            width: 6.0,
            height: 0.0,
            thickness: 2.0,
            gap: 3.0,
            ..plain(Kind::Cross)
        },
        Layer {
            name: "Bottom".into(),
            x: 0.0,
            y: 7.0,
            width: 6.0,
            thickness: 2.0,
            rotation: 90.0,
            ..plain(Kind::Line)
        },
    ];
    let chevron = vec![
        Layer {
            name: "Chevron".into(),
            x: -3.0,
            y: 3.0,
            width: 8.0,
            thickness: 1.5,
            rotation: -45.0,
            mirror_x: true,
            smooth: true,
            ..plain(Kind::Line)
        },
        Layer {
            name: "Dot".into(),
            width: 1.0,
            height: 1.0,
            ..plain(Kind::Rect)
        },
    ];
    let brackets = vec![
        Layer {
            name: "Brackets".into(),
            width: 22.0,
            thickness: 2.0,
            sweep: 60.0,
            rotation: 90.0,
            mirror_x: true,
            smooth: true,
            ..plain(Kind::Arc)
        },
        Layer {
            name: "Dot".into(),
            width: 2.0,
            height: 2.0,
            ..plain(Kind::Rect)
        },
    ];
    vec![
        Template {
            name: "Classic",
            layers: classic,
        },
        Template {
            name: "Cross",
            layers: cross,
        },
        Template {
            name: "Gap + dot",
            layers: gap_dot,
        },
        Template {
            name: "Dot",
            layers: dot,
        },
        Template {
            name: "Circle",
            layers: circle,
        },
        Template {
            name: "T",
            layers: t_shape,
        },
        Template {
            name: "Chevron",
            layers: chevron,
        },
        Template {
            name: "Brackets",
            layers: brackets,
        },
        Template {
            name: "Blank",
            layers: Vec::new(),
        },
    ]
}

/// `layers` with every fill recoloured to `color` (outlines keep theirs).
pub fn recolor(mut layers: Vec<Layer>, color: Color) -> Vec<Layer> {
    for l in &mut layers {
        l.color = color;
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_but_blank_draws_something() {
        for t in all() {
            let c = crate::layer::render(&t.layers);
            let lit = c.pixels.iter().filter(|&&p| p >> 24 > 0).count();
            assert_eq!(lit == 0, t.name == "Blank", "{}", t.name);
            assert!(c.width < 80, "{} is {} px wide", t.name, c.width);
        }
    }
}
