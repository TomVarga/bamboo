#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;

use bamboo::files::DecodedPng;
use bamboo::state::{Images, Status};
use bamboo::{API_KEY_URL, SOURCE_URL, VERSION};

const BG_TOP: egui::Color32 = egui::Color32::from_rgb(0xe4, 0xff, 0xe0);
const BG_BOTTOM: egui::Color32 = egui::Color32::from_rgb(0xee, 0xff, 0xeb);
const GATE_BOTTOM: egui::Color32 = egui::Color32::from_rgb(0xcd, 0xf8, 0xff);
const SETTINGS_BOTTOM: egui::Color32 = egui::Color32::from_rgb(0xd4, 0xf9, 0xff);
const SETTINGS_TOP: egui::Color32 = egui::Color32::from_rgb(0xc5, 0xf7, 0xff);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0x0f, 0xd5, 0xf5);
const CYAN_HOVER: egui::Color32 = egui::Color32::from_rgb(0x01, 0xad, 0xe4);
const ICON: egui::Color32 = egui::Color32::from_black_alpha(128);
const DONE_GREEN: egui::Color32 = egui::Color32::from_rgb(0x64, 0xde, 0x64);
const DONE_GREEN_HOVER: egui::Color32 = egui::Color32::from_rgb(0x4b, 0xac, 0x4b);
const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(0x00, 0x9e, 0x00);
const ERR_RED: egui::Color32 = egui::Color32::from_rgb(0xff, 0x35, 0x35);
const LOAD_ORANGE: egui::Color32 = egui::Color32::from_rgb(0xff, 0xa4, 0x00);
const TEXT: egui::Color32 = egui::Color32::from_rgb(55, 70, 60);

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([600.0, 500.0])
        .with_min_inner_size([600.0, 500.0])
        .with_max_inner_size([1024.0, 8192.0])
        .with_resizable(true)
        .with_maximize_button(false)
        .with_fullscreen(false)
        .with_drag_and_drop(true);
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }

    eframe::run_native(
        "Bamboo",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(BambooApp::new(cc)))),
    )
}

fn app_icon() -> Option<egui::IconData> {
    let DecodedPng {
        width,
        height,
        rgba,
    } = bamboo::files::decode_png(bamboo::files::app_icon_png()).ok()?;
    Some(egui::IconData {
        rgba,
        width,
        height,
    })
}

struct UploadEvent {
    id: u64,
    result: Result<PathBuf, String>,
}

struct BambooApp {
    api_key: String,
    gate_input: String,
    settings_open: bool,
    settings_input: String,
    images: Images,
    tx: Sender<UploadEvent>,
    rx: Receiver<UploadEvent>,
}

impl BambooApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        style_ui(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        Self {
            api_key: bamboo::settings::load_key(&bamboo::settings::settings_path()),
            gate_input: String::new(),
            settings_open: false,
            settings_input: String::new(),
            images: Images::default(),
            tx,
            rx,
        }
    }

    fn on_main(&self) -> bool {
        !self.api_key.is_empty() && !self.settings_open
    }

    fn commit_key(&mut self, raw: &str) -> bool {
        let key = raw.trim();
        if key.is_empty() {
            dialog("Invalid API key", "You must enter a valid API key!");
            return false;
        }
        if let Err(err) = bamboo::settings::save_key(&bamboo::settings::settings_path(), key) {
            dialog("Bamboo", &format!("Could not save the API key.\n{err}"));
            return false;
        }
        self.api_key = key.to_string();
        true
    }

    fn ingest(&mut self, ctx: &egui::Context, path: PathBuf) {
        if path.is_dir() {
            for image in bamboo::files::images_in_folder(&path) {
                self.enqueue(ctx, image);
            }
        } else if bamboo::files::is_image(&path) {
            self.enqueue(ctx, path);
        }
    }

    fn enqueue(&mut self, ctx: &egui::Context, path: PathBuf) {
        let Some(id) = self.images.add(path.clone()) else {
            return;
        };
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let key = self.api_key.clone();
        // ponytail: one thread per image, same as the electron app; cap it if big folders stall the machine.
        std::thread::spawn(move || {
            let result = compress(&key, &path);
            let _ = tx.send(UploadEvent { id, result });
            ctx.request_repaint();
        });
    }

    fn pick_files(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Bamboo - Add Images")
            .add_filter("Images", &["jpg", "jpeg", "png"]);
        if let Some(dir) = dirs::document_dir() {
            dialog = dialog.set_directory(dir);
        }
        let Some(paths) = dialog.pick_files() else {
            return;
        };
        for path in paths {
            if bamboo::files::is_image(&path) {
                self.enqueue(ctx, path);
            }
        }
    }
}

