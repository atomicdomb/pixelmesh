//! Pixel-art paint tab: edits the texture in place, so every stroke shows up
//! on the model straight away. Shares its view (zoom / pan) with the UV tab.

use crate::app::App;
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, Sense, Stroke};
use glam::Vec3;
use glam::Vec2;
use pixelmesh::icons::Icon;
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
    /// Rectangular selection.
    Select,
    /// Select the connected area of one colour.
    Wand,
    /// Drag the selected pixels somewhere else.
    Move,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelOp {
    Replace,
    Add,
    Subtract,
}

/// Combine a new region with the selection that was there before.
fn combine(base: &Option<Vec<bool>>, region: Vec<bool>, op: SelOp) -> Option<Vec<bool>> {
    let merged: Vec<bool> = match (base, op) {
        (Some(b), SelOp::Add) => b.iter().zip(&region).map(|(a, r)| *a || *r).collect(),
        // Removing from "nothing selected" means removing from everything.
        (None, SelOp::Subtract) => region.iter().map(|r| !*r).collect(),
        (Some(b), SelOp::Subtract) => b.iter().zip(&region).map(|(a, r)| *a && !*r).collect(),
        _ => region,
    };
    merged.iter().any(|m| *m).then_some(merged)
}

const TOOLS: [(Tool, Icon, &str, &str); 9] = [
    (Tool::Pencil, Icon::Pencil, "Pencil", "Pencil  (B)\nLeft button paints the foreground colour, right button the background colour"),
    (Tool::Eraser, Icon::Eraser, "Eraser", "Eraser  (E)\nMakes pixels transparent"),
    (Tool::Fill, Icon::Fill, "Fill", "Fill  (G)\nFills the connected area of one colour"),
    (Tool::Picker, Icon::Picker, "Picker", "Colour picker  (I)\nHolding Alt with any tool does the same"),
    (Tool::Line, Icon::Line, "Line", "Line  (L)\nDrag to draw a straight line"),
    (Tool::Rect, Icon::Rect, "Rect", "Rectangle  (U)\nDrag to draw a rectangle"),
    (Tool::Select, Icon::BoxSelect, "Select", "Select  (M)\nDrag a rectangle; painting then only affects the selected pixels.\nShift adds, Ctrl removes, a single click deselects"),
    (Tool::Wand, Icon::Wand, "Wand", "Magic wand  (W)\nSelects the connected area of one colour. Shift adds, Ctrl removes"),
    (Tool::Move, Icon::Move, "Move", "Move  (V)\nDrag the selected pixels to a new place (the whole image if nothing is selected).\nHold Ctrl while starting the drag to leave a copy behind. The arrow keys nudge by one pixel."),
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
    /// The stroke's own pixels (pencil, line, rectangle), laid over `base` every frame.
    layer: Vec<u8>,
    /// Selection tools: how the new region combines, and the selection before the drag.
    op: SelOp,
    base_selection: Option<Vec<bool>>,
    moved: bool,
    /// At least one sample has been applied.
    started: bool,
    /// Painted in the 3D view: (last cursor position, last surface hit as object and face).
    on_model: Option<(Pos2, Option<(usize, usize)>)>,
    /// Move tool: the pixels being carried, and the image they were lifted out of.
    carried: Option<(pixels::Clip, Vec<u8>)>,
    /// Move tool: leave the original pixels where they were.
    copy: bool,
    /// Move tool: where the carried pixels currently sit.
    carried_at: (i32, i32),
}

/// Pixels that were just moved or pasted and can be moved again without disturbing what is under them.
struct Float {
    clip: pixels::Clip,
    /// The image without the floating pixels.
    under: Vec<u8>,
    /// Texture version right after they were put down: any other edit ends the float.
    version: u64,
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
    /// Brush opacity, 0..1: multiplies the colour's own alpha.
    pub opacity: f32,
    /// Lay paint over what is there (shading) instead of replacing the pixel outright.
    pub blend: bool,
    pub show_uv: bool,
    pub show_grid: bool,
    stroke: Option<StrokeState>,
    new_dialog: Option<NewCanvas>,
    /// Colours found in the image, and the texture version they were collected from.
    used: (u64, Vec<Rgba>),
    checker: Option<egui::TextureHandle>,
    /// One flag per pixel; None = no selection, everything can be painted.
    selection: Option<Vec<bool>>,
    /// Outline of the selection on the pixel grid, rebuilt when the selection changes.
    outline: Vec<[(i32, i32); 2]>,
    outline_dirty: bool,
    /// Distinct UV edges of the whole scene for the overlay, with the scene hash they were built from.
    uv_edges: (u64, Vec<[Vec2; 2]>),
    /// While the Paint tab is open, the left button paints in the 3D view too.
    pub on_model: bool,
    /// Mirror strokes left-right / top-bottom.
    pub mirror: [bool; 2],
    /// Where the mirror lines are, in pixels from the left / top; negative = the middle of the image.
    pub mirror_at: [i32; 2],
    /// The starter swatches: the built-in ones, or an imported palette.
    pub palette: Vec<Rgba>,
    /// Pixels cut or copied.
    clip: Option<pixels::Clip>,
    /// Pixel under the mouse in the Paint tab, if it is there.
    hover: Option<(i32, i32)>,
    float: Option<Float>,
}

