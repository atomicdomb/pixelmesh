//! Pixel-art paint tab: edits the texture in place, so every stroke shows up
//! on the model straight away. Shares its view (zoom / pan) with the UV tab.

use crate::app::App;
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, Sense, Stroke};
use glam::Vec2;
use pixelmesh::pixels::{self, Rgba};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pencil,
    Eraser,
    Fill,
    Picker,
    Line,
    Rect,
}

const TOOLS: [(Tool, &str, &str); 6] = [
    (Tool::Pencil, "Pencil (B)", "Left button paints the foreground colour, right button the background colour"),
    (Tool::Eraser, "Eraser (E)", "Makes pixels transparent"),
    (Tool::Fill, "Fill (G)", "Fills the connected area of one colour"),
    (Tool::Picker, "Picker (I)", "Picks a colour from the image. Holding Alt with any tool does the same"),
    (Tool::Line, "Line (L)", "Drag to draw a straight line"),
    (Tool::Rect, "Rect (U)", "Drag to draw a rectangle"),
];

/// A small general-purpose starting palette.
const DEFAULT_PALETTE: [Rgba; 16] = [
    [0, 0, 0, 255], [255, 255, 255, 255], [128, 128, 128, 255], [64, 64, 72, 255],
    [190, 38, 51, 255], [235, 137, 49, 255], [247, 226, 107, 255], [163, 206, 39, 255],
    [68, 137, 26, 255], [49, 162, 242, 255], [0, 87, 132, 255], [47, 72, 78, 255],
    [178, 80, 180, 255], [224, 111, 139, 255], [164, 100, 34, 255], [73, 60, 43, 255],
];

struct StrokeState {
    tool: Tool,
    color: Rgba,
    /// Started with the right button (background colour).
    secondary: bool,
    start: (i32, i32),
    last: Option<(i32, i32)>,
    /// Pixels before the stroke: restored each frame for line / rectangle previews, and kept for undo.
    base: Arc<Vec<u8>>,
    size: [u32; 2],
}

struct NewCanvas {
    width: u32,
    height: u32,
    /// 0 = white, 1 = transparent, 2 = background colour
    fill: usize,
}

pub struct Paint {
    pub tool: Tool,
    pub fg: Rgba,
    pub bg: Rgba,
    pub brush: u32,
    pub filled: bool,
    pub show_uv: bool,
    pub show_grid: bool,
    stroke: Option<StrokeState>,
    new_dialog: Option<NewCanvas>,
    /// Colours found in the image, and the texture version they were collected from.
    used: (u64, Vec<Rgba>),
    checker: Option<egui::TextureHandle>,
}

impl Default for Paint {
    fn default() -> Self {
        Paint {
            tool: Tool::Pencil,
            fg: [0, 0, 0, 255],
            bg: [255, 255, 255, 255],
            brush: 1,
            filled: false,
            show_uv: true,
            show_grid: true,
            stroke: None,
            new_dialog: None,
            used: (0, Vec::new()),
            checker: None,
        }
    }
}

impl Paint {
    pub fn open_new_dialog(&mut self, current: Option<[u32; 2]>) {
        let [width, height] = current.unwrap_or([32, 32]);
        self.new_dialog = Some(NewCanvas { width, height, fill: 0 });
    }

    pub fn is_painting(&self) -> bool {
        self.stroke.is_some()
    }
}

fn color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

