//! The crosshair editor window.
//!
//! Runs as its own program, so the overlay never carries a GUI toolkit or a
//! GPU context. Every change is saved after a short pause: `config.toml` plus
//! the baked `overlay.bin`, which a running overlay picks up immediately.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::config::{self, Config, RightClickMode};
use crate::crosshair::Crosshair;
use crate::layer::{Kind, Layer};
use crate::updater::{self, Release};
use crosslay_format::CONFIG_FILE;
use eframe::egui::{
    self, Color32, ColorImage, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod draw;

/// How long the editor waits after the last change before saving.
const SAVE_DELAY: Duration = Duration::from_millis(250);

pub fn run(dir: PathBuf) -> Result<(), String> {
    let path = dir.join(CONFIG_FILE);
    let (cfg, warning) = match Config::load_or_create(&path) {
        // Bring overlay.bin up to date if config.toml was edited by hand.
        Ok(cfg) if config::needs_bake(&dir) => match cfg.save(&dir) {
            Ok(()) => (cfg, None),
            Err(e) => (cfg, Some(format!("Couldn't update the overlay: {e}"))),
        },
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
            .with_title(format!("crosslay editor {}", updater::CURRENT))
            .with_inner_size([1240.0, 780.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "crosslay-editor",
        options,
        Box::new(move |cc| Ok(Box::new(Menu::new(dir, cfg, warning, &cc.egui_ctx)))),
    )
    .map_err(|e| format!("cannot open the editor window: {e}"))
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
    /// The data folder: config.toml, overlay.bin and images/.
    dir: PathBuf,
    cfg: Config,
    /// What is on disk, to know whether there is anything to save.
    saved: Config,
    /// Config as of the previous frame, to detect edits.
    prev: Config,
    changed_at: Option<Instant>,
    live_apply: bool,
    selected: usize,
    /// Editing the selected crosshair's right-click crosshair instead of the
    /// crosshair itself (when it has one).
    editing_right_click: bool,
    /// Index of the layer being edited, in the edited crosshair's layers.
    layer: usize,
    /// Preview drag movement not applied yet (under a pixel), in pixels.
    drag_rest: Vec2,
    status: Status,
    preview: Option<Preview>,
    zoom: u32,
    background: Background,
    image_input: String,
    /// Text in the Valorant code box.
    code_input: String,
    /// Update check / install, driven by a background thread.
    update: Arc<Mutex<UpdateState>>,
    /// Tools, undo history and other drawing state.
    studio: draw::Studio,
}

#[derive(Clone)]
enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Installing(Release),
    Installed(Release),
    Failed(String),
}

impl Menu {
    fn new(dir: PathBuf, cfg: Config, warning: Option<String>, ctx: &egui::Context) -> Self {
        let selected = cfg.profile % cfg.crosshairs.len();
        let status = match warning {
            Some(w) => Status {
                text: w,
                error: true,
            },
            None => Status {
                text: format!("Editing {}", dir.join(CONFIG_FILE).display()),
                error: false,
            },
        };
        let mut menu = Self {
            dir,
            saved: cfg.clone(),
            prev: cfg.clone(),
            cfg,
            changed_at: None,
            live_apply: true,
            selected,
            editing_right_click: false,
            layer: 0,
            drag_rest: Vec2::ZERO,
            status,
            preview: None,
            zoom: 10,
            background: Background::Scene,
            image_input: String::new(),
            code_input: String::new(),
            update: Arc::new(Mutex::new(UpdateState::Idle)),
            studio: draw::Studio::default(),
        };
        menu.sync_image_input();
        if menu.cfg.auto_update && cfg!(windows) {
            menu.check_for_update(ctx);
        }
        menu
    }

    fn current(&mut self) -> &mut Crosshair {
        let n = self.cfg.crosshairs.len();
        self.selected = self.selected.min(n - 1);
        &mut self.cfg.crosshairs[self.selected]
    }

    /// The crosshair the editing panels and the preview work on: the
    /// selected one, or its right-click crosshair.
    fn target(&mut self) -> &mut Crosshair {
        let alt = self.editing_right_click;
        let ch = self.current();
        if alt && ch.right_click.is_some() {
            ch.right_click.as_deref_mut().expect("checked above")
        } else {
            ch
        }
    }

    fn editing_right_click(&mut self) -> bool {
        self.editing_right_click && self.current().right_click.is_some()
    }

    fn select(&mut self, i: usize) {
        self.selected = i.min(self.cfg.crosshairs.len() - 1);
        // The crosshair being edited is the one the overlay shows.
        self.cfg.profile = self.selected;
        self.sync_image_input();
    }

    /// Call after switching which crosshair is edited: shows its image path
    /// and selects its top layer.
    fn sync_image_input(&mut self) {
        let ch = self.target();
        let (image, top) = (ch.image.clone(), ch.layers.len().saturating_sub(1));
        self.image_input = image;
        self.layer = top;
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Status {
            text: text.into(),
            error,
        };
    }

    fn save(&mut self) {
        self.cfg.profile = self.selected;
        match self.cfg.save(&self.dir) {
            Ok(()) => {
                self.saved = self.cfg.clone();
                #[cfg(windows)]
                let running = crate::win::overlay_running();
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
        match Config::load(&self.dir.join(CONFIG_FILE)) {
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
        let images = self.dir.join("images");
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
                self.target().image = rel;
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

    #[cfg(windows)]
    fn start_overlay(&mut self) {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("crosslay.exe")));
        let started = exe.as_ref().is_some_and(|exe| {
            std::process::Command::new(exe)
                .arg("--dir")
                .arg(&self.dir)
                .spawn()
                .is_ok()
        });
        if !started {
            self.set_status(
                "Couldn't start crosslay.exe; keep it next to the editor",
                true,
            );
        }
    }

    fn update_state(&self) -> UpdateState {
        self.update
            .lock()
            .map(|s| s.clone())
            .unwrap_or(UpdateState::Idle)
    }

    /// Run `job` on a background thread, showing `busy` meanwhile.
    fn run_update_job(
        &self,
        ctx: &egui::Context,
        busy: UpdateState,
        job: impl FnOnce() -> UpdateState + Send + 'static,
    ) {
        if let Ok(mut s) = self.update.lock() {
            *s = busy;
        }
        let state = Arc::clone(&self.update);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = job();
            if let Ok(mut s) = state.lock() {
                *s = result;
            }
            ctx.request_repaint();
        });
    }

    fn check_for_update(&self, ctx: &egui::Context) {
        self.run_update_job(ctx, UpdateState::Checking, || match updater::check() {
            Ok(Some(rel)) => UpdateState::Available(rel),
            Ok(None) => UpdateState::UpToDate,
            Err(e) => UpdateState::Failed(e),
        });
    }

    fn install_update(&self, ctx: &egui::Context, rel: Release) {
        let dir = self.dir.clone();
        self.run_update_job(ctx, UpdateState::Installing(rel.clone()), move || {
            match updater::install(&rel, &dir) {
                Ok(()) => UpdateState::Installed(rel),
                Err(e) => UpdateState::Failed(e),
            }
        });
    }

    /// Start the freshly installed editor and close this one.
    fn restart(&self, ctx: &egui::Context) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe)
                .arg("--dir")
                .arg(&self.dir)
                .spawn();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// A strip across the top when there's an update to act on.
    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let state = self.update_state();
        let ctx = ui.ctx().clone();
        let show = matches!(
            state,
            UpdateState::Available(_) | UpdateState::Installing(_) | UpdateState::Installed(_)
        );
        if !show {
            return;
        }
        egui::Panel::top("update").show(ui, |ui| {
            ui.horizontal(|ui| match &state {
                UpdateState::Available(rel) => {
                    ui.label(
                        RichText::new(format!("crosslay {} is available", rel.version)).strong(),
                    );
                    if ui.button("Install and restart").clicked() {
                        self.install_update(&ctx, rel.clone());
                    }
                    if !rel.url.is_empty() {
                        ui.hyperlink_to("What's new", &rel.url);
                    }
                }
                UpdateState::Installing(rel) => {
                    ui.spinner();
                    ui.label(format!("Installing crosslay {}...", rel.version));
                }
                UpdateState::Installed(rel) => {
                    ui.label(
                        RichText::new(format!("Updated to crosslay {}", rel.version))
                            .color(Color32::from_rgb(80, 200, 120)),
                    );
                    if ui.button("Restart editor").clicked() {
                        self.restart(&ctx);
                    }
                }
                _ => {}
            });
        });
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

        self.update_banner(ui);
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("profiles")
            .resizable(false)
            .exact_size(190.0)
            .show(ui, |ui| self.profile_list(ui));
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(330.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
            });
        egui::CentralPanel::default().show(ui, |ui| self.canvas_area(ui));

        self.shortcuts(&ctx);
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
        self.track_history(ctx);
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
                .on_hover_text("Reload config.toml from disk")
                .clicked()
            {
                self.revert();
            }
            ui.separator();
            #[cfg(windows)]
            {
                if crate::win::overlay_running() {
                    ui.label(
                        RichText::new("Overlay running").color(Color32::from_rgb(80, 200, 120)),
                    );
                } else if ui
                    .button("Start overlay")
                    .on_hover_text("Runs crosslay.exe from the editor's folder")
                    .clicked()
                {
                    self.start_overlay();
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
                    let mut label = ch.name.clone();
                    if ch.uses_image() {
                        label.push_str("  (image)");
                    }
                    if ch.right_click.is_some() {
                        label.push_str("  + right click");
                    }
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

    /// Valorant codes, PNG crosshairs and placement on screen.
    fn more_sections(&mut self, ui: &mut egui::Ui) {
        // ---- Valorant code -----------------------------------------------
        section(ui, "Valorant crosshair code", |ui| {
            ui.label(
                RichText::new(
                    "Paste a code from Valorant (Settings > Crosshair > Import) to turn its \
                     primary crosshair (its ADS crosshair when editing the right-click \
                     crosshair) into shapes. This replaces the shapes above.",
                )
                .weak(),
            );
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.code_input)
                        .hint_text("0;P;c;5;h;0;0l;4;0o;0;0a;1;0f;0;1b;0")
                        .desired_width(300.0),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Import").clicked() || enter) && !self.code_input.trim().is_empty() {
                    let section = if self.editing_right_click() {
                        crate::valorant::Section::Ads
                    } else {
                        crate::valorant::Section::Primary
                    };
                    match crate::valorant::import(&self.code_input, section) {
                        Ok(settings) => {
                            self.target().layers = settings.to_layers();
                            self.sync_image_input();
                            self.set_status("Imported the Valorant crosshair code", false);
                        }
                        Err(e) => self.set_status(format!("Couldn't import: {e}"), true),
                    }
                }
                let exported = crate::valorant::Settings::from_layers(&self.target().layers);
                let hint = match &exported {
                    Ok(_) => "Copy this crosshair as a Valorant code".to_string(),
                    Err(e) => format!("Valorant can't show this crosshair: {e}"),
                };
                if ui
                    .add_enabled(exported.is_ok(), egui::Button::new("Copy code"))
                    .on_hover_text(&hint)
                    .on_disabled_hover_text(&hint)
                    .clicked()
                    && let Ok(settings) = exported
                {
                    let code = crate::valorant::export(&settings);
                    ui.ctx().copy_text(code.clone());
                    self.code_input = code;
                    self.set_status("Copied the Valorant code to the clipboard", false);
                }
            });
        });

        // ---- Image -------------------------------------------------------
        section(ui, "Image", |ui| {
            ui.label(RichText::new("Use your own PNG instead of the shapes.").weak());
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
                        self.dir.join(raw)
                    };
                    self.set_image(&p);
                }
                if self.target().uses_image() && ui.button("Remove").clicked() {
                    self.target().image.clear();
                    self.image_input.clear();
                }
            });
            let ch = self.target();
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

        let ch = self.target();
        // ---- Common ------------------------------------------------------
        section(ui, "Position on screen", |ui| {
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
    }

    /// Buttons to add shapes, and the list of layers, top first.
    fn layer_list(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("+ Add shape", |ui| {
            for kind in Kind::ALL {
                if ui
                    .button(kind.label())
                    .on_hover_text(kind_hint(kind))
                    .clicked()
                {
                    let l = self.new_shape(kind);
                    self.push_shape(l);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Or draw one on the canvas");
        ui.add_space(4.0);

        let sel = self.layer;
        let layers = &mut self.target().layers;
        let n = layers.len();
        if n == 0 {
            ui.label(
                RichText::new("No shapes yet. Draw one on the canvas or pick a starter.").weak(),
            );
            return;
        }
        let mut select = None;
        for i in (0..n).rev() {
            let l = &mut layers[i];
            ui.horizontal(|ui| {
                ui.checkbox(&mut l.visible, "")
                    .on_hover_text("Show this shape");
                let mut text =
                    RichText::new(format!("{}  ({})", l.name, l.shape.label().to_lowercase()));
                if !l.visible {
                    text = text.weak();
                }
                if ui
                    .add_sized(
                        [ui.available_width(), 22.0],
                        egui::Button::selectable(i == sel, text),
                    )
                    .clicked()
                {
                    select = Some(i);
                }
            });
        }
        let mut sel = select.unwrap_or(sel).min(n - 1);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(sel + 1 < n, egui::Button::new("Move up"))
                .on_hover_text("Draw it above the shape over it")
                .clicked()
            {
                layers.swap(sel, sel + 1);
                sel += 1;
            }
            if ui
                .add_enabled(sel > 0, egui::Button::new("Move down"))
                .on_hover_text("Draw it below the shape under it")
                .clicked()
            {
                layers.swap(sel, sel - 1);
                sel -= 1;
            }
            if ui.button("Duplicate").clicked() {
                let mut copy = layers[sel].clone();
                copy.name = unique_layer_name(layers, &copy.name);
                layers.insert(sel + 1, copy);
                sel += 1;
            }
            if ui.button("Delete").clicked() {
                layers.remove(sel);
                sel = sel.saturating_sub(1);
            }
        });
        self.layer = sel;
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
            ui.label("Right-click crosshair");
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut self.cfg.right_click,
                    RightClickMode::Hold,
                    "While held",
                )
                .on_hover_text("Shown while the right mouse button is held");
                ui.selectable_value(&mut self.cfg.right_click, RightClickMode::Toggle, "Toggle")
                    .on_hover_text("Each right click switches, for games with toggle aim");
            });
            ui.end_row();
            ui.label("Updates");
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.cfg.auto_update, "Install automatically")
                    .on_hover_text(
                        "When the overlay starts, check GitHub for a new version and install it",
                    );
                let busy = matches!(
                    self.update_state(),
                    UpdateState::Checking | UpdateState::Installing(_)
                );
                if ui
                    .add_enabled(cfg!(windows) && !busy, egui::Button::new("Check now"))
                    .clicked()
                {
                    self.check_for_update(ui.ctx());
                }
                let weak = ui.visuals().weak_text_color();
                let (text, color) = match self.update_state() {
                    UpdateState::Idle => (format!("Version {}", updater::CURRENT), weak),
                    UpdateState::Checking => ("Checking...".into(), weak),
                    UpdateState::UpToDate => {
                        (format!("Version {} is the latest", updater::CURRENT), weak)
                    }
                    UpdateState::Available(r) | UpdateState::Installing(r) => (
                        format!("{} available", r.version),
                        ui.visuals().warn_fg_color,
                    ),
                    UpdateState::Installed(r) => (format!("Updated to {}", r.version), weak),
                    UpdateState::Failed(e) => (e, ui.visuals().error_fg_color),
                };
                ui.label(RichText::new(text).color(color));
            });
            ui.end_row();
            let hk = &mut self.cfg.hotkeys;
            for (label, value) in [
                ("Show / hide", &mut hk.toggle),
                ("Open editor", &mut hk.editor),
                ("Next crosshair", &mut hk.next),
                ("Previous crosshair", &mut hk.previous),
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

    fn update_preview(&mut self, ctx: &egui::Context) {
        let ch = self.target().clone();
        if self.preview.as_ref().is_some_and(|p| p.key == ch) {
            return;
        }
        let (canvas, error) = match ch.render(&self.dir) {
            Ok(c) => (c, None),
            Err(e) => (ch.render_or_shapes(&self.dir), Some(e)),
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

fn opacity_slider(v: &mut f32) -> egui::Slider<'_> {
    egui::Slider::new(v, 0.0..=1.0).fixed_decimals(3)
}

fn kind_hint(kind: Kind) -> &'static str {
    match kind {
        Kind::Cross => "Four lines around a centre, like Valorant's inner and outer lines",
        Kind::Line => "A single line; rotate it or repeat it around the centre",
        Kind::Rect => "A rectangle; a small square makes a dot",
        Kind::Circle => "A filled circle",
        Kind::Ring => "A circle outline",
        Kind::Arc => "Part of a ring",
        Kind::Triangle => "A triangle, pointing up before rotation",
    }
}

/// `base`, or `base 2`, `base 3`... whichever no layer uses yet.
fn unique_layer_name(layers: &[Layer], base: &str) -> String {
    let base = base
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end();
    let base = if base.is_empty() { "Shape" } else { base };
    let taken = |n: &str| layers.iter().any(|l| l.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .unwrap()
}

/// A size slider in half pixels. Typing a bigger value than the slider
/// shows is allowed.
fn size_slider(v: &mut f32, max: f32) -> egui::Slider<'_> {
    egui::Slider::new(v, 0.0..=max)
        .step_by(0.5)
        .max_decimals(1)
        .clamping(egui::SliderClamping::Never)
        .suffix(" px")
}

/// Controls for one layer.
fn layer_editor(ui: &mut egui::Ui, l: &mut Layer) {
    ui.spacing_mut().slider_width = 130.0;
    grid(ui, "layer_grid", |ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut l.name).desired_width(200.0));
        ui.end_row();

        ui.label("Shape");
        egui::ComboBox::from_id_salt("layer_shape")
            .selected_text(l.shape.label())
            .show_ui(ui, |ui| {
                for k in Kind::ALL {
                    ui.selectable_value(&mut l.shape, k, k.label())
                        .on_hover_text(kind_hint(k));
                }
            });
        ui.end_row();

        let size = |ui: &mut egui::Ui, label: &str, v: &mut f32, max: f32, hint: &str| {
            ui.label(label);
            let r = ui.add(size_slider(v, max));
            if !hint.is_empty() {
                r.on_hover_text(hint);
            }
            ui.end_row();
        };
        match l.shape {
            Kind::Cross => {
                size(
                    ui,
                    "Horizontal length",
                    &mut l.width,
                    40.0,
                    "Left and right lines",
                );
                size(
                    ui,
                    "Vertical length",
                    &mut l.height,
                    40.0,
                    "Top and bottom lines",
                );
                size(ui, "Thickness", &mut l.thickness, 10.0, "");
                size(
                    ui,
                    "Gap",
                    &mut l.gap,
                    40.0,
                    "Space between the centre and the lines. At 0 they join into a solid cross.",
                );
            }
            Kind::Line => {
                size(ui, "Length", &mut l.width, 60.0, "");
                size(ui, "Thickness", &mut l.thickness, 10.0, "");
            }
            Kind::Rect | Kind::Triangle => {
                size(ui, "Width", &mut l.width, 40.0, "");
                size(ui, "Height", &mut l.height, 40.0, "");
            }
            Kind::Circle => size(ui, "Diameter", &mut l.width, 60.0, ""),
            Kind::Ring | Kind::Arc => {
                size(
                    ui,
                    "Diameter",
                    &mut l.width,
                    100.0,
                    "Outer edge to outer edge",
                );
                size(ui, "Thickness", &mut l.thickness, 10.0, "");
                if l.shape == Kind::Arc {
                    ui.label("Sweep");
                    ui.add(
                        egui::Slider::new(&mut l.sweep, 0.0..=360.0)
                            .step_by(1.0)
                            .suffix("°"),
                    )
                    .on_hover_text("How much of the circle the arc covers");
                    ui.end_row();
                }
            }
        }

        ui.label("Position");
        ui.horizontal(|ui| {
            for (axis, v) in [("X", &mut l.x), ("Y", &mut l.y)] {
                ui.label(axis);
                ui.add(
                    egui::DragValue::new(v)
                        .speed(0.25)
                        .range(-500.0..=500.0)
                        .max_decimals(1)
                        .suffix(" px"),
                );
            }
            if ui
                .button("Center")
                .on_hover_text("Put it on the screen centre")
                .clicked()
            {
                l.x = 0.0;
                l.y = 0.0;
            }
        })
        .response
        .on_hover_text("From the screen centre; Y grows downwards. Drag in the preview too.");
        ui.end_row();

        if l.shape.rotates() {
            ui.label("Rotation");
            ui.add(
                egui::Slider::new(&mut l.rotation, -180.0..=180.0)
                    .step_by(1.0)
                    .clamping(egui::SliderClamping::Never)
                    .suffix("°"),
            )
            .on_hover_text("Clockwise");
            ui.end_row();
        }

        ui.label("Repeat");
        ui.add(egui::Slider::new(&mut l.copies, 1..=crate::layer::MAX_COPIES).suffix("×"))
            .on_hover_text(
                "Copies spread evenly around the screen centre: 4 turns one line into a cross",
            );
        ui.end_row();

        ui.label("Mirror");
        ui.horizontal(|ui| {
            ui.checkbox(&mut l.mirror_x, "Left-right")
                .on_hover_text("Also draw it flipped across the centre, left to right");
            ui.checkbox(&mut l.mirror_y, "Up-down")
                .on_hover_text("Also draw it flipped across the centre, top to bottom");
        });
        ui.end_row();

        ui.label("Color");
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 90.0;
            color_edit(ui, &mut l.color);
            ui.add(opacity_slider(&mut l.opacity));
        });
        ui.end_row();

        ui.label("Outline");
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 90.0;
            ui.add(
                egui::Slider::new(&mut l.outline, 0.0..=6.0)
                    .step_by(0.5)
                    .max_decimals(1)
                    .clamping(egui::SliderClamping::Never)
                    .suffix(" px"),
            )
            .on_hover_text("0 = no outline. Outlines sit behind every shape.");
            ui.add_enabled_ui(l.outline > 0.0, |ui| {
                color_edit(ui, &mut l.outline_color);
                ui.add(opacity_slider(&mut l.outline_opacity));
            });
        });
        ui.end_row();

        ui.label("Smooth edges");
        ui.checkbox(&mut l.smooth, "").on_hover_text(
            "Anti-aliasing. Off keeps every pixel sharp, which suits straight lines.",
        );
        ui.end_row();
    });
}