impl eframe::App for BambooApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.rx.try_recv() {
            match event.result {
                Ok(backup) => {
                    let message = match trash::delete(&backup) {
                        Ok(()) => None,
                        Err(err) => {
                            eprintln!("bamboo: could not move original to trash: {err}");
                            Some(format!(
                                "Compressed. The original is still at {}",
                                backup.display()
                            ))
                        }
                    };
                    self.images.mark_done(event.id, message);
                }
                Err(message) => self.images.mark_error(event.id, message),
            }
        }
        if self.images.any_loading() {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.on_main() {
            let dropped = ui.input(|input| input.raw.dropped_files.clone());
            for file in dropped {
                self.ingest(&ctx, file.path().to_path_buf());
            }
        }
        let hovering = self.on_main() && ui.input(|input| !input.raw.hovered_files.is_empty());
        if hovering {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
        }
        #[cfg(target_os = "macos")]
        if ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Q)) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        let rect = ui.max_rect();
        if self.api_key.is_empty() {
            self.gate(ui, rect);
        } else if self.settings_open {
            self.settings(ui, rect);
        } else {
            self.main_view(ui, rect, hovering);
        }
        if self.images.any_loading() {
            ui.ctx().request_repaint();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BG_BOTTOM.to_normalized_gamma_f32()
    }
}

