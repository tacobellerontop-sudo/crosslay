//! The drawing side of the editor: a big zoomed canvas with tools to draw
//! shapes straight onto it, starter crosshairs, a short list of settings
//! for the selected shape, and undo.

use super::*;
use crate::layer::MAX_COPIES;
use crate::templates;

/// What clicking and dragging on the canvas does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    /// Click a shape to select it, drag to move it.
    Select,
    /// Click to place a square dot.
    Dot,
    /// Drag from one end to the other.
    Line,
    /// Drag from the centre outwards.
    Circle,
    /// Drag from the centre outwards.
    Ring,
}

impl Tool {
    const ALL: [Tool; 5] = [
        Tool::Select,
        Tool::Dot,
        Tool::Line,
        Tool::Circle,
        Tool::Ring,
    ];

    fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Dot => "Dot",
            Tool::Line => "Line",
            Tool::Circle => "Circle",
            Tool::Ring => "Ring",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Tool::Select => "Click a shape to select it, drag to move it (S)",
            Tool::Dot => "Click to place a dot (D)",
            Tool::Line => "Drag from one end of the line to the other (L)",
            Tool::Circle => "Drag from the middle outwards (C)",
            Tool::Ring => "Drag from the middle outwards (R)",
        }
    }

    fn key(self) -> egui::Key {
        match self {
            Tool::Select => egui::Key::S,
            Tool::Dot => egui::Key::D,
            Tool::Line => egui::Key::L,
            Tool::Circle => egui::Key::C,
            Tool::Ring => egui::Key::R,
        }
    }
}

/// How a new shape is repeated around the screen centre.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Symmetry {
    /// Four copies, a quarter turn apart: draw one arm, get a cross.
    Four,
    /// A mirrored copy on the other side.
    LeftRight,
    Off,
}

impl Symmetry {
    const ALL: [Symmetry; 3] = [Symmetry::Four, Symmetry::LeftRight, Symmetry::Off];

    fn label(self) -> &'static str {
        match self {
            Symmetry::Four => "4-way",
            Symmetry::LeftRight => "Left-right",
            Symmetry::Off => "Off",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Symmetry::Four => "Four copies around the centre: draw one arm, get a cross",
            Symmetry::LeftRight => "A mirrored copy on the other side",
            Symmetry::Off => "Just what you draw",
        }
    }

    fn apply(self, l: &mut Layer) {
        l.copies = if self == Symmetry::Four { 4 } else { 1 };
        l.mirror_x = self == Symmetry::LeftRight;
        l.mirror_y = false;
    }

    fn of(l: &Layer) -> Option<Symmetry> {
        match (l.copies.clamp(1, MAX_COPIES), l.mirror_x, l.mirror_y) {
            (4, false, false) => Some(Symmetry::Four),
            (1, true, false) => Some(Symmetry::LeftRight),
            (1, false, false) => Some(Symmetry::Off),
            _ => None,
        }
    }
}

/// Drawing state that isn't saved.
pub(super) struct Studio {
    tool: Tool,
    symmetry: Symmetry,
    /// Colour, line width and outline for new shapes.
    color: Color,
    size: f32,
    outline: bool,
    /// The shape being drawn by the current drag, and where the drag began
    /// (in whole pixels from the centre).
    drawing: Option<(usize, Vec2)>,
    grid: bool,
    /// Earlier and undone configs.
    undo: Vec<Config>,
    redo: Vec<Config>,
    /// Inside a run of changes that undo as one (a drag, typing).
    in_burst: bool,
    last_edit: Instant,
    gallery: Vec<(templates::Template, TextureHandle)>,
}

impl Default for Studio {
    fn default() -> Self {
        Studio {
            tool: Tool::Line,
            symmetry: Symmetry::Four,
            color: Color::rgb(255, 255, 255),
            size: 2.0,
            outline: true,
            drawing: None,
            grid: true,
            undo: Vec::new(),
            redo: Vec::new(),
            in_burst: false,
            last_edit: Instant::now(),
            gallery: Vec::new(),
        }
    }
}

/// Most undo steps kept.
const UNDO_LIMIT: usize = 200;
/// Changes closer together than this undo as one step.
const BURST: Duration = Duration::from_millis(600);

