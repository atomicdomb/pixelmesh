//! 3D viewport: navigation, picking, modal move/rotate/scale and frame building.

use crate::app::{App, SelMode};
use crate::render::{Batch, FrameData};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Sense};
use eframe::egui_glow;
use glam::{Quat, Vec3};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Grab,
    Rotate,
    Scale,
}

/// A Blender-style modal transform: move the mouse, click to confirm, Esc / right-click to cancel.
pub struct Modal {
    kind: ModalKind,
    /// Constraint direction (world axis, or the normal when extruding).
    axis: Option<Vec3>,
    axis_name: &'static str,
    verts: Vec<u32>,
    orig: Vec<Vec3>,
    pivot: Vec3,
    start_mouse: Option<Pos2>,
    readout: String,
}

const ORANGE: [f32; 4] = [1.0, 0.55, 0.1, 1.0];

impl App {
    pub fn start_modal(&mut self, kind: ModalKind, axis: Option<Vec3>) {
        if self.active_verts().is_empty() {
            self.status = "Nothing selected.".into();
            return;
        }
        self.push_undo();
        self.start_modal_no_undo(kind, axis);
    }

    pub fn start_modal_no_undo(&mut self, kind: ModalKind, axis: Option<Vec3>) {
        let verts: Vec<u32> = self.active_verts().into_iter().collect();
        if verts.is_empty() {
            return;
        }
        let orig: Vec<Vec3> = verts.iter().map(|&v| self.mesh.positions[v as usize]).collect();
        let pivot = orig.iter().copied().sum::<Vec3>() / orig.len() as f32;
        self.modal = Some(Modal {
            kind,
            axis,
            axis_name: if axis.is_some() { "normal" } else { "" },
            verts,
            orig,
            pivot,
            start_mouse: None,
            readout: String::new(),
        });
    }

    fn update_modal(&mut self, rect: egui::Rect, mouse: Pos2, snap: bool) {
        let Some(m) = &mut self.modal else { return };
        let start = *m.start_mouse.get_or_insert(mouse);
        let cam = &self.camera;
        let toward_viewer = cam.back();

        match m.kind {
            ModalKind::Grab => {
                let delta = match m.axis {
                    Some(a) => {
                        let t = line_param(m.pivot, a, cam.ray(rect, mouse)) - line_param(m.pivot, a, cam.ray(rect, start));
                        let t = if snap { (t / self.grid_snap).round() * self.grid_snap } else { t };
                        m.readout = format!("{t:.3}");
                        a * t
                    }
                    None => {
                        let hit = |p: Pos2| plane_hit(m.pivot, toward_viewer, cam.ray(rect, p));
                        let mut d = hit(mouse) - hit(start);
                        if snap {
                            d = (d / self.grid_snap).round() * self.grid_snap;
                        }
                        m.readout = format!("{:.3}, {:.3}, {:.3}", d.x, d.y, d.z);
                        d
                    }
                };
                for (i, &v) in m.verts.iter().enumerate() {
                    self.mesh.positions[v as usize] = m.orig[i] + delta;
                }
            }
            ModalKind::Rotate => {
                let Some(centre) = cam.project(rect, m.pivot) else { return };
                let angle_of = |p: Pos2| (p.y - centre.y).atan2(p.x - centre.x);
                // Screen Y points down, so a counter-clockwise turn on screen is a negative atan2 delta.
                let mut angle = -(angle_of(mouse) - angle_of(start));
                let axis = m.axis.unwrap_or(toward_viewer);
                if axis.dot(toward_viewer) < 0.0 {
                    angle = -angle;
                }
                if snap {
                    let step = 15f32.to_radians();
                    angle = (angle / step).round() * step;
                }
                m.readout = format!("{:.1}°", angle.to_degrees());
                let q = Quat::from_axis_angle(axis.normalize(), angle);
                for (i, &v) in m.verts.iter().enumerate() {
                    self.mesh.positions[v as usize] = m.pivot + q * (m.orig[i] - m.pivot);
                }
            }
            ModalKind::Scale => {
                let Some(centre) = cam.project(rect, m.pivot) else { return };
                let mut s = centre.distance(mouse) / centre.distance(start).max(1.0);
                if snap {
                    s = (s * 10.0).round() / 10.0;
                }
                m.readout = format!("×{s:.3}");
                for (i, &v) in m.verts.iter().enumerate() {
                    let d = m.orig[i] - m.pivot;
                    self.mesh.positions[v as usize] = m.pivot + match m.axis {
                        Some(a) => d + a * d.dot(a) * (s - 1.0),
                        None => d * s,
                    };
                }
            }
        }
    }

    fn modal_keys(&mut self, ui: &egui::Ui) {
        let Some(m) = &mut self.modal else { return };
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
        for (key, axis, name) in [(Key::X, Vec3::X, "X"), (Key::Y, Vec3::Y, "Y"), (Key::Z, Vec3::Z, "Z")] {
            if pressed(key) {
                // Pressing the same axis again frees the constraint.
                if m.axis == Some(axis) {
                    m.axis = None;
                    m.axis_name = "";
                } else {
                    m.axis = Some(axis);
                    m.axis_name = name;
                }
            }
        }
        if pressed(Key::Escape) {
            self.cancel_modal();
        } else if pressed(Key::Enter) {
            self.modal = None;
        }
    }