impl Default for Paint {
    fn default() -> Self {
        Paint {
            tool: Tool::Pencil,
            fg: [0, 0, 0, 255],
            bg: [255, 255, 255, 255],
            brush: 1,
            filled: false,
            opacity: 1.0,
            blend: true,
            show_uv: true,
            show_grid: true,
            stroke: None,
            new_dialog: None,
            used: (0, Vec::new()),
            checker: None,
            selection: None,
            outline: Vec::new(),
            outline_dirty: false,
            uv_edges: (0, Vec::new()),
            on_model: false,
            mirror: [false; 2],
            mirror_at: [-1; 2],
            palette: DEFAULT_PALETTE.to_vec(),
            clip: None,
            hover: None,
            float: None,
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

    fn set_selection(&mut self, selection: Option<Vec<bool>>) {
        self.selection = selection;
        self.outline_dirty = true;
        self.float = None;
    }

    /// Position of mirror line `k` (0 = the vertical line, 1 = the horizontal one) for an image `size` pixels across.
    fn mirror_line(&self, k: usize, size: u32) -> i32 {
        if self.mirror_at[k] < 0 { size as i32 / 2 } else { self.mirror_at[k].min(size as i32) }
    }
}

/// Read a palette from a file: an image (its colours, in order of appearance) or a
/// GIMP .gpl / JASC .pal / .hex / paint.net .txt list.
pub fn load_palette_file(path: &std::path::Path) -> Result<Vec<Rgba>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let is_image = path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg", "bmp", "tga"].contains(&e.to_ascii_lowercase().as_str()));
    let colors = if is_image {
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
        pixels::image_palette(img.as_raw(), 256)
    } else {
        pixels::parse_palette(&String::from_utf8_lossy(&bytes))
    };
    if colors.is_empty() { Err("no colours found in it".into()) } else { Ok(colors) }
}

fn color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

impl App {
    fn paint_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let w = ((ui.available_width() - 4.0 * 8.0) / 9.0).floor().min(64.0);
            for (tool, icon, label, tip) in TOOLS {
                if self.tile(ui, icon, label, self.paint.tool == tool, w).on_hover_text(tip).clicked() {
                    self.paint.tool = tool;
                }
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
            ui.label("Opacity");
            let mut percent = (self.paint.opacity * 100.0).round();
            if ui.add(egui::DragValue::new(&mut percent).range(1.0..=100.0).suffix("%"))
                .on_hover_text("How strongly the brush paints. At 10% each stroke tints what is underneath a little: use black to shade, white to highlight.\nMultiplies the alpha of the chosen colour.")
                .changed()
            {
                self.paint.opacity = percent / 100.0;
            }
            for preset in [10.0, 25.0, 50.0, 100.0] {
                if ui.selectable_label(percent == preset, format!("{preset:.0}")).clicked() {
                    self.paint.opacity = preset / 100.0;
                }
            }
            ui.checkbox(&mut self.paint.blend, "Blend")
                .on_hover_text("On: see-through paint is laid over the pixel underneath (each pixel once per stroke).\nOff: the pixel is replaced by the colour and its alpha exactly, for writing real transparency into the image.");
        });
        let size = self.texture.as_ref().map(|t| t.size);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.paint.on_model, "Paint on model")
                .on_hover_text("While the Paint tab is open, paint straight onto the model in the 3D view with the left button (pencil, eraser, fill and picker).\nOrbit with the right or middle button as usual. Untick to select and move things in the 3D view again.");
            ui.separator();
            ui.label("Mirror");
            for (k, label, tip) in [(0, "X", "Mirror strokes left to right, across a vertical line"), (1, "Y", "Mirror strokes top to bottom, across a horizontal line")] {
                ui.toggle_value(&mut self.paint.mirror[k], label).on_hover_text(tip);
                if let (true, Some(size)) = (self.paint.mirror[k], size) {
                    let mut at = self.paint.mirror_line(k, size[k]);
                    if ui.add(egui::DragValue::new(&mut at).range(0..=size[k] as i32).suffix(" px"))
                        .on_hover_text("Where the mirror line is. Double-click and type, or drag. Right-click to put it back in the middle.")
                        .secondary_clicked()
                    {
                        self.paint.mirror_at[k] = -1;
                    } else if at != self.paint.mirror_line(k, size[k]) {
                        self.paint.mirror_at[k] = at;
                    }
                }
            }
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

        // Selection commands.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label("Selection");
            ui.add_enabled_ui(size.is_some(), |ui| {
                if ui.small_button("All").on_hover_text("Ctrl+A").clicked() {
                    self.paint_select_all();
                }
                if ui.small_button("None").on_hover_text("Ctrl+D or Esc").clicked() {
                    self.paint.set_selection(None);
                }
                if ui.small_button("Invert").on_hover_text("Ctrl+Shift+I").clicked() {
                    self.paint_invert_selection();
                }
                if ui.small_button("From UV faces").on_hover_text("Select the pixels covered by the faces shown in the UV tab, so you can paint just those faces. Shift adds to the selection.").clicked() {
                    let add = ui.input(|i| i.modifiers.shift);
                    self.paint_select_uv_faces(add);
                }
            });
            if let Some(sel) = &self.paint.selection {
                ui.weak(format!("{} px", sel.iter().filter(|s| **s).count()));
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label("Pixels");
            ui.add_enabled_ui(size.is_some(), |ui| {
                let ctx = ui.ctx().clone();
                if ui.small_button("Copy").on_hover_text("Copy the selected pixels (the whole image if nothing is selected)  ·  Ctrl+C").clicked() {
                    self.paint_clipboard(&ctx, true, false, false);
                }
                if ui.small_button("Cut").on_hover_text("Ctrl+X").clicked() {
                    self.paint_clipboard(&ctx, false, true, false);
                }
                if ui.add_enabled(self.paint.clip.is_some(), egui::Button::new("Paste").small())
                    .on_hover_text("Paste under the mouse (or where it was copied from) and switch to the Move tool to place it  ·  Ctrl+V").clicked()
                {
                    self.paint_clipboard(&ctx, false, false, true);
                }
            });
            ui.separator();
            if ui.small_button("Palette…").on_hover_text("Load a palette: GIMP .gpl, JASC .pal, .hex, paint.net .txt, or any image (its colours are used).\nAseprite exports all of these from its palette menu. The palette is remembered for next time.").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Palette or image", &["gpl", "pal", "hex", "txt", "png", "jpg", "jpeg", "bmp", "tga"]).pick_file() {
                    match load_palette_file(&p) {
                        Ok(colors) => {
                            self.status = format!("Palette: {} colours from {}", colors.len(), p.display());
                            self.paint.palette = colors;
                            self.settings.palette = Some(p.to_string_lossy().into_owned());
                            self.settings.save();
                        }
                        Err(e) => self.status = format!("Could not read a palette from {}: {e}", p.display()),
                    }
                }
            }
            if ui.add_enabled(self.settings.palette.is_some(), egui::Button::new("Default").small()).on_hover_text("Go back to the built-in palette").clicked() {
                self.paint.palette = DEFAULT_PALETTE.to_vec();
                self.settings.palette = None;
                self.settings.save();
            }
        });

        // Palette: fixed starter colours, then the colours already in the image.
        if let Some(t) = &self.texture {
            if self.paint.used.0 != t.version && self.paint.stroke.is_none() {
                self.paint.used = (t.version, pixels::used_colors(&t.rgba, 32));
            }
        }
        let used = self.paint.used.1.clone();
        let palette = self.paint.palette.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            for c in palette.iter().copied().chain(used.into_iter().filter(|c| !palette.contains(c))) {
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

    fn paint_select_all(&mut self) {
        if let Some(t) = &self.texture {
            let all = vec![true; (t.size[0] * t.size[1]) as usize];
            self.paint.set_selection(Some(all));
        }
    }

    fn paint_invert_selection(&mut self) {
        let Some(t) = &self.texture else { return };
        let inverted = match &self.paint.selection {
            Some(sel) => combine(&None, sel.iter().map(|s| !*s).collect(), SelOp::Replace),
            // Inverting "nothing" selects everything.
            None => Some(vec![true; (t.size[0] * t.size[1]) as usize]),
        };
        self.paint.set_selection(inverted);
    }

    /// Select the pixels under the UVs of the faces the UV tab is showing.
    fn paint_select_uv_faces(&mut self, add: bool) {
        let Some(t) = &self.texture else { return };
        let (w, h) = (t.size[0] as i32, t.size[1] as i32);
        let faces = self.active_faces();
        let mut region = vec![false; (w * h) as usize];
        for &f in &faces {
            // Face outline in pixel coordinates (row 0 at the top).
            let poly: Vec<Vec2> = self.mesh().faces[f].uvs.iter().map(|uv| Vec2::new(uv.x * w as f32, (1.0 - uv.y) * h as f32)).collect();
            let lo = poly.iter().fold(Vec2::splat(f32::MAX), |a, p| a.min(*p));
            let hi = poly.iter().fold(Vec2::splat(f32::MIN), |a, p| a.max(*p));
            for y in (lo.y.floor() as i32).max(0)..(hi.y.ceil() as i32).min(h) {
                for x in (lo.x.floor() as i32).max(0)..(hi.x.ceil() as i32).min(w) {
                    // A pixel belongs to the face when its centre is inside the outline.
                    if point_in_polygon(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), &poly) {
                        region[(y * w + x) as usize] = true;
                    }
                }
            }
        }
        if !region.iter().any(|r| *r) {
            self.status = "No pixels under those faces: select an object or faces in the 3D view first.".into();
            return;
        }
        let op = if add { SelOp::Add } else { SelOp::Replace };
        let selection = combine(&self.paint.selection, region, op);
        self.paint.set_selection(selection);
    }

    /// Delete key: make the selected pixels transparent.
    fn paint_clear_selection(&mut self) {
        let Some(sel) = self.paint.selection.clone() else { return };
        let Some(t) = self.texture.as_mut() else { return };
        let before = t.rgba.clone();
        let size = t.size;
        let data = Arc::make_mut(&mut t.rgba);
        for (k, _) in sel.iter().enumerate().filter(|(_, s)| **s) {
            data[k * 4..k * 4 + 4].copy_from_slice(&[0, 0, 0, 0]);
        }
        if *t.rgba != *before {
            self.push_pixels_undo(before, size);
            self.texture_changed();
        }
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

        // A selection made for another image size no longer means anything.
        if self.paint.selection.as_ref().is_some_and(|s| s.len() != (w * h) as usize) {
            self.paint.set_selection(None);
        }

        // ---- keys
        let keyboard_free = !ctx.egui_wants_keyboard_input();
        if resp.hovered() && keyboard_free && self.paint.stroke.is_none() {
            let key = |k: Key| ui.input(|i| i.key_pressed(k) && !i.modifiers.command);
            let ctrl = |k: Key, shift: bool| ui.input(|i| i.key_pressed(k) && i.modifiers.command && i.modifiers.shift == shift);
            if ctrl(Key::A, false) {
                self.paint_select_all();
            } else if ctrl(Key::D, false) || key(Key::Escape) {
                self.paint.set_selection(None);
            } else if ctrl(Key::I, true) {
                self.paint_invert_selection();
            } else if key(Key::Delete) || key(Key::Backspace) {
                self.paint_clear_selection();
            }
            self.paint_tool_keys(ui);
            for (k, dx, dy) in [(Key::ArrowLeft, -1, 0), (Key::ArrowRight, 1, 0), (Key::ArrowUp, 0, -1), (Key::ArrowDown, 0, 1)] {
                if key(k) {
                    self.paint_nudge(dx, dy);
                }
            }
        }

        // ---- start a stroke
        self.paint.hover = resp.hover_pos().map(pixel_at);
        let (pressed_primary, pressed_secondary, primary_down, secondary_down) = ui.input(|i| {
            (i.pointer.button_pressed(PointerButton::Primary), i.pointer.button_pressed(PointerButton::Secondary),
             i.pointer.button_down(PointerButton::Primary), i.pointer.button_down(PointerButton::Secondary))
        });
        if self.paint.stroke.is_none() && resp.hovered() && !space && (pressed_primary || pressed_secondary) {
            if let Some(mouse) = mouse {
                let secondary = pressed_secondary && !pressed_primary;
                let tool = if mods.alt { Tool::Picker } else { self.paint.tool };
                self.begin_stroke(tool, secondary, pixel_at(mouse), mods, None);
            }
        }

        // ---- continue / finish the stroke (strokes made in the 3D view are handled there)
        if self.paint.stroke.as_ref().is_some_and(|s| s.on_model.is_none()) {
            let mut stroke = self.paint.stroke.take().unwrap();
            // The stroke lasts as long as the button that started it is held.
            let held = if stroke.secondary { secondary_down } else { primary_down };
            if let Some(mouse) = mouse {
                let cur = pixel_at(mouse);
                if stroke.last != Some(cur) && self.stroke_sample(&mut stroke, cur, true) {
                    self.texture_changed();
                }
            }
            if held {
                self.paint.stroke = Some(stroke);
                ctx.request_repaint();
            } else {
                self.end_stroke(stroke);
            }
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

        // ---- UV overlay: every visible object's UV edges; the faces shown in the UV tab are brighter
        if self.paint.show_uv {
            // Many objects share the same spots on a sprite sheet, so the overlay is the set of
            // distinct UV edges, rebuilt only when some mesh changes.
            let key = self.objects.iter().filter(|o| o.visible).fold(7, |h, o| pixelmesh::mesh::mix(h, o.mesh.fingerprint()));
            if self.paint.uv_edges.0 != key {
                let mut unique: std::collections::HashSet<[i32; 4]> = std::collections::HashSet::new();
                let q = |v: Vec2| ((v.x * 65536.0).round() as i32, (v.y * 65536.0).round() as i32);
                for obj in self.objects.iter().filter(|o| o.visible) {
                    for face in &obj.mesh.faces {
                        for k in 0..face.uvs.len() {
                            let (a, b) = (q(face.uvs[k]), q(face.uvs[(k + 1) % face.uvs.len()]));
                            let (a, b) = if a <= b { (a, b) } else { (b, a) };
                            unique.insert([a.0, a.1, b.0, b.1]);
                        }
                    }
                }
                let edges = unique.into_iter().map(|e| [Vec2::new(e[0] as f32, e[1] as f32) / 65536.0, Vec2::new(e[2] as f32, e[3] as f32) / 65536.0]).collect();
                self.paint.uv_edges = (key, edges);
            }
            let visible = rect.expand(2.0);
            let under = Stroke::new(3.0, Color32::from_rgba_unmultiplied(0, 0, 0, 50));
            let over = Stroke::new(1.0, Color32::from_rgba_unmultiplied(170, 205, 255, 150));
            for [a, b] in &self.paint.uv_edges.1 {
                let (a, b) = (view.to_screen(*a), view.to_screen(*b));
                if visible.intersects(Rect::from_two_pos(a, b)) {
                    // Dark underlay so the line reads on light and dark pixels alike.
                    painter.line_segment([a, b], under);
                    painter.line_segment([a, b], over);
                }
            }
            if let Some(obj) = self.objects.get(self.active).filter(|o| o.visible) {
                for fi in self.active_faces() {
                    let pts: Vec<Pos2> = obj.mesh.faces[fi].uvs.iter().map(|&uv| view.to_screen(uv)).collect();
                    painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(3.0, Color32::from_rgba_unmultiplied(0, 0, 0, 110))));
                    painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0, Color32::from_rgb(255, 170, 60))));
                }
            }
        }

        // ---- mirror lines
        for k in 0..2 {
            if !self.paint.mirror[k] {
                continue;
            }
            let at = self.paint.mirror_line(k, size[k]) as f32;
            let (a, b) = if k == 0 {
                (view.to_screen(Vec2::new(at / w as f32, 0.0)), view.to_screen(Vec2::new(at / w as f32, 1.0)))
            } else {
                (view.to_screen(Vec2::new(0.0, 1.0 - at / h as f32)), view.to_screen(Vec2::new(1.0, 1.0 - at / h as f32)))
            };
            painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(120)));
            painter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.5, Color32::from_rgb(120, 220, 255)), 6.0, 4.0));
        }

        // ---- selection outline: dark line with light dashes on top, so it shows on any colour
        if self.paint.outline_dirty {
            self.paint.outline = match &self.paint.selection {
                Some(sel) if sel.len() == (w * h) as usize => pixels::mask_outline(sel, w, h),
                _ => Vec::new(),
            };
            self.paint.outline_dirty = false;
        }
        let corner = |(x, y): (i32, i32)| view.to_screen(Vec2::new(x as f32 / w as f32, 1.0 - y as f32 / h as f32));
        for seg in &self.paint.outline {
            let (a, b) = (corner(seg[0]), corner(seg[1]));
            if !rect.expand(2.0).intersects(Rect::from_two_pos(a, b)) {
                continue;
            }
            painter.line_segment([a, b], Stroke::new(1.5, Color32::from_black_alpha(220)));
            painter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.5, Color32::WHITE), 4.0, 4.0));
        }

        // ---- brush outline and readout
        if let Some(p) = resp.hover_pos() {
            let (x, y) = pixel_at(p);
            let tool = if mods.alt { Tool::Picker } else { self.paint.tool };
            let n = if matches!(tool, Tool::Fill | Tool::Picker | Tool::Select | Tool::Wand | Tool::Move) { 1 } else { self.paint.brush as i32 };
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
    /// Tool and brush shortcuts shared by the Paint tab and painting on the model.
    pub fn paint_tool_keys(&mut self, ui: &egui::Ui) {
        let key = |k: Key| ui.input(|i| i.key_pressed(k) && !i.modifiers.command);
        for (k, tool) in [(Key::B, Tool::Pencil), (Key::E, Tool::Eraser), (Key::G, Tool::Fill), (Key::I, Tool::Picker), (Key::L, Tool::Line), (Key::U, Tool::Rect), (Key::M, Tool::Select), (Key::W, Tool::Wand), (Key::V, Tool::Move)] {
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

    pub fn load_saved_palette(&mut self) {
        if let Some(path) = self.settings.palette.clone() {
            if let Ok(colors) = load_palette_file(std::path::Path::new(&path)) {
                self.paint.palette = colors;
            }
        }
    }

    fn begin_stroke(&mut self, tool: Tool, secondary: bool, start: (i32, i32), mods: egui::Modifiers, on_model: Option<(Pos2, Option<(usize, usize)>)>) {
        let Some(t) = &self.texture else { return };
        let mut color = match tool {
            Tool::Eraser => [0, 0, 0, 0],
            _ if secondary => self.paint.bg,
            _ => self.paint.fg,
        };
        color[3] = (color[3] as f32 * self.paint.opacity).round() as u8;
        let op = if mods.command { SelOp::Subtract } else if mods.shift { SelOp::Add } else { SelOp::Replace };
        self.paint.stroke = Some(StrokeState {
            tool, color, secondary, start, last: None, base: t.rgba.clone(), size: t.size,
            layer: vec![0; t.rgba.len()],
            op, base_selection: self.paint.selection.clone(), moved: false,
            started: false, on_model, carried: None, copy: mods.command, carried_at: (0, 0),
        });
    }

    /// Apply the stroke at pixel `cur`. `connect` joins it to the previous sample with a line
    /// (off when the brush jumped to another part of the texture). Returns true if pixels changed.
    fn stroke_sample(&mut self, stroke: &mut StrokeState, cur: (i32, i32), connect: bool) -> bool {
        let Some(t) = self.texture.as_mut() else { return false };
        let (w, h) = (t.size[0], t.size[1]);
        let brush = self.paint.brush;
        let mask = self.paint.selection.as_deref();
        let first = !stroke.started;
        stroke.moved |= stroke.started;
        stroke.started = true;
        let from = if connect { stroke.last.unwrap_or(cur) } else { cur };
        // The stroke as drawn, plus its mirror images.
        let (mx, my) = (self.paint.mirror[0] && stroke.on_model.is_none(), self.paint.mirror[1] && stroke.on_model.is_none());
        let (ax, ay) = (self.paint.mirror_line(0, w), self.paint.mirror_line(1, h));
        let mut copies: Vec<(bool, bool)> = vec![(false, false)];
        if mx {
            copies.push((true, false));
        }
        if my {
            copies.push((false, true));
        }
        if mx && my {
            copies.push((true, true));
        }
        let place = |p: (i32, i32), (fx, fy): (bool, bool), size: u32| {
            (if fx { pixels::mirror_coord(p.0, ax, size) } else { p.0 }, if fy { pixels::mirror_coord(p.1, ay, size) } else { p.1 })
        };
        let mut changed = false;
        let mut new_selection = None;
        match stroke.tool {
            Tool::Select => {
                let (x0, x1) = (stroke.start.0.min(cur.0).max(0), stroke.start.0.max(cur.0).min(w as i32 - 1));
                let (y0, y1) = (stroke.start.1.min(cur.1).max(0), stroke.start.1.max(cur.1).min(h as i32 - 1));
                let mut region = vec![false; (w * h) as usize];
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        region[(y as u32 * w + x as u32) as usize] = true;
                    }
                }
                new_selection = Some(combine(&stroke.base_selection, region, stroke.op));
            }
            Tool::Wand => {
                if first {
                    let region = pixels::flood_region(&t.rgba, w, h, cur.0, cur.1, None);
                    new_selection = Some(combine(&stroke.base_selection, region, stroke.op));
                }
            }
            Tool::Move => {
                let float = self.paint.float.take().filter(|f| first && f.version == t.version && !stroke.copy);
                if let Some(f) = float {
                    // Still holding what was just moved or pasted: carry on with it, over the same background.
                    stroke.carried = Some((f.clip, f.under));
                } else if first {
                    // Pick the selected pixels up, leaving a hole (or, when copying, the pixels themselves).
                    if let Some(clip) = pixels::lift(&stroke.base, w, h, stroke.base_selection.as_deref()) {
                        let mut hole = stroke.base.to_vec();
                        if !stroke.copy {
                            match &stroke.base_selection {
                                Some(sel) => pixels::clear_masked(&mut hole, sel),
                                None => hole.fill(0),
                            }
                        }
                        stroke.carried = Some((clip, hole));
                    }
                }
                if let Some((clip, hole)) = &stroke.carried {
                    let at = (clip.origin.0 + cur.0 - stroke.start.0, clip.origin.1 + cur.1 - stroke.start.1);
                    let data = Arc::make_mut(&mut t.rgba);
                    data.copy_from_slice(hole);
                    let covered = pixels::drop_clip(data, w, h, clip, at);
                    stroke.carried_at = at;
                    if stroke.base_selection.is_some() {
                        new_selection = Some(covered.iter().any(|c| *c).then_some(covered));
                    }
                    changed = true;
                }
            }
            Tool::Eraser => {
                let data = Arc::make_mut(&mut t.rgba);
                for &flip in &copies {
                    pixels::line(data, w, h, place(from, flip, brush), place(cur, flip, brush), brush, [0, 0, 0, 0]);
                }
                changed = true;
            }
            Tool::Pencil | Tool::Line | Tool::Rect => {
                // The stroke is drawn on its own layer and laid over the untouched image, so
                // see-through paint tints each pixel once however often the brush crosses it.
                if stroke.tool != Tool::Pencil {
                    stroke.layer.fill(0);
                }
                for &flip in &copies {
                    match stroke.tool {
                        Tool::Pencil => pixels::line(&mut stroke.layer, w, h, place(from, flip, brush), place(cur, flip, brush), brush, stroke.color),
                        Tool::Line => pixels::line(&mut stroke.layer, w, h, place(stroke.start, flip, brush), place(cur, flip, brush), brush, stroke.color),
                        _ => pixels::rect(&mut stroke.layer, w, h, place(stroke.start, flip, 1), place(cur, flip, 1), self.paint.filled, brush, stroke.color),
                    }
                }
                let data = Arc::make_mut(&mut t.rgba);
                pixels::composite(data, &stroke.base, &stroke.layer, self.paint.blend);
                changed = true;
            }
            Tool::Fill => {
                if first {
                    let region = pixels::flood_region(&stroke.base, w, h, cur.0, cur.1, mask);
                    for (k, _) in region.iter().enumerate().filter(|(_, inside)| **inside) {
                        stroke.layer[k * 4..k * 4 + 4].copy_from_slice(&stroke.color);
                    }
                    let data = Arc::make_mut(&mut t.rgba);
                    pixels::composite(data, &stroke.base, &stroke.layer, self.paint.blend);
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
        // With a selection, whatever the tool drew outside it is put back.
        if let (true, Some(mask), true) = (changed, mask, stroke.tool != Tool::Move) {
            let data = Arc::make_mut(&mut t.rgba);
            pixels::restore_outside(data, &stroke.base, mask);
        }
        if let Some(sel) = new_selection {
            self.paint.set_selection(sel);
        }
        stroke.last = Some(cur);
        changed
    }

    fn end_stroke(&mut self, mut stroke: StrokeState) {
        if let (Some((mut clip, under)), Some(t)) = (stroke.carried.take(), &self.texture) {
            clip.origin = stroke.carried_at;
            self.paint.float = Some(Float { clip, under, version: t.version });
        }
        if stroke.tool == Tool::Select && !stroke.moved && stroke.op == SelOp::Replace {
            // A plain click with the select tool clears the selection.
            self.paint.set_selection(None);
        } else if self.texture.as_ref().is_some_and(|t| *t.rgba != *stroke.base) {
            // One undo step per stroke.
            self.push_pixels_undo(stroke.base, stroke.size);
        }
    }

    /// Arrow keys: shift the selected pixels (or the whole image) by one pixel.
    fn paint_nudge(&mut self, dx: i32, dy: i32) {
        self.begin_stroke(Tool::Move, false, (0, 0), egui::Modifiers::NONE, None);
        let Some(mut stroke) = self.paint.stroke.take() else { return };
        if self.stroke_sample(&mut stroke, (0, 0), false) | self.stroke_sample(&mut stroke, (dx, dy), false) {
            self.texture_changed();
        }
        self.end_stroke(stroke);
    }

    /// Copy, cut or paste pixels.
    pub fn paint_clipboard(&mut self, ctx: &egui::Context, copy: bool, cut: bool, paste: bool) {
        let Some(t) = self.texture.as_mut() else { return };
        let (w, h) = (t.size[0], t.size[1]);
        let selection = self.paint.selection.clone().filter(|s| s.len() == (w * h) as usize);
        if copy || cut {
            let Some(clip) = pixels::lift(&t.rgba, w, h, selection.as_deref()) else { return };
            self.status = format!("{} {}x{} pixels.", if cut { "Cut" } else { "Copied" }, clip.w, clip.h);
            self.paint.clip = Some(clip);
            // Something has to be on the system clipboard for a later Ctrl+V to reach the program.
            ctx.copy_text("PixelMesh pixels".into());
            if cut {
                let before = t.rgba.clone();
                let size = t.size;
                let data = Arc::make_mut(&mut t.rgba);
                match &selection {
                    Some(sel) => pixels::clear_masked(data, sel),
                    None => data.fill(0),
                }
                self.push_pixels_undo(before, size);
                self.texture_changed();
            }
        } else if paste {
            let Some(clip) = self.paint.clip.clone() else {
                self.status = "Nothing to paste: copy some pixels first (Ctrl+C).".into();
                return;
            };
            // Centred under the mouse when it is over the image, otherwise where it came from.
            let at = match self.paint.hover {
                Some((x, y)) => (x - clip.w as i32 / 2, y - clip.h as i32 / 2),
                None => clip.origin,
            };
            let at = (at.0.clamp(1 - clip.w as i32, w as i32 - 1), at.1.clamp(1 - clip.h as i32, h as i32 - 1));
            let before = t.rgba.clone();
            let size = t.size;
            let data: &mut Vec<u8> = Arc::make_mut(&mut t.rgba);
            let covered = pixels::drop_clip(data, w, h, &clip, at);
            self.push_pixels_undo(before.clone(), size);
            self.paint.set_selection(covered.iter().any(|c| *c).then_some(covered));
            self.paint.tool = Tool::Move;
            self.texture_changed();
            if let Some(t) = &self.texture {
                self.paint.float = Some(Float { clip: pixels::Clip { origin: at, ..clip }, under: before.to_vec(), version: t.version });
            }
            self.status = "Pasted. Drag it into place with the Move tool, or nudge it with the arrow keys.".into();
        }
    }

    // ------------------------------------------------- painting on the model

    /// The left button paints in the 3D view while the Paint tab is open and "Paint on model" is ticked.
    pub fn painting_on_model(&self) -> bool {
        self.paint.on_model && self.show_uv && self.left_tab == crate::app::LeftTab::Paint && self.texture.is_some()
    }

    /// The texture pixel on the model's surface under a point of the 3D view, and which face it is on.
    fn model_pixel(&self, rect: Rect, pos: Pos2) -> Option<((i32, i32), (usize, usize))> {
        let t = self.texture.as_ref()?;
        let (origin, dir) = self.camera.ray(rect, pos);
        let inv = Vec3::ONE / dir;
        let mut best: Option<(f32, glam::Vec2, (usize, usize))> = None;
        for (i, obj) in self.objects.iter().enumerate().filter(|(_, o)| o.visible) {
            // Skip objects whose bounding box the ray misses (plain objects only: modifiers add geometry).
            if let (false, Some((lo, hi))) = (obj.has_modifiers(), obj.bounds()) {
                let (t0, t1) = ((lo - origin) * inv, (hi - origin) * inv);
                let (near, far) = (t0.min(t1).max_element(), t0.max(t1).min_element());
                if near > far + 1e-4 || far < 0.0 {
                    continue;
                }
            }
            if let Some((f, dist, uv)) = obj.display_mesh().raycast_uv(origin, dir) {
                if best.is_none_or(|(d, _, _)| dist < d) {
                    best = Some((dist, uv, (i, f)));
                }
            }
        }
        let (_, uv, face) = best?;
        let px = ((uv.x * t.size[0] as f32).floor() as i32, ((1.0 - uv.y) * t.size[1] as f32).floor() as i32);
        Some((px, face))
    }

    /// Run the paint tools in the 3D view. Returns the texture pixel under the mouse, for the readout.
    pub fn paint_on_model(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect, mouse: Option<Pos2>, mods: egui::Modifiers) -> Option<(i32, i32)> {
        let (pressed, down) = ui.input(|i| (i.pointer.button_pressed(PointerButton::Primary), i.pointer.button_down(PointerButton::Primary)));
        let hover = mouse.filter(|m| rect.contains(*m)).and_then(|m| self.model_pixel(rect, m));
        if self.paint.stroke.is_none() && pressed && resp.hovered() && !mods.alt {
            if let (Some(mouse), Some((px, _))) = (mouse, hover) {
                let tool = match self.paint.tool {
                    Tool::Eraser | Tool::Fill | Tool::Picker => self.paint.tool,
                    Tool::Pencil | Tool::Line | Tool::Rect => Tool::Pencil,
                    Tool::Select | Tool::Wand | Tool::Move => {
                        self.status = "That tool works in the Paint tab. On the model: pencil, eraser, fill and picker.".into();
                        return hover.map(|h| h.0);
                    }
                };
                self.begin_stroke(tool, false, px, egui::Modifiers::NONE, Some((mouse, None)));
            }
        }
        if self.paint.stroke.as_ref().is_some_and(|s| s.on_model.is_some()) {
            let mut stroke = self.paint.stroke.take().unwrap();
            let mut changed = false;
            if let (Some(mouse), Some((last_pos, mut last_face))) = (mouse, stroke.on_model) {
                // Walk the cursor's path in small steps so a fast stroke leaves no gaps.
                let steps = ((mouse - last_pos).length() / 1.5).ceil().max(1.0) as usize;
                for k in if stroke.started { 1 } else { 0 }..=steps {
                    let p = last_pos + (mouse - last_pos) * (k as f32 / steps as f32);
                    match self.model_pixel(rect, p) {
                        Some((px, face)) => {
                            // Only join samples on the same face: neighbouring faces can be far apart on the texture.
                            let connect = last_face == Some(face);
                            if stroke.last != Some(px) || !connect {
                                changed |= self.stroke_sample(&mut stroke, px, connect);
                            }
                            last_face = Some(face);
                        }
                        None => last_face = None,
                    }
                }
                stroke.on_model = Some((mouse, last_face));
            }
            if changed {
                self.texture_changed();
            }
            if down {
                self.paint.stroke = Some(stroke);
                ui.ctx().request_repaint();
            } else {
                self.end_stroke(stroke);
            }
        }
        hover.map(|h| h.0)
    }
}

fn point_in_polygon(p: Vec2, poly: &[Vec2]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}