impl Menu {
    /// Record undo history. Call once per frame, before `prev` is updated.
    pub(super) fn track_history(&mut self, ctx: &egui::Context) {
        let s = &mut self.studio;
        let busy = ctx.input(|i| i.pointer.any_down());
        if self.cfg != self.prev {
            if !s.in_burst {
                s.undo.push(self.prev.clone());
                if s.undo.len() > UNDO_LIMIT {
                    s.undo.remove(0);
                }
                s.redo.clear();
            }
            s.in_burst = true;
            s.last_edit = Instant::now();
        } else if s.in_burst && !busy && s.last_edit.elapsed() >= BURST {
            s.in_burst = false;
        } else if s.in_burst {
            ctx.request_repaint_after(BURST);
        }
    }

    fn undo(&mut self) {
        if let Some(c) = self.studio.undo.pop() {
            self.studio.redo.push(std::mem::replace(&mut self.cfg, c));
            self.after_history_jump();
        }
    }

    fn redo(&mut self) {
        if let Some(c) = self.studio.redo.pop() {
            self.studio.undo.push(std::mem::replace(&mut self.cfg, c));
            self.after_history_jump();
        }
    }

    fn after_history_jump(&mut self) {
        self.prev = self.cfg.clone();
        self.changed_at = Some(Instant::now());
        self.studio.in_burst = false;
        self.studio.drawing = None;
        self.selected = self.selected.min(self.cfg.crosshairs.len() - 1);
        let n = self.target().layers.len();
        self.layer = self.layer.min(n.saturating_sub(1));
    }