impl BambooApp {
    fn gate(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        paint_gradient(ui.painter(), rect, SETTINGS_TOP, GATE_BOTTOM);
        let bottom = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - 64.0),
            rect.right_bottom(),
        );
        let middle =
            egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), bottom.top()));

        let width = (middle.width() * 0.72).clamp(240.0, 460.0);
        {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(middle)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            child.add_space((middle.height() - 180.0).max(0.0) / 2.0);
            let (icon, _) = child.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
            paint_alert(child.painter(), icon.center(), 28.0, ICON);
            child.add_space(8.0);
            child.label(
                egui::RichText::new(
                    "In order to use this app, you must have a valid Tinify API key.",
                )
                .size(18.0)
                .color(TEXT),
            );
            child.label(
                egui::RichText::new("Enter your API key below:")
                    .size(15.0)
                    .color(TEXT),
            );
            child.add_space(6.0);
            child.allocate_ui_with_layout(
                egui::vec2(width, 40.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| key_field(ui, &mut self.gate_input, "MY-API-KEY"),
            );
        }

        let show_done = !self.gate_input.trim().is_empty();
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bottom)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        child.add_space(8.0);
        if show_done {
            let clicked = green_button(&mut child, "Done");
            if clicked {
                let key = self.gate_input.clone();
                if self.commit_key(&key) {
                    self.gate_input.clear();
                }
            }
        } else if link_line(&mut child, "Click here", " to get an API key.") {
            open_url(API_KEY_URL);
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        paint_gradient(ui.painter(), rect, SETTINGS_TOP, SETTINGS_BOTTOM);
        let toolbar = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - 54.0),
            rect.right_bottom(),
        );
        let body =
            egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), toolbar.top()));
        {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body.shrink(16.0))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.label(egui::RichText::new("API key").color(TEXT).size(16.0));
            child.add_space(6.0);
            key_field(&mut child, &mut self.settings_input, "");
        }

        ui.painter()
            .rect_filled(toolbar, 0.0, egui::Color32::from_black_alpha(13));
        let save = toolbar_text_button(
            ui,
            egui::pos2(toolbar.left() + toolbar.width() * 0.33, toolbar.center().y),
            "save",
            "Save",
            paint_save,
        );
        let close = toolbar_text_button(
            ui,
            egui::pos2(toolbar.left() + toolbar.width() * 0.67, toolbar.center().y),
            "close-settings",
            "Close",
            paint_close,
        );
        if save.clicked() {
            let key = self.settings_input.clone();
            if self.commit_key(&key) {
                self.settings_input = self.api_key.clone();
            }
        }
        if close.clicked() {
            self.settings_open = false;
        }
    }

    fn main_view(&mut self, ui: &mut egui::Ui, rect: egui::Rect, hovering: bool) {
        paint_gradient(ui.painter(), rect, BG_TOP, BG_BOTTOM);
        let toolbar = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - 54.0),
            rect.right_bottom(),
        );
        let body =
            egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), toolbar.top()));
        if hovering {
            ui.painter()
                .rect_filled(body, 0.0, egui::Color32::from_black_alpha(64));
        }

        if self.images.is_empty() {
            let color = if hovering {
                egui::Color32::from_black_alpha(150)
            } else {
                egui::Color32::from_black_alpha(90)
            };
            drop_hint(ui, body, color);
        } else {
            self.image_list(ui, body);
        }

        ui.painter()
            .rect_filled(toolbar, 0.0, egui::Color32::from_black_alpha(13));
        let about = icon_button(
            ui,
            square(
                egui::pos2(toolbar.left() + toolbar.width() / 6.0, toolbar.center().y),
                44.0,
            ),
            "about",
            "About",
            paint_help,
        );
        let settings = icon_button(
            ui,
            square(
                egui::pos2(
                    toolbar.left() + toolbar.width() * 5.0 / 6.0,
                    toolbar.center().y,
                ),
                44.0,
            ),
            "settings",
            "Settings",
            paint_gear,
        );
        if about.clicked() {
            show_about();
        }
        if settings.clicked() {
            self.settings_input = self.api_key.clone();
            self.settings_open = true;
        }
        let plus_center = egui::pos2(toolbar.center().x, toolbar.top() + 8.0);
        if plus_button(ui.ctx(), plus_center) {
            self.pick_files(ui.ctx());
        }
    }

    fn image_list(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let rows: Vec<Row> = self
            .images
            .iter()
            .map(|item| Row {
                id: item.id,
                path: item.path.clone(),
                name: item.file_name.clone(),
                status: item.status,
                message: item.message.clone(),
            })
            .collect();
        let mut action = None;
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                for row in &rows {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 36.0),
                        egui::Sense::click(),
                    );
                    if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(13));
                    }
                    let icon = egui::pos2(rect.left() + 20.0, rect.center().y);
                    paint_status(ui, icon, row.status);
                    ui.painter().text(
                        egui::pos2(rect.left() + 40.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &row.name,
                        egui::FontId::proportional(16.0),
                        TEXT,
                    );
                    if let Some(message) = &row.message {
                        response.clone().on_hover_text(message);
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, &row.name)
                    });
                    egui::Popup::menu(&response).show(|ui| image_menu(ui, row, &mut action));
                }
            });

        match action {
            Some(Action::Remove(id)) => self.images.remove(id),
            Some(Action::RemoveAll) => self.images.remove_all(),
            Some(Action::Reveal(path)) => {
                if let Err(err) = bamboo::files::reveal(&path) {
                    dialog("Bamboo", &format!("Could not show the file.\n{err}"));
                }
            }
            None => {}
        }
    }
}

struct Row {
    id: u64,
    path: PathBuf,
    name: String,
    status: Status,
    message: Option<String>,
}

enum Action {
    Remove(u64),
    RemoveAll,
    Reveal(PathBuf),
}