impl App {
    fn paint_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (tool, label, tip) in TOOLS {
                ui.selectable_value(&mut self.paint.tool, tool, label).on_hover_text(tip);
            }
        });
        ui.horizontal(|ui| {
            ui.color_edit_button_srgba_unmultiplied(&mut self.paint.fg).on_hover_text("Foreground colour (left button)");
            ui.color_edit_button_srgba_unmultiplied(&mut self.paint.bg).on_hover_text("Background colour (right button)");
            if ui.small_button("Swap (X)").clicked() {
                std::mem::swap(&mut self.paint.fg, &mut self.paint.bg);
            }
            ui.label("Size");
            ui.add(egui::DragValue::new(&mut self.paint.brush).range(1..=32)).on_hover_text("Brush size in pixels  ·  [ and ]");
            ui.checkbox(&mut self.paint.filled, "Filled rect");
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.paint.show_uv, "UV overlay").on_hover_text("Draw the models' UV outlines over the image");
            ui.checkbox(&mut self.paint.show_grid, "Grid");
            if ui.small_button("New…").clicked() {
                self.paint.open_new_dialog(self.texture.as_ref().map(|t| t.size));
            }
            let dirty = self.texture.as_ref().is_some_and(|t| t.dirty);
            if ui.add_enabled(self.texture.is_some(), egui::Button::new(if dirty { "Save *" } else { "Save" }).small())
                .on_hover_text("Save the image as PNG  ·  Ctrl+S").clicked()
            {
                self.save_texture();
            }
            if ui.add_enabled(self.texture.is_some(), egui::Button::new("Save as…").small()).clicked() {
                self.save_texture_as();
            }
            if ui.small_button("Fit view").clicked() {
                self.uv.fit_view();
            }
        });

        // Palette: fixed starter colours, then the colours already in the image.
        if let Some(t) = &self.texture {
            if self.paint.used.0 != t.version && self.paint.stroke.is_none() {
                self.paint.used = (t.version, pixels::used_colors(&t.rgba, 32));
            }
        }
        let used = self.paint.used.1.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            for c in DEFAULT_PALETTE.iter().copied().chain(used.into_iter().filter(|c| !DEFAULT_PALETTE.contains(c))) {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::click());
                ui.painter().rect_filled(r, 2.0, color32(c));
                let border = if c == self.paint.fg { Color32::WHITE } else { Color32::from_gray(60) };
                ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
                if resp.clicked() {
                    self.paint.fg = c;
                } else if resp.secondary_clicked() {
                    self.paint.bg = c;
                }
            }
        });
    }

    fn new_canvas_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.paint.new_dialog.take() else { return };
        let mut create = false;
        let mut open = true;
        egui::Window::new("New canvas").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Width");
                ui.add(egui::DragValue::new(&mut dlg.width).range(1..=4096).suffix(" px"));
                ui.label("Height");
                ui.add(egui::DragValue::new(&mut dlg.height).range(1..=4096).suffix(" px"));
            });
            ui.horizontal_wrapped(|ui| {
                for (w, h) in [(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (64, 32), (128, 64), (256, 128)] {
                    if ui.small_button(format!("{w}x{h}")).clicked() {
                        (dlg.width, dlg.height) = (w, h);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Fill");
                ui.radio_value(&mut dlg.fill, 0, "White");
                ui.radio_value(&mut dlg.fill, 1, "Transparent");
                ui.radio_value(&mut dlg.fill, 2, "Background colour");
            });
            if dlg.fill == 1 {
                ui.weak("Transparent pixels are see-through on the model.");
            }
            if self.texture.as_ref().is_some_and(|t| t.dirty) {
                ui.colored_label(Color32::from_rgb(255, 190, 90), "The current image has unsaved changes and will be replaced.");
            }
            ui.horizontal(|ui| {
                create = ui.button("Create").clicked();
            });
        });
        if create {
            let fill = [[255, 255, 255, 255], [0, 0, 0, 0], self.paint.bg][dlg.fill];
            self.new_canvas(ctx, dlg.width, dlg.height, fill);
        } else if open {
            self.paint.new_dialog = Some(dlg);
        }
    }

    pub fn paint_editor(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.paint_toolbar(ui);
        self.new_canvas_dialog(&ctx);

        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(34, 36, 40));
        let font = egui::FontId::proportional(12.0);

        let Some((size, tex_id)) = self.texture.as_ref().map(|t| (t.size, t.handle.id())) else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "No image yet.\nUse \"New…\" above to start a canvas,\nor load one in the Texture section.", font, Color32::from_gray(180));
            return;
        };
        let (w, h) = (size[0], size[1]);
        let tex = Vec2::new(w as f32, h as f32);

        // ---- view
        let (mods, mouse, space) = ui.input(|i| (i.modifiers, i.pointer.latest_pos(), i.key_down(Key::Space)));
        let panning = resp.dragged_by(PointerButton::Middle) || (space && resp.dragged_by(PointerButton::Primary));
        self.uv.navigate(ui, &resp, rect, panning);
        let view = self.uv.view(rect, tex);
        let sheet = view.sheet();
        let texel = self.uv.texel_size(tex);
        let pixel_at = |p: Pos2| {
            let uv = view.to_uv(p);
            ((uv.x * w as f32).floor() as i32, ((1.0 - uv.y) * h as f32).floor() as i32)
        };
        let pixel_rect = |x: i32, y: i32, n: i32| {
            let a = view.to_screen(Vec2::new(x as f32 / w as f32, 1.0 - y as f32 / h as f32));
            let b = view.to_screen(Vec2::new((x + n) as f32 / w as f32, 1.0 - (y + n) as f32 / h as f32));
            Rect::from_two_pos(a, b)
        };

        // ---- keys
        let keyboard_free = !ctx.egui_wants_keyboard_input();
        if resp.hovered() && keyboard_free && self.paint.stroke.is_none() {
            let key = |k: Key| ui.input(|i| i.key_pressed(k) && !i.modifiers.command);
            for (k, tool) in [(Key::B, Tool::Pencil), (Key::E, Tool::Eraser), (Key::G, Tool::Fill), (Key::I, Tool::Picker), (Key::L, Tool::Line), (Key::U, Tool::Rect)] {
                if key(k) {
                    self.paint.tool = tool;
                }
            }
            if key(Key::X) {
                std::mem::swap(&mut self.paint.fg, &mut self.paint.bg);
            }
            if key(Key::OpenBracket) {
                self.paint.brush = self.paint.brush.saturating_sub(1).max(1);
            }
            if key(Key::CloseBracket) {
                self.paint.brush = (self.paint.brush + 1).min(32);
            }
        }

        // ---- start a stroke
        let (pressed_primary, pressed_secondary, primary_down, secondary_down) = ui.input(|i| {
            (i.pointer.button_pressed(PointerButton::Primary), i.pointer.button_pressed(PointerButton::Secondary),
             i.pointer.button_down(PointerButton::Primary), i.pointer.button_down(PointerButton::Secondary))
        });
        if self.paint.stroke.is_none() && resp.hovered() && !space && (pressed_primary || pressed_secondary) {
            if let (Some(mouse), Some(t)) = (mouse, &self.texture) {
                let secondary = pressed_secondary && !pressed_primary;
                let tool = if mods.alt { Tool::Picker } else { self.paint.tool };
                let color = match tool {
                    Tool::Eraser => [0, 0, 0, 0],
                    _ if secondary => self.paint.bg,
                    _ => self.paint.fg,
                };
                self.paint.stroke = Some(StrokeState { tool, color, secondary, start: pixel_at(mouse), last: None, base: t.rgba.clone(), size: t.size });
            }
        }

        // ---- continue / finish the stroke
        let mut changed = false;
        if let Some(mut stroke) = self.paint.stroke.take() {
            // The stroke lasts as long as the button that started it is held.
            let held = if stroke.secondary { secondary_down } else { primary_down };
            if let (Some(mouse), Some(t)) = (mouse, self.texture.as_mut()) {
                let cur = pixel_at(mouse);
                if stroke.last != Some(cur) {
                    let brush = self.paint.brush;
                    match stroke.tool {
                        Tool::Pencil | Tool::Eraser => {
                            let data = Arc::make_mut(&mut t.rgba);
                            pixels::line(data, w, h, stroke.last.unwrap_or(cur), cur, brush, stroke.color);
                            changed = true;
                        }
                        Tool::Line | Tool::Rect => {
                            // Redraw the shape from the untouched image every frame.
                            t.rgba = stroke.base.clone();
                            let data = Arc::make_mut(&mut t.rgba);
                            if stroke.tool == Tool::Line {
                                pixels::line(data, w, h, stroke.start, cur, brush, stroke.color);
                            } else {
                                pixels::rect(data, w, h, stroke.start, cur, self.paint.filled, brush, stroke.color);
                            }
                            changed = true;
                        }
                        Tool::Fill => {
                            if stroke.last.is_none() {
                                let data = Arc::make_mut(&mut t.rgba);
                                pixels::flood_fill(data, w, h, cur.0, cur.1, stroke.color);
                                changed = true;
                            }
                        }
                        Tool::Picker => {
                            if let Some(c) = pixels::get(&t.rgba, w, h, cur.0, cur.1) {
                                if stroke.secondary {
                                    self.paint.bg = c;
                                } else {
                                    self.paint.fg = c;
                                }
                            }
                        }
                    }
                    stroke.last = Some(cur);
                }
            }
            if held {
                self.paint.stroke = Some(stroke);
                ctx.request_repaint();
            } else if self.texture.as_ref().is_some_and(|t| *t.rgba != *stroke.base) {
                // One undo step per stroke.
                self.push_pixels_undo(stroke.base, stroke.size);
            }
        }
        if changed {
            self.texture_changed();
        }

        // ---- draw: transparency checker, image, grid
        let checker = self.paint.checker.get_or_insert_with(|| {
            let (a, b) = (Color32::from_gray(150), Color32::from_gray(110));
            let img = egui::ColorImage::new([2, 2], vec![a, b, b, a]);
            let options = egui::TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..egui::TextureOptions::NEAREST };
            ctx.load_texture("checker", img, options)
        });
        // Checker squares cover a few texels each, however big the image is.
        let cell = (tex.max_element() / 16.0).ceil().max(1.0);
        painter.image(checker.id(), sheet, Rect::from_min_max(Pos2::ZERO, Pos2::new(tex.x / (2.0 * cell), tex.y / (2.0 * cell))), Color32::WHITE);
        painter.image(tex_id, sheet, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        if self.paint.show_grid && texel >= 6.0 {
            let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 26));
            let visible = sheet.intersect(rect);
            for i in 0..=w {
                let x = sheet.left() + i as f32 / w as f32 * sheet.width();
                if x >= visible.left() && x <= visible.right() {
                    painter.line_segment([Pos2::new(x, visible.top()), Pos2::new(x, visible.bottom())], stroke);
                }
            }
            for j in 0..=h {
                let y = sheet.top() + j as f32 / h as f32 * sheet.height();
                if y >= visible.top() && y <= visible.bottom() {
                    painter.line_segment([Pos2::new(visible.left(), y), Pos2::new(visible.right(), y)], stroke);
                }
            }
        }
        painter.rect_stroke(sheet, 0.0, Stroke::new(1.0, Color32::from_gray(150)), egui::StrokeKind::Outside);

        // ---- UV overlay: every visible object's faces; the ones shown in the UV tab are brighter
        if self.paint.show_uv {
            let active = self.active_faces();
            for (oi, obj) in self.objects.iter().enumerate() {
                if !obj.visible {
                    continue;
                }
                for (fi, face) in obj.mesh.faces.iter().enumerate() {
                    let pts: Vec<Pos2> = face.uvs.iter().map(|&uv| view.to_screen(uv)).collect();
                    let strong = oi == self.active && active.contains(&fi);
                    // Dark underlay so the line reads on light and dark pixels alike.
                    painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(3.0, Color32::from_rgba_unmultiplied(0, 0, 0, if strong { 110 } else { 50 }))));
                    let line = if strong { Color32::from_rgb(255, 170, 60) } else { Color32::from_rgba_unmultiplied(170, 205, 255, 150) };
                    painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0, line)));
                }
            }
        }

        // ---- brush outline and readout
        if let Some(p) = resp.hover_pos() {
            let (x, y) = pixel_at(p);
            let tool = if mods.alt { Tool::Picker } else { self.paint.tool };
            let n = if matches!(tool, Tool::Fill | Tool::Picker) { 1 } else { self.paint.brush as i32 };
            let off = (n - 1) / 2;
            let r = pixel_rect(x - off, y - off, n);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
            let under = self.texture.as_ref().and_then(|t| pixels::get(&t.rgba, w, h, x, y));
            let text = match under {
                Some(c) => format!("{x}, {y}    #{:02X}{:02X}{:02X}  alpha {}    {w}x{h}", c[0], c[1], c[2], c[3]),
                None => format!("{w}x{h}"),
            };
            painter.text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, text, font, Color32::from_gray(210));
        }
    }
}
