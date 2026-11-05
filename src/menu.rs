//! The crosshair editor ("menu").
//!
//! Runs as its own process (`crosslay --menu`), separate from the overlay, so
//! the overlay never carries a GUI toolkit or a GPU context. The editor's
//! only connection to the overlay is the config file: every change is saved
//! (debounced) and the overlay reloads it. On Windows it is also poked
//! directly so it updates right away.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::config::{self, Config};
use crate::crosshair::Crosshair;
use eframe::egui::{
    self, Color32, ColorImage, Pos2, Rect, RichText, Sense, TextureHandle, TextureOptions, Vec2,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the editor waits after the last change before saving.
const SAVE_DELAY: Duration = Duration::from_millis(250);

pub fn run(path: PathBuf) -> Result<(), String> {
    let (cfg, warning) = match Config::load_or_create(&path) {
        Ok(cfg) => (cfg, None),
        Err(e) => (
            Config::default(),
            Some(format!(
                "{e}. Showing the defaults; saving will replace the file."
            )),
        ),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("crosslay")
            .with_inner_size([1100.0, 680.0])
            .with_min_inner_size([760.0, 500.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "crosslay",
        options,
        Box::new(move |_cc| Ok(Box::new(Menu::new(path, cfg, warning)))),
    )
    .map_err(|e| format!("cannot open the menu: {e}"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Background {
    Dark,
    Light,
    Checker,
    Scene,
}

struct Status {
    text: String,
    error: bool,
}

struct Preview {
    key: Crosshair,
    texture: Option<TextureHandle>,
    size: [u32; 2],
    anchor: (i32, i32),
    error: Option<String>,
}

struct Menu {
    path: PathBuf,
    base: PathBuf,
    cfg: Config,
    /// What is on disk, to know whether there is anything to save.
    saved: Config,
    /// Config as of the previous frame, to detect edits.
    prev: Config,
    changed_at: Option<Instant>,
    live_apply: bool,
    selected: usize,
    status: Status,
    preview: Option<Preview>,
    zoom: u32,
    background: Background,
    image_input: String,
}

impl Menu {
    fn new(path: PathBuf, cfg: Config, warning: Option<String>) -> Self {
        let selected = cfg.profile % cfg.crosshairs.len();
        let status = match warning {
            Some(w) => Status {
                text: w,
                error: true,
            },
            None => Status {
                text: format!("Editing {}", path.display()),
                error: false,
            },
        };
        let mut menu = Self {
            base: config::base_dir(&path),
            path,
            saved: cfg.clone(),
            prev: cfg.clone(),
            cfg,
            changed_at: None,
            live_apply: true,
            selected,
            status,
            preview: None,
            zoom: 6,
            background: Background::Scene,
            image_input: String::new(),
        };
        menu.sync_image_input();
        menu
    }

    fn current(&mut self) -> &mut Crosshair {
        let n = self.cfg.crosshairs.len();
        self.selected = self.selected.min(n - 1);
        &mut self.cfg.crosshairs[self.selected]
    }

    fn select(&mut self, i: usize) {
        self.selected = i.min(self.cfg.crosshairs.len() - 1);
        // The crosshair being edited is the one the overlay shows.
        self.cfg.profile = self.selected;
        self.sync_image_input();
    }

    fn sync_image_input(&mut self) {
        self.image_input = self.current().image.clone();
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Status {
            text: text.into(),
            error,
        };
    }

    fn save(&mut self) {
        self.cfg.profile = self.selected;
        match self.cfg.save(&self.path) {
            Ok(()) => {
                self.saved = self.cfg.clone();
                #[cfg(windows)]
                let running = {
                    crate::windows::notify_overlay();
                    crate::windows::overlay_running()
                };
                #[cfg(not(windows))]
                let running = false;
                let msg = if running {
                    "Saved, overlay updated"
                } else {
                    "Saved"
                };
                self.set_status(msg, false);
            }
            Err(e) => self.set_status(format!("Not saved: {e}"), true),
        }
    }

    fn revert(&mut self) {
        match Config::load(&self.path) {
            Ok(cfg) => {
                self.cfg = cfg.clone();
                self.saved = cfg.clone();
                self.prev = cfg;
                self.changed_at = None;
                self.select(self.cfg.profile % self.cfg.crosshairs.len());
                self.set_status("Reloaded from disk", false);
            }
            Err(e) => self.set_status(e, true),
        }
    }

    /// Copy a PNG into the config's `images` folder (so the crosshair keeps
    /// working if the original moves) and return the path to store.
    fn import_png(&self, src: &Path) -> Result<String, String> {
        crate::image::load_png(src)?; // validate before copying
        let images = self.base.join("images");
        if src.parent().and_then(|p| p.canonicalize().ok()) == images.canonicalize().ok() {
            let name = src.file_name().unwrap_or_default().to_string_lossy();
            return Ok(format!("images/{name}"));
        }
        std::fs::create_dir_all(&images)
            .map_err(|e| format!("cannot create {}: {e}", images.display()))?;
        let bytes =
            std::fs::read(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "crosshair".into());
        for n in 1.. {
            let name = if n == 1 {
                format!("{stem}.png")
            } else {
                format!("{stem}-{n}.png")
            };
            let dest = images.join(&name);
            match std::fs::read(&dest) {
                Ok(existing) if existing == bytes => return Ok(format!("images/{name}")),
                Ok(_) => continue,
                Err(_) => {
                    std::fs::write(&dest, &bytes)
                        .map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
                    return Ok(format!("images/{name}"));
                }
            }
        }
        unreachable!()
    }

    /// Use a PNG for the selected crosshair.
    fn set_image(&mut self, src: &Path) {
        match self.import_png(src) {
            Ok(rel) => {
                self.current().image = rel;
                self.sync_image_input();
                self.set_status(format!("Imported {}", src.display()), false);
            }
            Err(e) => self.set_status(e, true),
        }
    }

    /// Add a new crosshair profile made from a PNG.
    fn new_from_png(&mut self, src: &Path) {
        match self.import_png(src) {
            Ok(rel) => {
                let name = src
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Image".into());
                self.cfg.crosshairs.push(Crosshair {
                    name,
                    image: rel,
                    ..Default::default()
                });
                self.select(self.cfg.crosshairs.len() - 1);
                self.set_status(format!("Created a crosshair from {}", src.display()), false);
            }
            Err(e) => self.set_status(e, true),
        }
    }

    fn unique_name(&self, base: &str) -> String {
        let taken = |n: &str| self.cfg.crosshairs.iter().any(|c| c.name == n);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base} {i}"))
            .find(|n| !taken(n))
            .unwrap()
    }
}

#[cfg(windows)]
fn pick_png() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("PNG image", &["png"])
        .set_title("Choose a crosshair PNG")
        .pick_file()
}

#[cfg(not(windows))]
fn pick_png() -> Option<PathBuf> {
    None
}

const HAS_FILE_DIALOG: bool = cfg!(windows);

impl eframe::App for Menu {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ui.spacing_mut().slider_width = 160.0;
        self.handle_dropped_files(&ctx);

        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("profiles")
            .resizable(false)
            .exact_size(210.0)
            .show(ui, |ui| self.profile_list(ui));
        egui::Panel::right("preview")
            .resizable(true)
            .default_size(340.0)
            .show(ui, |ui| self.preview_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.editor(ui));
        });

        self.drop_hint(&ctx);
        self.autosave(&ctx);
    }
}

impl Menu {
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for path in dropped {
            self.new_from_png(&path);
        }
    }

    fn drop_hint(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("drop"),
        ));
        let rect = ctx.content_rect();
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(180));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop a PNG to create a crosshair from it",
            egui::FontId::proportional(24.0),
            Color32::WHITE,
        );
    }

    fn autosave(&mut self, ctx: &egui::Context) {
        if self.cfg != self.prev {
            self.prev = self.cfg.clone();
            self.changed_at = Some(Instant::now());
        }
        if !self.live_apply {
            return;
        }
        if let Some(t) = self.changed_at {
            let elapsed = t.elapsed();
            if elapsed >= SAVE_DELAY {
                self.changed_at = None;
                if self.cfg != self.saved {
                    self.save();
                }
            } else {
                ctx.request_repaint_after(SAVE_DELAY - elapsed);
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let dirty = self.cfg != self.saved;
            if ui
                .checkbox(&mut self.live_apply, "Live apply")
                .on_hover_text(
                    "Save every change right away, so the overlay updates while you edit",
                )
                .changed()
                && self.live_apply
            {
                self.changed_at = Some(Instant::now() - SAVE_DELAY);
            }
            if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
                self.save();
            }
            if ui
                .button("Revert")
                .on_hover_text("Reload the config file from disk")
                .clicked()
            {
                self.revert();
            }
            ui.separator();
            #[cfg(windows)]
            {
                if crate::windows::overlay_running() {
                    ui.label(
                        RichText::new("Overlay running").color(Color32::from_rgb(80, 200, 120)),
                    );
                } else if ui.button("Start overlay").clicked()
                    && let Ok(exe) = std::env::current_exe()
                {
                    let _ = std::process::Command::new(exe)
                        .arg("--config")
                        .arg(&self.path)
                        .spawn();
                }
                ui.separator();
            }
            let color = if self.status.error {
                ui.visuals().error_fg_color
            } else if dirty && !self.live_apply {
                ui.visuals().warn_fg_color
            } else {
                ui.visuals().weak_text_color()
            };
            let text = if dirty && !self.live_apply && !self.status.error {
                "Unsaved changes".to_string()
            } else {
                self.status.text.clone()
            };
            ui.label(RichText::new(text).color(color));
        });
    }

    fn profile_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Crosshairs");
        ui.add_space(4.0);
        let mut select = None;
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 110.0)
            .show(ui, |ui| {
                for (i, ch) in self.cfg.crosshairs.iter().enumerate() {
                    let label = if ch.uses_image() {
                        format!("{}  (image)", ch.name)
                    } else {
                        ch.name.clone()
                    };
                    let resp = ui.add_sized(
                        [ui.available_width(), 24.0],
                        egui::Button::selectable(i == self.selected, label),
                    );
                    if resp.clicked() {
                        select = Some(i);
                    }
                }
            });
        if let Some(i) = select {
            self.select(i);
        }

        ui.separator();
        let n = self.cfg.crosshairs.len();
        ui.horizontal_wrapped(|ui| {
            if ui.button("+ New").clicked() {
                let name = self.unique_name("Crosshair");
                self.cfg.crosshairs.push(Crosshair {
                    name,
                    ..Default::default()
                });
                self.select(n);
            }
            if ui.button("Duplicate").clicked() {
                let mut copy = self.current().clone();
                copy.name = self.unique_name(&copy.name);
                self.cfg.crosshairs.insert(self.selected + 1, copy);
                self.select(self.selected + 1);
            }
            if ui.add_enabled(n > 1, egui::Button::new("Delete")).clicked() {
                self.cfg.crosshairs.remove(self.selected);
                self.select(self.selected.saturating_sub(1));
            }
            if ui
                .add_enabled(self.selected > 0, egui::Button::new("Up"))
                .on_hover_text("Move up")
                .clicked()
            {
                self.cfg.crosshairs.swap(self.selected, self.selected - 1);
                self.select(self.selected - 1);
            }
            if ui
                .add_enabled(self.selected + 1 < n, egui::Button::new("Down"))
                .on_hover_text("Move down")
                .clicked()
            {
                self.cfg.crosshairs.swap(self.selected, self.selected + 1);
                self.select(self.selected + 1);
            }
        });
        if HAS_FILE_DIALOG
            && ui.button("+ New from PNG...").clicked()
            && let Some(p) = pick_png()
        {
            self.new_from_png(&p);
        }
        ui.label(
            RichText::new("Tip: drop a PNG anywhere on this window")
                .small()
                .weak(),
        );
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let ch = self.current();
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut ch.name).desired_width(240.0));
        });
        ui.add_space(8.0);

        // ---- Image -------------------------------------------------------
        section(ui, "Image", |ui| {
            ui.label(RichText::new("Use your own PNG instead of the drawn shapes.").weak());
            ui.horizontal(|ui| {
                if HAS_FILE_DIALOG
                    && ui.button("Choose PNG...").clicked()
                    && let Some(p) = pick_png()
                {
                    self.set_image(&p);
                }
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.image_input)
                        .hint_text("path to a .png file")
                        .desired_width(260.0),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Load").clicked() || enter) && !self.image_input.trim().is_empty() {
                    let raw = PathBuf::from(self.image_input.trim());
                    let p = if raw.is_absolute() {
                        raw
                    } else {
                        self.base.join(raw)
                    };
                    self.set_image(&p);
                }
                if self.current().uses_image() && ui.button("Remove").clicked() {
                    self.current().image.clear();
                    self.image_input.clear();
                }
            });
            let ch = self.current();
            if ch.uses_image() {
                grid(ui, "image_grid", |ui| {
                    ui.label("Scale");
                    ui.add(
                        egui::Slider::new(&mut ch.image_scale, 0.1..=8.0)
                            .logarithmic(true)
                            .suffix("×"),
                    );
                    ui.end_row();
                });
            }
        });

        let uses_image = self.current().uses_image();
        let ch = self.current();

        // ---- Shapes ------------------------------------------------------
        section(ui, "Shape", |ui| {
            if uses_image {
                ui.label(RichText::new("Shapes are hidden while an image is set.").weak());
            }
            ui.add_enabled_ui(!uses_image, |ui| {
                grid(ui, "shape_grid", |ui| {
                    ui.label("Color");
                    color_edit(ui, &mut ch.color);
                    ui.end_row();
                    ui.label("Arm length");
                    ui.add(egui::Slider::new(&mut ch.length, 0..=60).suffix(" px"));
                    ui.end_row();
                    ui.label("Thickness");
                    ui.add(egui::Slider::new(&mut ch.thickness, 1..=20).suffix(" px"));
                    ui.end_row();
                    ui.label("Gap");
                    ui.add(egui::Slider::new(&mut ch.gap, 0..=60).suffix(" px"));
                    ui.end_row();
                    ui.label("T-style");
                    ui.checkbox(&mut ch.t_style, "Hide the top arm");
                    ui.end_row();
                    ui.label("Center dot");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut ch.center_dot, "");
                        ui.add_enabled(
                            ch.center_dot,
                            egui::Slider::new(&mut ch.dot_size, 1..=20).suffix(" px"),
                        );
                    });
                    ui.end_row();
                    ui.label("Circle radius");
                    ui.add(egui::Slider::new(&mut ch.circle_radius, 0..=120).suffix(" px"));
                    ui.end_row();
                    ui.label("Circle thickness");
                    ui.add_enabled(
                        ch.circle_radius > 0,
                        egui::Slider::new(&mut ch.circle_thickness, 1..=20).suffix(" px"),
                    );
                    ui.end_row();
                    ui.label("Outline");
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut ch.outline, 0..=6).suffix(" px"));
                        color_edit(ui, &mut ch.outline_color);
                    });
                    ui.end_row();
                });
            });
        });

        // ---- Common ------------------------------------------------------
        section(ui, "Placement", |ui| {
            grid(ui, "common_grid", |ui| {
                ui.label("Opacity");
                ui.add(egui::Slider::new(&mut ch.opacity, 0.0..=1.0).fixed_decimals(2));
                ui.end_row();
                ui.label("Offset X");
                ui.add(
                    egui::DragValue::new(&mut ch.offset_x)
                        .range(-2000..=2000)
                        .suffix(" px"),
                );
                ui.end_row();
                ui.label("Offset Y");
                ui.add(
                    egui::DragValue::new(&mut ch.offset_y)
                        .range(-2000..=2000)
                        .suffix(" px"),
                );
                ui.end_row();
                ui.label("");
                if ui.button("Reset offset").clicked() {
                    ch.offset_x = 0;
                    ch.offset_y = 0;
                }
                ui.end_row();
            });
        });

        // ---- Settings ----------------------------------------------------
        egui::CollapsingHeader::new(RichText::new("Settings & hotkeys").strong())
            .default_open(false)
            .show(ui, |ui| self.settings(ui));
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        grid(ui, "settings_grid", |ui| {
            ui.label("Monitor");
            ui.add(egui::DragValue::new(&mut self.cfg.monitor).range(0..=16))
                .on_hover_text("0 = primary monitor, 1.. = the others");
            ui.end_row();
            ui.label("Start visible");
            ui.checkbox(&mut self.cfg.visible, "");
            ui.end_row();
            ui.label("Live reload");
            ui.checkbox(
                &mut self.cfg.live_reload,
                "Overlay picks up config edits automatically",
            );
            ui.end_row();
            let hk = &mut self.cfg.hotkeys;
            for (label, value) in [
                ("Show / hide", &mut hk.toggle),
                ("Open menu", &mut hk.menu),
                ("Next crosshair", &mut hk.next),
                ("Previous crosshair", &mut hk.previous),
                ("Reload config", &mut hk.reload),
                ("Quit overlay", &mut hk.quit),
            ] {
                ui.label(label);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(value)
                            .desired_width(150.0)
                            .hint_text("disabled"),
                    );
                    if !value.trim().is_empty()
                        && let Err(e) = value.parse::<crate::hotkey::Hotkey>()
                    {
                        ui.label(RichText::new("invalid").color(ui.visuals().error_fg_color))
                            .on_hover_text(e);
                    }
                });
                ui.end_row();
            }
        });
        ui.label(
            RichText::new("Examples: Ctrl+Alt+X, Shift+F8, Numpad5. Leave empty to disable.")
                .small()
                .weak(),
        );
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Preview");
        ui.horizontal(|ui| {
            ui.label("Zoom");
            ui.add(egui::Slider::new(&mut self.zoom, 1..=16).suffix("×"));
        });
        ui.horizontal(|ui| {
            ui.label("Background");
            ui.selectable_value(&mut self.background, Background::Scene, "Scene");
            ui.selectable_value(&mut self.background, Background::Dark, "Dark");
            ui.selectable_value(&mut self.background, Background::Light, "Light");
            ui.selectable_value(&mut self.background, Background::Checker, "Checker");
        });
        ui.add_space(6.0);

        self.update_preview(ui.ctx());
        let side = ui
            .available_width()
            .min(ui.available_height() - 30.0)
            .max(120.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
        let painter = ui.painter_at(rect);
        paint_background(&painter, rect, self.background);

        let Some(p) = &self.preview else { return };
        if let Some(tex) = &p.texture {
            // 1× = real on-screen size, regardless of the editor's UI scaling.
            let px = self.zoom as f32 / ui.ctx().pixels_per_point();
            let center = rect.center();
            let min = Pos2::new(
                center.x - (p.anchor.0 as f32 + 0.5) * px,
                center.y - (p.anchor.1 as f32 + 0.5) * px,
            );
            let r =
                Rect::from_min_size(min, Vec2::new(p.size[0] as f32 * px, p.size[1] as f32 * px));
            painter.image(
                tex.id(),
                r,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let info = match &p.error {
            Some(e) => {
                RichText::new(format!("Image error: {e}")).color(ui.visuals().error_fg_color)
            }
            None => RichText::new(format!("{} × {} px", p.size[0], p.size[1])).weak(),
        };
        ui.label(info);
    }

    fn update_preview(&mut self, ctx: &egui::Context) {
        let ch = self.cfg.crosshairs[self.selected.min(self.cfg.crosshairs.len() - 1)].clone();
        if self.preview.as_ref().is_some_and(|p| p.key == ch) {
            return;
        }
        let (canvas, error) = match ch.render(&self.base) {
            Ok(c) => (c, None),
            Err(e) => (ch.render_or_shapes(&self.base), Some(e)),
        };
        let texture = Some(ctx.load_texture(
            "crosshair-preview",
            to_color_image(&canvas),
            TextureOptions::NEAREST,
        ));
        self.preview = Some(Preview {
            key: ch,
            texture,
            size: [canvas.width, canvas.height],
            anchor: canvas.anchor,
            error,
        });
    }
}

fn to_color_image(c: &Canvas) -> ColorImage {
    let pixels = c
        .pixels
        .iter()
        .map(|&px| {
            // Canvas is already premultiplied, which is what Color32 stores.
            Color32::from_rgba_premultiplied(
                (px >> 16) as u8,
                (px >> 8) as u8,
                px as u8,
                (px >> 24) as u8,
            )
        })
        .collect();
    ColorImage::new([c.width as usize, c.height as usize], pixels)
}

fn paint_background(painter: &egui::Painter, rect: Rect, bg: Background) {
    match bg {
        Background::Dark => {
            painter.rect_filled(rect, 4.0, Color32::from_rgb(24, 24, 28));
        }
        Background::Light => {
            painter.rect_filled(rect, 4.0, Color32::from_rgb(235, 235, 235));
        }
        Background::Checker => {
            painter.rect_filled(rect, 4.0, Color32::from_gray(200));
            let s = 12.0;
            let (cols, rows) = (
                (rect.width() / s).ceil() as i32,
                (rect.height() / s).ceil() as i32,
            );
            for y in 0..rows {
                for x in 0..cols {
                    if (x + y) % 2 == 0 {
                        let min = rect.min + Vec2::new(x as f32 * s, y as f32 * s);
                        let r = Rect::from_min_size(min, Vec2::splat(s)).intersect(rect);
                        painter.rect_filled(r, 0.0, Color32::from_gray(150));
                    }
                }
            }
        }
        Background::Scene => {
            // A rough "game" backdrop: sky, horizon and ground in stripes, so
            // the crosshair can be judged against light and dark at once.
            let bands = [
                Color32::from_rgb(120, 170, 220),
                Color32::from_rgb(170, 200, 230),
                Color32::from_rgb(200, 190, 160),
                Color32::from_rgb(110, 120, 80),
                Color32::from_rgb(60, 70, 50),
                Color32::from_rgb(35, 35, 40),
            ];
            let h = rect.height() / bands.len() as f32;
            for (i, c) in bands.iter().enumerate() {
                let r = Rect::from_min_size(
                    rect.min + Vec2::new(0.0, i as f32 * h),
                    Vec2::new(rect.width(), h + 1.0),
                );
                painter.rect_filled(r.intersect(rect), 0.0, *c);
            }
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(15.0));
            ui.add_space(4.0);
            body(ui);
        });
    ui.add_space(8.0);
}

fn grid(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 8.0])
        .min_col_width(110.0)
        .show(ui, body);
}

fn color_edit(ui: &mut egui::Ui, c: &mut Color) -> egui::Response {
    let mut c32 = Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a);
    let resp = egui::color_picker::color_edit_button_srgba(
        ui,
        &mut c32,
        egui::color_picker::Alpha::OnlyBlend,
    );
    if resp.changed() {
        let [r, g, b, a] = c32.to_srgba_unmultiplied();
        *c = Color::rgba(r, g, b, a);
    }
    resp
}