fn image_menu(ui: &mut egui::Ui, row: &Row, action: &mut Option<Action>) {
    ui.set_min_width(180.0);
    if ui
        .add_enabled(row.status != Status::Loading, egui::Button::new("Remove"))
        .clicked()
    {
        *action = Some(Action::Remove(row.id));
    }
    if ui.button("Show in Folder").clicked() {
        *action = Some(Action::Reveal(row.path.clone()));
    }
    ui.separator();
    if ui.button("Remove All").clicked() {
        *action = Some(Action::RemoveAll);
    }
}

fn compress(key: &str, path: &std::path::Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(path).map_err(|_| "Could not read file!".to_string())?;
    let compressed = bamboo::tinify::shrink(key, &bytes)?;
    bamboo::files::replace_with(path, &compressed)
}

fn style_ui(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    });
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = BG_BOTTOM;
    visuals.override_text_color = Some(TEXT);
    visuals.hyperlink_color = CYAN_HOVER;
    visuals.selection.bg_fill = CYAN;
    visuals.window_corner_radius = egui::CornerRadius::same(6);
    ctx.set_visuals(visuals);
}

fn key_field(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    egui::Frame::new()
        .fill(egui::Color32::from_white_alpha(210))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(20, 30, 30));
            let mut edit = egui::TextEdit::singleline(text)
                .font(egui::FontId::monospace(18.0))
                .align(egui::Align2::CENTER_CENTER)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY);
            if !hint.is_empty() {
                edit = edit.hint_text(hint);
            }
            edit.show(ui);
        });
}

fn green_button(ui: &mut egui::Ui, label: &str) -> bool {
    let response = ui.add_sized(
        [100.0, 30.0],
        egui::Button::new(
            egui::RichText::new(label)
                .color(egui::Color32::WHITE)
                .size(14.0),
        )
        .fill(DONE_GREEN)
        .corner_radius(egui::CornerRadius::same(2)),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(response.rect, 2.0, DONE_GREEN_HOVER);
        ui.painter().text(
            response.rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
    }
    response.clicked()
}

fn link_line(ui: &mut egui::Ui, link: &str, rest: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (leading, _) = ui.allocate_space(egui::vec2(
            (ui.available_width() - 220.0).max(0.0) / 2.0,
            0.0,
        ));
        let _ = leading;
        if ui
            .link(egui::RichText::new(link).size(14.0).color(CYAN_HOVER))
            .clicked()
        {
            clicked = true;
        }
        ui.label(
            egui::RichText::new(rest)
                .size(14.0)
                .color(egui::Color32::from_black_alpha(170)),
        );
    });
    clicked
}

fn toolbar_text_button(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    id: &str,
    label: &str,
    paint: fn(&egui::Painter, egui::Pos2, egui::Color32),
) -> egui::Response {
    let rect = egui::Rect::from_center_size(center, egui::vec2(120.0, 36.0));
    let response = ui.interact(rect, egui::Id::unique(id), egui::Sense::click());
    let color = if response.hovered() || response.has_focus() {
        CYAN_HOVER
    } else {
        ICON
    };
    paint(ui.painter(), center - egui::vec2(28.0, 0.0), color);
    ui.painter().text(
        center + egui::vec2(4.0, 1.0),
        egui::Align2::LEFT_CENTER,
        label.to_uppercase(),
        egui::FontId::proportional(14.0),
        color,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}

fn icon_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: &str,
    label: &str,
    paint: fn(&egui::Painter, egui::Pos2, egui::Color32),
) -> egui::Response {
    let response = ui.interact(rect, egui::Id::unique(id), egui::Sense::click());
    let color = if response.hovered() || response.has_focus() {
        CYAN_HOVER
    } else {
        ICON
    };
    paint(ui.painter(), rect.center(), color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}

fn plus_button(ctx: &egui::Context, center: egui::Pos2) -> bool {
    let mut clicked = false;
    egui::Area::new(egui::Id::unique("add-images"))
        .order(egui::Order::Foreground)
        .fixed_pos(center - egui::vec2(32.5, 32.5))
        .fade_in(false)
        .show(ctx, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(65.0, 65.0), egui::Sense::click());
            let center = rect.center();
            let fill = if response.hovered() || response.has_focus() {
                CYAN_HOVER
            } else {
                CYAN
            };
            ui.painter().circle_filled(center, 32.5, fill);
            paint_plus(ui.painter(), center, egui::Color32::WHITE);
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Add images"));
            response.clone().on_hover_text("Add images");
            clicked = response.clicked();
        });
    clicked
}

