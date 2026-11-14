//! 2D UV editor: shows the texture with the UV outlines of the selected faces
//! and lets you drag corners or whole faces, snapping to texture pixels.

use crate::app::App;
use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, Sense, Stroke};
use glam::Vec2;

pub struct UvEditor {
    /// Screen pixels per UV unit; 0 = fit on next frame.
    zoom: f32,
    /// Screen offset of the UV-square centre from the panel centre.
    pan: egui::Vec2,
    pub snap: bool,
    pub show_all: bool,
    drag: Option<Drag>,
}

struct Drag {
    /// (face, corner, original uv)
    corners: Vec<(usize, usize, Vec2)>,
    anchor: Vec2,
    start: Pos2,
}

impl Default for UvEditor {
    fn default() -> Self {
        UvEditor { zoom: 0.0, pan: egui::Vec2::ZERO, snap: true, show_all: true, drag: None }
    }
}

impl App {
    pub fn uv_editor(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("UV editor");
            if ui.small_button("Fit view").clicked() {
                self.uv.zoom = 0.0;
            }
            ui.weak("drag corner / face · scroll zoom · right-drag pan");
        });
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(34, 36, 40));

        if self.uv.zoom <= 0.0 {
            self.uv.zoom = rect.width().min(rect.height()) * 0.85;
            self.uv.pan = egui::Vec2::ZERO;
        }
        let tex = self.tex_size();

        // ---- view navigation
        if resp.hovered() {
            let (scroll, pinch, mouse) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos()));
            let factor = (scroll * 0.004).exp() * pinch;
            if factor != 1.0 {
                if let Some(mouse) = mouse {
                    // Zoom about the cursor.
                    let centre = rect.center() + self.uv.pan;
                    let new_zoom = (self.uv.zoom * factor).clamp(40.0, 60000.0);
                    let k = new_zoom / self.uv.zoom;
                    self.uv.pan = (mouse + (centre - mouse) * k) - rect.center();
                    self.uv.zoom = new_zoom;
                }
            }
        }
        if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) {
            self.uv.pan += resp.drag_delta();
        }
        let (zoom, origin) = (self.uv.zoom, rect.center() + self.uv.pan);
        let to_screen = |uv: Vec2| Pos2::new(origin.x + (uv.x - 0.5) * zoom, origin.y - (uv.y - 0.5) * zoom);
        let to_uv = |p: Pos2| Vec2::new((p.x - origin.x) / zoom + 0.5, 0.5 - (p.y - origin.y) / zoom);

        // ---- texture and pixel grid
        let sheet = Rect::from_two_pos(to_screen(Vec2::new(0.0, 1.0)), to_screen(Vec2::new(1.0, 0.0)));
        match &self.texture {
            Some(t) => {
                painter.rect_filled(sheet, 0.0, Color32::from_gray(70));
                painter.image(t.handle.id(), sheet, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                painter.rect_filled(sheet, 0.0, Color32::from_gray(58));
            }
        }
        let px = zoom / tex.x.max(tex.y);
        if px >= 6.0 {
            let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 22));
            let visible = sheet.intersect(rect);
            for i in 0..=tex.x as i32 {
                let x = sheet.left() + i as f32 / tex.x * sheet.width();
                if x >= visible.left() && x <= visible.right() {
                    painter.line_segment([Pos2::new(x, visible.top()), Pos2::new(x, visible.bottom())], stroke);
                }
            }
            for j in 0..=tex.y as i32 {
                let y = sheet.top() + j as f32 / tex.y * sheet.height();
                if y >= visible.top() && y <= visible.bottom() {
                    painter.line_segment([Pos2::new(visible.left(), y), Pos2::new(visible.right(), y)], stroke);
                }
            }
        }
        painter.rect_stroke(sheet, 0.0, Stroke::new(1.0, Color32::from_gray(150)), egui::StrokeKind::Outside);

        // ---- face outlines
        let active = self.active_faces();
        if self.uv.show_all {
            let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 170, 255, 70));
            for (fi, face) in self.mesh.faces.iter().enumerate() {
                if !active.contains(&fi) {
                    painter.add(egui::Shape::closed_line(face.uvs.iter().map(|&uv| to_screen(uv)).collect(), stroke));
                }
            }
        }
        for &fi in &active {
            let pts: Vec<Pos2> = self.mesh.faces[fi].uvs.iter().map(|&uv| to_screen(uv)).collect();
            painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.5, Color32::from_rgb(255, 150, 40))));
            for p in pts {
                painter.circle_filled(p, 3.5, Color32::from_rgb(255, 220, 120));
            }
        }

        // ---- dragging
        if resp.drag_started_by(PointerButton::Primary) {
            // The drag only registers after a few pixels of movement, so pick at the press position.
            if let Some(start) = ui.input(|i| i.pointer.press_origin()) {
                let mut corners = Vec::new();
                let mut anchor = Vec2::ZERO;
                // Nearest corner within reach?
                let mut best: Option<(Vec2, f32)> = None;
                for &fi in &active {
                    for &uv in &self.mesh.faces[fi].uvs {
                        let d = to_screen(uv).distance(start);
                        if d <= 9.0 && best.map_or(true, |(_, bd)| d < bd) {
                            best = Some((uv, d));
                        }
                    }
                }
                if let Some((hit, _)) = best {
                    // Corners sharing that UV position move together so islands stay stitched.
                    anchor = hit;
                    for &fi in &active {
                        for (k, &uv) in self.mesh.faces[fi].uvs.iter().enumerate() {
                            if uv.distance(hit) < 1e-5 {
                                corners.push((fi, k, uv));
                            }
                        }
                    }
                } else {
                    let p = to_uv(start);
                    if active.iter().any(|&fi| point_in_polygon(p, &self.mesh.faces[fi].uvs)) {
                        // Grabbed inside a face: move every selected face as one island.
                        for &fi in &active {
                            for (k, &uv) in self.mesh.faces[fi].uvs.iter().enumerate() {
                                corners.push((fi, k, uv));
                            }
                        }
                        anchor = self.mesh.uv_bounds(&active).map(|(lo, _)| lo).unwrap_or(Vec2::ZERO);
                    }
                }
                if !corners.is_empty() {
                    self.push_undo();
                    self.uv.drag = Some(Drag { corners, anchor, start });
                }
            }
        }
        if let Some(drag) = &self.uv.drag {
            if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
                let mut delta = to_uv(pos) - to_uv(drag.start);
                if self.uv.snap {
                    // Snap so the anchor lands exactly on a pixel corner.
                    let target = ((drag.anchor + delta) * tex).round() / tex;
                    delta = target - drag.anchor;
                }
                for &(fi, k, orig) in &drag.corners {
                    self.mesh.faces[fi].uvs[k] = orig + delta;
                }
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                self.uv.drag = None;
            }
        }

        // ---- cursor readout
        if let Some(p) = resp.hover_pos() {
            let uv = to_uv(p);
            let text = format!("px {:.0}, {:.0}", (uv.x * tex.x).floor(), ((1.0 - uv.y) * tex.y).floor());
            painter.text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::proportional(12.0), Color32::from_gray(190));
        }
        if active.is_empty() {
            painter.text(rect.center_top() + egui::vec2(0.0, 10.0), egui::Align2::CENTER_TOP, "Select faces in the 3D view to edit their UVs", egui::FontId::proportional(12.0), Color32::from_gray(170));
        }
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