    /// Keyboard shortcuts, unless a text box has the keyboard.
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let redo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let redo2 = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        if ctx.input_mut(|i| i.consume_shortcut(&redo) || i.consume_shortcut(&redo2)) {
            self.redo();
        } else if ctx.input_mut(|i| i.consume_shortcut(&undo)) {
            self.undo();
        }
        let (delete, tool) = ctx.input(|i| {
            let delete = i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace);
            let tool = if i.modifiers.any() {
                None
            } else {
                Tool::ALL.into_iter().find(|t| i.key_pressed(t.key()))
            };
            (delete, tool)
        });
        if delete {
            self.delete_layer();
        }
        if let Some(t) = tool {
            self.studio.tool = t;
        }
        // Arrow keys nudge the selected shape a pixel.
        let nudge = ctx.input(|i| {
            let mut v = Vec2::ZERO;
            for (k, d) in [
                (Key::ArrowLeft, Vec2::new(-1.0, 0.0)),
                (Key::ArrowRight, Vec2::new(1.0, 0.0)),
                (Key::ArrowUp, Vec2::new(0.0, -1.0)),
                (Key::ArrowDown, Vec2::new(0.0, 1.0)),
            ] {
                if i.key_pressed(k) {
                    v += d;
                }
            }
            v
        });
        if nudge != Vec2::ZERO {
            let i = self.layer;
            if let Some(l) = self.target().layers.get_mut(i) {
                l.x += nudge.x;
                l.y += nudge.y;
            }
        }
    }

    fn delete_layer(&mut self) {
        let i = self.layer;
        let layers = &mut self.target().layers;
        if i < layers.len() {
            layers.remove(i);
            self.layer = i.saturating_sub(1);
        }
    }

    /// A new shape of `kind` in the drawing colour and style.
    pub(super) fn new_shape(&mut self, kind: Kind) -> Layer {
        let s = &self.studio;
        let mut l = Layer::new(kind);
        l.x = 0.0;
        l.y = 0.0;
        l.color = s.color;
        l.outline = if s.outline { 1.0 } else { 0.0 };
        s.symmetry.apply(&mut l);
        match kind {
            Kind::Rect => {
                l.width = s.size;
                l.height = s.size;
            }
            Kind::Line | Kind::Ring | Kind::Arc | Kind::Cross => l.thickness = s.size,
            _ => {}
        }
        l.name = unique_layer_name(&self.target().layers, &l.name);
        l
    }

    pub(super) fn push_shape(&mut self, l: Layer) -> usize {
        let layers = &mut self.target().layers;
        layers.push(l);
        self.layer = layers.len() - 1;
        self.layer
    }

    // ---- Layout -------------------------------------------------------------

    /// Everything in the middle of the window.
    pub(super) fn canvas_area(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        self.right_click_tabs(ui);
        ui.add_space(4.0);
        self.gallery(ui);
        ui.add_space(4.0);
        self.toolbar(ui);
        ui.add_space(4.0);
        self.canvas(ui);
    }

    /// Choose between the normal and the right-click crosshair.
    fn right_click_tabs(&mut self, ui: &mut egui::Ui) {
        let has = self.current().right_click.is_some();
        ui.horizontal(|ui| {
            let mut alt = self.editing_right_click && has;
            ui.selectable_value(&mut alt, false, RichText::new("Normal").strong());
            if has {
                ui.selectable_value(&mut alt, true, RichText::new("Right mouse held").strong())
                    .on_hover_text(match self.cfg.right_click {
                        RightClickMode::Hold => "Shown while the right mouse button is held",
                        RightClickMode::Toggle => "Each right click switches to it and back",
                    });
                if alt != self.editing_right_click {
                    self.editing_right_click = alt;
                    self.sync_image_input();
                }
                ui.separator();
                if ui
                    .small_button("Copy from normal")
                    .on_hover_text("Make the right-click crosshair the same as the normal one")
                    .clicked()
                {
                    let copy = self.current().right_click_template();
                    self.current().right_click = Some(Box::new(copy));
                    self.sync_image_input();
                }
                if ui
                    .small_button("Remove")
                    .on_hover_text("No right-click crosshair: the overlay ignores the mouse")
                    .clicked()
                {
                    self.current().right_click = None;
                    self.editing_right_click = false;
                    self.sync_image_input();
                }
            } else if ui
                .small_button("+ Right-click crosshair")
                .on_hover_text(
                    "Show a different crosshair while the right mouse button is held, \
                     e.g. when aiming down sights",
                )
                .clicked()
            {
                let copy = self.current().right_click_template();
                self.current().right_click = Some(Box::new(copy));
                self.editing_right_click = true;
                self.sync_image_input();
            }
        });
    }

    /// Thumbnails of the starter crosshairs.
    fn gallery(&mut self, ui: &mut egui::Ui) {
        if self.studio.gallery.is_empty() {
            for t in templates::all() {
                let tex = ui.ctx().load_texture(
                    format!("template-{}", t.name),
                    to_color_image(&crate::layer::render(&t.layers)),
                    TextureOptions::NEAREST,
                );
                self.studio.gallery.push((t, tex));
            }
        }
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Start from").weak())
                .on_hover_text("Replaces the shapes with a ready-made crosshair in your colour");
            for (i, (t, tex)) in self.studio.gallery.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(58.0, 58.0), Sense::click());
                let hovered = resp.hovered();
                let painter = ui.painter_at(rect);
                painter.rect_filled(
                    rect,
                    4.0,
                    if hovered {
                        Color32::from_rgb(70, 78, 70)
                    } else {
                        Color32::from_rgb(52, 58, 52)
                    },
                );
                // Twice real size, centred above the label.
                let s = 2.0 / ui.ctx().pixels_per_point();
                let size = tex.size_vec2() * s;
                let c = rect.center() - Vec2::new(0.0, 6.0);
                painter.image(
                    tex.id(),
                    Rect::from_center_size(c, size),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                painter.text(
                    Pos2::new(rect.center().x, rect.bottom() - 3.0),
                    egui::Align2::CENTER_BOTTOM,
                    t.name,
                    egui::FontId::proportional(10.0),
                    Color32::from_gray(210),
                );
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    picked = Some(i);
                }
            }
        });
        if let Some(i) = picked {
            let layers = self.studio.gallery[i].0.layers.clone();
            let color = self.studio.color;
            let ch = self.target();
            ch.layers = templates::recolor(layers, color);
            ch.image.clear();
            self.sync_image_input();
            let name = self.studio.gallery[i].0.name;
            self.set_status(format!("Started from \"{name}\" (Ctrl+Z to undo)"), false);
        }
    }

    /// Tools, symmetry and the style for new shapes.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let s = &mut self.studio;
            for t in Tool::ALL {
                ui.selectable_value(&mut s.tool, t, t.label())
                    .on_hover_text(t.hint());
            }
            ui.separator();
            ui.label("Symmetry");
            for m in Symmetry::ALL {
                ui.selectable_value(&mut s.symmetry, m, m.label())
                    .on_hover_text(m.hint());
            }
            ui.separator();
            color_edit(ui, &mut s.color).on_hover_text("Colour for new shapes");
            ui.add(
                egui::DragValue::new(&mut s.size)
                    .range(0.5..=20.0)
                    .speed(0.1)
                    .max_decimals(1)
                    .prefix("width ")
                    .suffix(" px"),
            )
            .on_hover_text("Line width (and dot size) for new shapes");
            ui.checkbox(&mut s.outline, "Outline")
                .on_hover_text("Give new shapes a thin dark outline");
            ui.separator();
            if ui
                .add_enabled(!self.studio.undo.is_empty(), egui::Button::new("Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(!self.studio.redo.is_empty(), egui::Button::new("Redo"))
                .on_hover_text("Ctrl+Y")
                .clicked()
            {
                self.redo();
            }
        });
    }

    /// The zoomed crosshair you draw on.
    fn canvas(&mut self, ui: &mut egui::Ui) {
        self.update_preview(ui.ctx());
        let footer = 26.0;
        let size = Vec2::new(
            ui.available_width(),
            (ui.available_height() - footer).max(160.0),
        );
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        paint_background(&painter, rect, self.background);
        // 1× = real on-screen size, regardless of the editor's UI scaling.
        let px = self.zoom as f32 / ui.ctx().pixels_per_point();
        let center = rect.center();
        let uses_image = self.target().uses_image();

        // Pixel grid and centre lines.
        if self.studio.grid && px >= 5.0 {
            let stroke = Stroke::new(1.0, Color32::from_black_alpha(28));
            let (nx, ny) = (
                (rect.width() / 2.0 / px).ceil() as i32 + 1,
                (rect.height() / 2.0 / px).ceil() as i32 + 1,
            );
            for k in -nx..=nx {
                let x = center.x + (k as f32 + 0.5) * px;
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    stroke,
                );
            }
            for k in -ny..=ny {
                let y = center.y + (k as f32 + 0.5) * px;
                painter.line_segment(
                    [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                    stroke,
                );
            }
        }
        let guide = Stroke::new(1.0, Color32::from_white_alpha(40));
        painter.line_segment(
            [
                Pos2::new(center.x, rect.top()),
                Pos2::new(center.x, rect.bottom()),
            ],
            guide,
        );
        painter.line_segment(
            [
                Pos2::new(rect.left(), center.y),
                Pos2::new(rect.right(), center.y),
            ],
            guide,
        );

        let mut info = None;
        if let Some(p) = &self.preview {
            if let Some(tex) = &p.texture {
                let min = Pos2::new(
                    center.x - (p.anchor.0 as f32 + 0.5) * px,
                    center.y - (p.anchor.1 as f32 + 0.5) * px,
                );
                let r = Rect::from_min_size(
                    min,
                    Vec2::new(p.size[0] as f32 * px, p.size[1] as f32 * px),
                );
                painter.image(
                    tex.id(),
                    r,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            info = Some(match &p.error {
                Some(e) => {
                    RichText::new(format!("Image error: {e}")).color(ui.visuals().error_fg_color)
                }
                None => RichText::new(format!("{} × {} px", p.size[0], p.size[1])).weak(),
            });
        }

        if uses_image {
            painter.text(
                rect.left_top() + Vec2::new(8.0, 8.0),
                egui::Align2::LEFT_TOP,
                "Showing your PNG. Remove it under Image to draw shapes.",
                egui::FontId::proportional(13.0),
                Color32::WHITE,
            );
        } else {
            self.canvas_input(ui, &resp, &painter, center, px);
        }

        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 110.0;
            ui.label("Zoom");
            ui.add(egui::Slider::new(&mut self.zoom, 1..=20).suffix("×"))
                .on_hover_text("1× is the real size on screen");
            ui.checkbox(&mut self.studio.grid, "Grid");
            ui.separator();
            for (bg, label) in [
                (Background::Scene, "Scene"),
                (Background::Dark, "Dark"),
                (Background::Light, "Light"),
                (Background::Checker, "Checker"),
            ] {
                ui.selectable_value(&mut self.background, bg, label);
            }
            if let Some(info) = info {
                ui.separator();
                ui.label(info);
            }
        });
    }

    /// Drawing, selecting and moving shapes with the mouse.
    fn canvas_input(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        painter: &egui::Painter,
        center: Pos2,
        px: f32,
    ) {
        let to_world = |p: Pos2| (p - center) / px;
        let snap = |v: Vec2| Vec2::new(v.x.round() + 0.0, v.y.round() + 0.0);
        let tool = self.studio.tool;

        // Where a click would land.
        if let Some(hover) = resp.hover_pos()
            && tool != Tool::Select
        {
            let w = snap(to_world(hover));
            let r = Rect::from_center_size(center + w * px, Vec2::splat(px));
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 220, 80, 200)),
                StrokeKind::Inside,
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }

        let press = ui
            .input(|i| i.pointer.press_origin())
            .map(|p| snap(to_world(p)));
        let pointer = resp.interact_pointer_pos().map(|p| snap(to_world(p)));
        match tool {
            Tool::Select => {
                let under = |layers: &[Layer], at: Vec2| {
                    layers.iter().rposition(|l| {
                        l.bounds().is_some_and(|b| {
                            at.x >= b.0 - 0.5
                                && at.x <= b.2 + 0.5
                                && at.y >= b.1 - 0.5
                                && at.y <= b.3 + 0.5
                        })
                    })
                };
                if (resp.clicked() || resp.drag_started())
                    && let Some(at) = resp.interact_pointer_pos().map(to_world)
                {
                    if let Some(i) = under(&self.target().layers, at) {
                        self.layer = i;
                    }
                    self.drag_rest = Vec2::ZERO;
                }
                if resp.dragged() {
                    self.drag_rest += resp.drag_delta() / px;
                    let step = Vec2::new(self.drag_rest.x.trunc(), self.drag_rest.y.trunc());
                    self.drag_rest -= step;
                    let i = self.layer;
                    if let Some(l) = self.target().layers.get_mut(i) {
                        l.x += step.x;
                        l.y += step.y;
                    }
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(if resp.dragged() {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    });
                }
            }
            Tool::Dot => {
                if resp.clicked()
                    && let Some(at) = pointer
                {
                    let mut l = self.new_shape(Kind::Rect);
                    l.name = unique_layer_name(&self.target().layers, "Dot");
                    l.x = at.x;
                    l.y = at.y;
                    self.push_shape(l);
                }
            }
            Tool::Line | Tool::Circle | Tool::Ring => {
                let kind = match tool {
                    Tool::Line => Kind::Line,
                    Tool::Circle => Kind::Circle,
                    _ => Kind::Ring,
                };
                if resp.drag_started()
                    && let Some(start) = press
                {
                    let l = self.new_shape(kind);
                    let i = self.push_shape(l);
                    self.studio.drawing = Some((i, start));
                }
                if resp.clicked()
                    && let Some(at) = pointer
                {
                    // A click without a drag: a small shape of that kind.
                    let mut l = self.new_shape(kind);
                    let end = match tool {
                        Tool::Line => at + Vec2::new(3.0, 0.0),
                        Tool::Circle => at + Vec2::new(2.0, 0.0),
                        _ => at + Vec2::new(7.0, 0.0),
                    };
                    shape_from_drag(&mut l, at, end);
                    self.push_shape(l);
                }
                if let Some((i, start)) = self.studio.drawing
                    && let Some(end) = pointer
                    && let Some(l) = self.target().layers.get_mut(i)
                {
                    shape_from_drag(l, start, end);
                }
                if resp.drag_stopped() {
                    self.studio.drawing = None;
                }
            }
        }

        // Box around the selected shape.
        let sel = self.layer;
        if let Some(b) = self.target().layers.get(sel).and_then(Layer::bounds) {
            let r = Rect::from_min_max(
                center + Vec2::new(b.0, b.1) * px,
                center + Vec2::new(b.2, b.3) * px,
            );
            painter.rect_stroke(
                r.expand(2.0),
                2.0,
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 170, 0, 150)),
                StrokeKind::Outside,
            );
        }
    }

    // ---- Right-hand panel ------------------------------------------------

    /// The crosshair's name, its shapes, the selected shape's settings and
    /// everything else folded away.
    pub(super) fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let ch = self.current();
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut ch.name).desired_width(f32::INFINITY));
        });
        ui.add_space(6.0);

        let uses_image = self.target().uses_image();
        ui.add_enabled_ui(!uses_image, |ui| {
            section(ui, "Shapes", |ui| self.layer_list(ui));
            let n = self.target().layers.len();
            if n > 0 {
                let i = self.layer.min(n - 1);
                self.layer = i;
                let layer = &mut self.target().layers[i];
                let title = format!("Selected: {}", layer.name);
                section(ui, &title, |ui| {
                    simple_props(ui, layer);
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new("More options")
                        .id_salt("more_layer_options")
                        .show(ui, |ui| layer_editor(ui, layer));
                });
            }
        });

        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new("Valorant code, image, position").strong())
            .id_salt("more_sections")
            .show(ui, |ui| self.more_sections(ui));
        egui::CollapsingHeader::new(RichText::new("Settings & hotkeys").strong())
            .default_open(false)
            .show(ui, |ui| self.settings(ui));
    }
}