fn square(center: egui::Pos2, size: f32) -> egui::Rect {
    egui::Rect::from_center_size(center, egui::vec2(size, size))
}

fn drop_hint(ui: &mut egui::Ui, rect: egui::Rect, color: egui::Color32) {
    let text_pos = egui::pos2(rect.center().x + 16.0, rect.center().y);
    let galley = ui.painter().layout_no_wrap(
        "Drop a PNG, JPG, or folder here!".to_owned(),
        egui::FontId::proportional(16.0),
        color,
    );
    let drop_center = egui::pos2(text_pos.x - galley.size().x / 2.0 - 18.0, rect.center().y);
    paint_droplet(ui.painter(), drop_center, 9.0, color);
    ui.painter().galley(
        egui::pos2(
            text_pos.x - galley.size().x / 2.0,
            text_pos.y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
}

fn paint_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    top: egui::Color32,
    bottom: egui::Color32,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    mesh.vertices.extend([
        egui::epaint::Vertex::untextured(rect.left_top(), top),
        egui::epaint::Vertex::untextured(rect.right_top(), top),
        egui::epaint::Vertex::untextured(rect.right_bottom(), bottom),
        egui::epaint::Vertex::untextured(rect.left_bottom(), bottom),
    ]);
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(mesh);
}

fn paint_status(ui: &egui::Ui, center: egui::Pos2, status: Status) {
    match status {
        Status::Loading => paint_loader(ui.painter(), center, ui.input(|input| input.time)),
        Status::Done => {
            ui.painter()
                .circle_stroke(center, 8.0, egui::Stroke::new(1.6, OK_GREEN));
            paint_check(ui.painter(), center, OK_GREEN);
        }
        Status::Error => {
            ui.painter()
                .circle_stroke(center, 8.0, egui::Stroke::new(1.6, ERR_RED));
            paint_x(ui.painter(), center, 4.0, ERR_RED);
        }
    }
}

fn paint_loader(painter: &egui::Painter, center: egui::Pos2, time: f64) {
    let stroke = egui::Stroke::new(2.0, LOAD_ORANGE);
    let start = time as f32 * 5.0;
    let sweep = 4.2;
    let steps = 18;
    for step in 0..steps {
        let a0 = start + sweep * step as f32 / steps as f32;
        let a1 = start + sweep * (step + 1) as f32 / steps as f32;
        painter.line_segment(
            [
                center + egui::vec2(a0.cos(), a0.sin()) * 8.0,
                center + egui::vec2(a1.cos(), a1.sin()) * 8.0,
            ],
            stroke,
        );
    }
}

fn paint_check(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.8, color);
    painter.line_segment(
        [
            center + egui::vec2(-4.5, 0.0),
            center + egui::vec2(-1.5, 3.2),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center + egui::vec2(-1.5, 3.2),
            center + egui::vec2(5.0, -3.5),
        ],
        stroke,
    );
}

fn paint_x(painter: &egui::Painter, center: egui::Pos2, r: f32, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.8, color);
    painter.line_segment(
        [center + egui::vec2(-r, -r), center + egui::vec2(r, r)],
        stroke,
    );
    painter.line_segment(
        [center + egui::vec2(-r, r), center + egui::vec2(r, -r)],
        stroke,
    );
}