    fn cancel_modal(&mut self) {
        if self.modal.take().is_some() {
            self.revert_last();
        }
    }

    fn viewport_keys(&mut self, ui: &egui::Ui) {
        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k) && !i.modifiers.command);
        if pressed(Key::G) {
            self.start_modal(ModalKind::Grab, None);
        } else if pressed(Key::R) {
            self.start_modal(ModalKind::Rotate, None);
        } else if pressed(Key::S) {
            self.start_modal(ModalKind::Scale, None);
        } else if pressed(Key::E) {
            self.extrude();
        } else if pressed(Key::D) && shift {
            if self.duplicate_selection() {
                self.start_modal_no_undo(ModalKind::Grab, None);
            }
        } else if pressed(Key::X) || pressed(Key::Delete) {
            self.delete_selection();
        } else if pressed(Key::A) {
            if alt { self.clear_selection() } else { self.select_all() }
        } else if pressed(Key::L) {
            self.select_linked();
        } else if pressed(Key::V) {
            self.set_mode(SelMode::Vertex);
        } else if pressed(Key::F) {
            self.set_mode(SelMode::Face);
        } else if pressed(Key::Num1) {
            (self.camera.yaw, self.camera.pitch) = (0.0, 0.0);
        } else if pressed(Key::Num3) {
            (self.camera.yaw, self.camera.pitch) = (std::f32::consts::FRAC_PI_2, 0.0);
        } else if pressed(Key::Num7) {
            (self.camera.yaw, self.camera.pitch) = (0.0, 1.5707);
        } else if pressed(Key::Num5) {
            self.camera.ortho = !self.camera.ortho;
        } else if pressed(Key::Home) {
            self.frame_all();
        }
    }

    fn pick(&mut self, rect: egui::Rect, pos: Pos2, additive: bool) {
        let (origin, dir) = self.camera.ray(rect, pos);
        match self.sel_mode {
            SelMode::Face => {
                let hit = self.mesh.raycast(origin, dir).map(|(f, _)| f);
                if !additive {
                    self.sel_faces.clear();
                }
                if let Some(f) = hit {
                    if !self.sel_faces.remove(&f) || !additive {
                        self.sel_faces.insert(f);
                    }
                }
            }
            SelMode::Vertex => {
                let mut best: Option<(u32, f32)> = None;
                for (i, &p) in self.mesh.positions.iter().enumerate() {
                    let Some(sp) = self.camera.project(rect, p) else { continue };
                    let d = sp.distance(pos);
                    if d > 12.0 || best.is_some_and(|(_, bd)| bd <= d) {
                        continue;
                    }
                    // Skip vertices hidden behind a face.
                    let (o, dir) = self.camera.ray(rect, sp);
                    let dist = (p - o).dot(dir);
                    if self.mesh.raycast(o, dir).is_some_and(|(_, t)| t < dist - 1e-3 * dist.max(1.0)) {
                        continue;
                    }
                    best = Some((i as u32, d));
                }
                if !additive {
                    self.sel_verts.clear();
                }
                if let Some((v, _)) = best {
                    if !self.sel_verts.remove(&v) || !additive {
                        self.sel_verts.insert(v);
                    }
                }
            }
        }
    }

    fn build_frame(&self, rect: egui::Rect) -> FrameData {
        let mut tris = Batch::default();
        let mut grid = Batch::default();
        let mut lines = Batch::default();
        let mut points = Batch::default();

        let n = 8;
        for i in -n..=n {
            let (f, e) = (i as f32, n as f32);
            let c = [1.0, 1.0, 1.0, if i == 0 { 0.0 } else { 0.09 }];
            grid.line(Vec3::new(f, 0.0, -e), Vec3::new(f, 0.0, e), c);
            grid.line(Vec3::new(-e, 0.0, f), Vec3::new(e, 0.0, f), c);
        }
        grid.line(Vec3::new(-8.0, 0.0, 0.0), Vec3::new(8.0, 0.0, 0.0), [0.9, 0.3, 0.3, 0.7]);
        grid.line(Vec3::new(0.0, 0.0, -8.0), Vec3::new(0.0, 0.0, 8.0), [0.3, 0.5, 0.95, 0.7]);

        let face_mode = self.sel_mode == SelMode::Face;
        for (fi, face) in self.mesh.faces.iter().enumerate() {
            let normal = self.mesh.face_normal(fi);
            let tint = if face_mode && self.sel_faces.contains(&fi) { [1.0, 0.55, 0.1, 0.35] } else { [0.0; 4] };
            let corner = |k: usize| (self.mesh.positions[face.verts[k] as usize], face.uvs[k]);
            for k in 1..face.verts.len() - 1 {
                for c in [0, k, k + 1] {
                    let (p, uv) = corner(c);
                    tris.push(p, normal, uv, tint);
                }
            }
        }

        let active = self.active_verts();
        for (a, b) in self.mesh.edges() {
            let selected = active.contains(&a) && active.contains(&b);
            let c = if selected { ORANGE } else { [0.04, 0.04, 0.06, 0.9] };
            lines.line(self.mesh.positions[a as usize], self.mesh.positions[b as usize], c);
        }
        if let Some(m) = &self.modal {
            if let Some(a) = m.axis {
                lines.line(m.pivot - a * 50.0, m.pivot + a * 50.0, [1.0, 1.0, 1.0, 0.5]);
            }
        }
        if !face_mode {
            for (i, &p) in self.mesh.positions.iter().enumerate() {
                let c = if self.sel_verts.contains(&(i as u32)) { ORANGE } else { [0.0, 0.0, 0.0, 1.0] };
                points.point(p, c);
            }
        }

        FrameData {
            view_proj: self.camera.view_proj(rect),
            tris,
            grid,
            lines,
            points,
            texture: self.texture.as_ref().map(|t| (t.version, t.size[0], t.size[1], t.rgba.clone())),
        }
    }

    pub fn viewport(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.viewport_rect = rect;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(52, 55, 62));

        let (mods, scroll, pinch, mouse) = ui.input(|i| {
            (i.modifiers, i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.latest_pos())
        });

        // ---- navigation (always available, also while a modal tool is running)
        if resp.hovered() {
            if scroll != 0.0 {
                self.camera.zoom((scroll * 0.004).exp());
            }
            if pinch != 1.0 {
                self.camera.zoom(pinch);
            }
        }
        let orbiting = resp.dragged_by(PointerButton::Middle)
            || resp.dragged_by(PointerButton::Secondary)
            || (resp.dragged_by(PointerButton::Primary) && mods.alt);
        if orbiting {
            let d = resp.drag_delta();
            if mods.shift {
                self.camera.pan(d.x, d.y, rect.height());
            } else {
                self.camera.orbit(d.x, d.y);
            }
        }

        // ---- tools
        let keyboard_free = !ui.ctx().egui_wants_keyboard_input();
        if self.modal.is_some() {
            if keyboard_free {
                self.modal_keys(ui);
            }
            if let Some(mouse) = mouse {
                self.update_modal(rect, mouse, mods.command);
            }
            if resp.clicked_by(PointerButton::Primary) {
                self.modal = None;
            } else if resp.clicked_by(PointerButton::Secondary) {
                self.cancel_modal();
            }
            ui.ctx().request_repaint();
        } else {
            if resp.hovered() && keyboard_free {
                self.viewport_keys(ui);
            }
            if resp.clicked_by(PointerButton::Primary) && !mods.alt {
                if let Some(pos) = resp.interact_pointer_pos() {
                    self.pick(rect, pos, mods.shift);
                }
            }
        }

        // ---- draw
        if let Some(renderer) = self.renderer.clone() {
            let frame = Arc::new(self.build_frame(rect));
            painter.add(egui::PaintCallback {
                rect,
                callback: Arc::new(egui_glow::CallbackFn::new(move |_info, painter| {
                    renderer.lock().unwrap().paint(painter.gl(), &frame);
                })),
            });
        }

        let font = egui::FontId::proportional(13.0);
        let hint = match &self.modal {
            Some(m) => {
                let what = match m.kind {
                    ModalKind::Grab => "Move",
                    ModalKind::Rotate => "Rotate",
                    ModalKind::Scale => "Scale",
                };
                let axis = if m.axis_name.is_empty() { String::new() } else { format!(" along {}", m.axis_name) };
                format!("{what}{axis}:  {}    [X/Y/Z] axis   [Ctrl] snap   [Click/Enter] confirm   [Esc/Right-click] cancel", m.readout)
            }
            None => "Click select · Shift+click add · Middle/Right-drag orbit · Shift+drag pan · Scroll zoom · G/R/S transform · E extrude".to_string(),
        };
        painter.text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, hint, font.clone(), Color32::from_gray(200));
        let mode = match self.sel_mode {
            SelMode::Vertex => "Vertex mode",
            SelMode::Face => "Face mode",
        };
        let proj = if self.camera.ortho { "Ortho" } else { "Perspective" };
        painter.text(rect.left_top() + egui::vec2(8.0, 8.0), egui::Align2::LEFT_TOP, format!("{mode} · {proj}"), font, Color32::from_gray(200));
    }
}

/// Parameter along the line `p + t*a` closest to a ray.
fn line_param(p: Vec3, a: Vec3, (o, d): (Vec3, Vec3)) -> f32 {
    let w = p - o;
    let (aa, ad, dd, aw, dw) = (a.dot(a), a.dot(d), d.dot(d), a.dot(w), d.dot(w));
    let denom = aa * dd - ad * ad;
    if denom.abs() < 1e-6 {
        return 0.0;
    }
    (ad * dw - dd * aw) / denom
}

/// Intersection of a ray with the plane through `p` with normal `n`.
fn plane_hit(p: Vec3, n: Vec3, (o, d): (Vec3, Vec3)) -> Vec3 {
    let denom = d.dot(n);
    if denom.abs() < 1e-6 {
        return p;
    }
    o + d * ((p - o).dot(n) / denom)
}