/// Set a line, circle or ring from a drag between two whole-pixel points.
fn shape_from_drag(l: &mut Layer, start: Vec2, end: Vec2) {
    let d = end - start;
    let len = d.length();
    match l.shape {
        Kind::Line => {
            let mid = start + d / 2.0;
            l.x = mid.x;
            l.y = mid.y;
            // Both end pixels are part of the line.
            l.width = len + 1.0;
            l.rotation = if len > 0.0 {
                d.y.atan2(d.x).to_degrees().round()
            } else {
                0.0
            };
            l.smooth = l.rotation.rem_euclid(90.0) != 0.0;
        }
        Kind::Circle | Kind::Ring => {
            l.x = start.x;
            l.y = start.y;
            let r = len.round();
            l.width = if l.shape == Kind::Ring {
                2.0 * r + l.thickness
            } else {
                2.0 * r + 1.0
            };
            l.smooth = true;
        }
        _ => {}
    }
}

/// The few settings most shapes need.
fn simple_props(ui: &mut egui::Ui, l: &mut Layer) {
    ui.spacing_mut().slider_width = 120.0;
    grid(ui, "simple_props", |ui| {
        ui.label("Colour");
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 90.0;
            color_edit(ui, &mut l.color);
            ui.add(opacity_slider(&mut l.opacity))
                .on_hover_text("Opacity");
        });
        ui.end_row();

        let row = |ui: &mut egui::Ui, label: &str, v: &mut f32, max: f32| {
            ui.label(label);
            ui.add(size_slider(v, max));
            ui.end_row();
        };
        match l.shape {
            Kind::Rect if l.width == l.height => {
                let mut s = l.width;
                row(ui, "Size", &mut s, 20.0);
                l.width = s;
                l.height = s;
            }
            Kind::Rect | Kind::Triangle => {
                row(ui, "Width", &mut l.width, 40.0);
                row(ui, "Height", &mut l.height, 40.0);
            }
            Kind::Circle => row(ui, "Size", &mut l.width, 60.0),
            Kind::Line => {
                row(ui, "Length", &mut l.width, 60.0);
                row(ui, "Width", &mut l.thickness, 10.0);
            }
            Kind::Ring | Kind::Arc => {
                row(ui, "Size", &mut l.width, 100.0);
                row(ui, "Width", &mut l.thickness, 10.0);
            }
            Kind::Cross => {
                if l.width == l.height {
                    let mut s = l.width;
                    row(ui, "Length", &mut s, 40.0);
                    l.width = s;
                    l.height = s;
                } else {
                    row(ui, "Horizontal", &mut l.width, 40.0);
                    row(ui, "Vertical", &mut l.height, 40.0);
                }
                row(ui, "Width", &mut l.thickness, 10.0);
                row(ui, "Gap", &mut l.gap, 40.0);
            }
        }

        ui.label("Outline");
        let mut on = l.outline > 0.0;
        if ui.checkbox(&mut on, "").changed() {
            l.outline = if on { 1.0 } else { 0.0 };
            if on && l.outline_opacity <= 0.0 {
                l.outline_opacity = 0.5;
            }
        }
        ui.end_row();

        if l.shape == Kind::Cross {
            return;
        }
        ui.label("Symmetry");
        ui.horizontal(|ui| {
            let current = Symmetry::of(l);
            for m in Symmetry::ALL {
                if ui
                    .selectable_label(current == Some(m), m.label())
                    .on_hover_text(m.hint())
                    .clicked()
                {
                    m.apply(l);
                }
            }
        });
        ui.end_row();
    });
}