fn paint_plus(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    let stroke = egui::Stroke::new(2.4, color);
    painter.line_segment(
        [
            center + egui::vec2(-8.0, 0.0),
            center + egui::vec2(8.0, 0.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            center + egui::vec2(0.0, -8.0),
            center + egui::vec2(0.0, 8.0),
        ],
        stroke,
    );
}

fn paint_help(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    painter.circle_stroke(center, 9.0, egui::Stroke::new(1.6, color));
    painter.text(
        center + egui::vec2(0.0, 1.0),
        egui::Align2::CENTER_CENTER,
        "?",
        egui::FontId::proportional(14.0),
        color,
    );
}

fn paint_gear(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.5, color);
    let teeth = 6;
    let sector = std::f32::consts::TAU / teeth as f32;
    let outer = 9.0;
    let valley = 6.3;
    let mut points = Vec::with_capacity(teeth * 4);
    for tooth in 0..teeth {
        let base = tooth as f32 * sector - std::f32::consts::FRAC_PI_2;
        for (turn, radius) in [
            (0.12, valley),
            (0.28, outer),
            (0.72, outer),
            (0.88, valley),
        ] {
            let angle = base + sector * turn;
            points.push(center + egui::vec2(angle.cos(), angle.sin()) * radius);
        }
    }
    painter.add(egui::Shape::closed_line(points, stroke));
    painter.circle_stroke(center, 2.4, stroke);
}

fn paint_alert(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: egui::Color32) {
    painter.circle_stroke(center, radius, egui::Stroke::new(2.0, color));
    painter.text(
        center + egui::vec2(0.0, 1.0),
        egui::Align2::CENTER_CENTER,
        "!",
        egui::FontId::proportional(radius),
        color,
    );
}

fn paint_droplet(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.6, color);
    let circle = center + egui::vec2(0.0, radius * 0.28);
    let tip = center + egui::vec2(0.0, -radius * 1.25);
    let distance = tip.distance(circle);
    if distance <= radius {
        painter.circle_stroke(circle, radius, stroke);
        return;
    }
    let toward = (circle - tip).normalized();
    let base = toward.angle();
    let offset = (radius / distance).asin();
    let length = (distance * distance - radius * radius).sqrt();
    let left = tip + egui::Vec2::angled(base - offset) * length;
    let right = tip + egui::Vec2::angled(base + offset) * length;
    painter.line_segment([tip, left], stroke);
    painter.line_segment([tip, right], stroke);
    painter.circle_stroke(circle, radius, stroke);
}

fn paint_save(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.6, color);
    let rect = egui::Rect::from_center_size(center, egui::vec2(14.0, 16.0));
    painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
    painter.line_segment(
        [
            egui::pos2(rect.left() + 3.0, rect.top()),
            egui::pos2(rect.left() + 3.0, rect.top() + 5.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(rect.right() - 3.0, rect.top()),
            egui::pos2(rect.right() - 3.0, rect.top() + 5.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(rect.left() + 3.0, rect.top() + 5.0),
            egui::pos2(rect.right() - 3.0, rect.top() + 5.0),
        ],
        stroke,
    );
}

fn paint_close(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    painter.circle_stroke(center, 8.0, egui::Stroke::new(1.6, color));
    paint_x(painter, center, 3.5, color);
}

fn show_about() {
    let description = format!(
        "Bamboo v{VERSION}\nCreated by Chris Anselmo\nRewritten in Rust by Tamás Varga and Grok\n\nBamboo is not affiliated with nor endorsed by Tinify/TinyPNG/TinyJPG."
    );
    let result = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title("Bamboo - About")
        .set_description(description)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            "Dismiss".into(),
            "View Source".into(),
        ))
        .show();
    if result == rfd::MessageDialogResult::Custom("View Source".into()) {
        open_url(SOURCE_URL);
    }
}

fn dialog(title: &str, description: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::OkCustom("Dismiss".into()))
        .show();
}

fn open_url(url: &str) {
    if let Err(err) = open::that(url) {
        dialog("Bamboo", &format!("Could not open {url}\n{err}"));
    }
}
