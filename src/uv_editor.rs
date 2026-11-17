//! 2D UV editor.
//!
//! Shows the texture with the UV layout of the faces chosen in the 3D view
//! and has its own selection (UV vertices or whole faces) with click, box
//! and circle tools. The selection can be moved, and scaled either with the
//! handles on its bounding box or with S. Everything can snap to texture pixels.

use crate::app::{App, SelectTool};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, Sense, Stroke};
use glam::Vec2;
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UvMode {
    Vertex,
    Face,
}

/// One UV coordinate: (face index, corner index) in the active mesh.
type Corner = (usize, usize);

pub struct UvEditor {
    /// Screen pixels per UV unit; 0 = fit on next frame.
    zoom: f32,
    /// Screen offset of the UV-square centre from the panel centre.
    pan: egui::Vec2,
    pub snap: bool,
    pub show_all: bool,
    pub mode: UvMode,
    pub sel: BTreeSet<Corner>,
    drag: Option<Drag>,
    region: Option<Region>,
}

enum DragKind {
    Move,
    /// Dragging a bounding-box handle: `anchor` stays put, `handle` follows the mouse.
    Handle { anchor: Vec2, handle: Vec2, axes: [bool; 2] },
    /// S key: scale about `pivot`; `half` is half the selection's size.
    Scale { pivot: Vec2, half: Vec2 },
    /// Dragging a scale arrow: stretch along one axis (0 = U, 1 = V) about the selection's centre.
    Axis { pivot: Vec2, half: Vec2, axis: usize },
}

struct Drag {
    kind: DragKind,
    /// (face, corner, original uv)
    corners: Vec<(usize, usize, Vec2)>,
    /// Point that should land on a pixel corner when moving with snap on.
    anchor: Vec2,
    start: Option<Pos2>,
    /// Keyboard-started: follows the mouse until a click confirms. Otherwise ends on mouse release.
    modal: bool,
    /// Axis lock from X / Y (0 = U, 1 = V).
    lock: Option<usize>,
    readout: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Replace,
    Add,
    Subtract,
}

struct Region {
    start: Pos2,
    op: Op,
    base: BTreeSet<Corner>,
}

impl Default for UvEditor {
    fn default() -> Self {
        UvEditor {
            zoom: 0.0,
            pan: egui::Vec2::ZERO,
            snap: true,
            show_all: true,
            mode: UvMode::Vertex,
            sel: BTreeSet::new(),
            drag: None,
            region: None,
        }
    }
}

/// Mapping between UV space and the panel. The unit square is drawn with the
/// texture's proportions, so texels are always square on screen.
#[derive(Clone, Copy)]
pub struct View {
    origin: Pos2,
    /// Screen pixels per UV unit along U and V.
    scale: Vec2,
}

impl View {
    pub fn to_screen(&self, uv: Vec2) -> Pos2 {
        Pos2::new(self.origin.x + (uv.x - 0.5) * self.scale.x, self.origin.y - (uv.y - 0.5) * self.scale.y)
    }

    pub fn to_uv(&self, p: Pos2) -> Vec2 {
        Vec2::new((p.x - self.origin.x) / self.scale.x + 0.5, 0.5 - (p.y - self.origin.y) / self.scale.y)
    }

    /// Screen rectangle covered by the whole texture.
    pub fn sheet(&self) -> Rect {
        Rect::from_two_pos(self.to_screen(Vec2::new(0.0, 1.0)), self.to_screen(Vec2::new(1.0, 0.0)))
    }
}

impl UvEditor {
    /// The current view (shared by the UV and paint tabs), fitting the texture first if asked to.
    pub fn view(&mut self, rect: Rect, tex: Vec2) -> View {
        if self.zoom <= 0.0 {
            self.zoom = rect.width().min(rect.height()) * 0.85;
            self.pan = egui::Vec2::ZERO;
        }
        let longest = tex.max_element().max(1.0);
        View { origin: rect.center() + self.pan, scale: Vec2::new(self.zoom * tex.x / longest, self.zoom * tex.y / longest) }
    }

    pub fn fit_view(&mut self) {
        self.zoom = 0.0;
    }

    /// Screen pixels per texel.
    pub fn texel_size(&self, tex: Vec2) -> f32 {
        self.zoom / tex.max_element().max(1.0)
    }

    /// Scroll to zoom about the cursor; `panning` says whether this frame's drag pans the view.
    pub fn navigate(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect, panning: bool) {
        if self.zoom > 0.0 && resp.hovered() {
            let (scroll, pinch, hover) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos()));
            let factor = (scroll * 0.004).exp() * pinch;
            if factor != 1.0 {
                if let Some(hover) = hover {
                    let centre = rect.center() + self.pan;
                    let new_zoom = (self.zoom * factor).clamp(40.0, 60000.0);
                    let k = new_zoom / self.zoom;
                    self.pan = (hover + (centre - hover) * k) - rect.center();
                    self.zoom = new_zoom;
                }
            }
        }
        if panning {
            self.pan += resp.drag_delta();
        }
    }

    /// True while a keyboard-started move / scale is following the mouse.
    pub fn is_modal(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.modal)
    }
}

fn combine(base: &BTreeSet<Corner>, hits: &BTreeSet<Corner>, op: Op) -> BTreeSet<Corner> {
    match op {
        Op::Replace => hits.clone(),
        Op::Add => base.union(hits).copied().collect(),
        Op::Subtract => base.difference(hits).copied().collect(),
    }
}

fn bounds(points: impl Iterator<Item = Vec2>) -> Option<(Vec2, Vec2)> {
    points.fold(None, |acc, p| match acc {
        None => Some((p, p)),
        Some((lo, hi)) => Some((lo.min(p), hi.max(p))),
    })
}

impl App {
    fn uv_of(&self, c: Corner) -> Vec2 {
        self.mesh().faces[c.0].uvs[c.1]
    }

    fn uv_sel_bounds(&self) -> Option<(Vec2, Vec2)> {
        bounds(self.uv.sel.iter().map(|&c| self.uv_of(c)))
    }

    /// Begin moving / scaling the current UV selection.
    fn uv_start_drag(&mut self, kind: DragKind, anchor: Vec2, start: Option<Pos2>, modal: bool) {
        if self.uv.sel.is_empty() {
            return;
        }
        let corners = self.uv.sel.iter().map(|&(f, k)| (f, k, self.uv_of((f, k)))).collect();
        self.push_undo();
        self.uv.drag = Some(Drag { kind, corners, anchor, start, modal, lock: None, readout: String::new() });
    }

    /// Round the selected UVs (or all shown ones when nothing is selected) to texture pixel corners.
    fn uv_snap_to_pixels(&mut self, active: &BTreeSet<usize>) {
        let tex = self.tex_size();
        let targets: Vec<Corner> = if self.uv.sel.is_empty() {
            active.iter().flat_map(|&f| (0..self.mesh().faces[f].uvs.len()).map(move |k| (f, k))).collect()
        } else {
            self.uv.sel.iter().copied().collect()
        };
        if targets.is_empty() {
            return;
        }
        self.push_undo();
        let mesh = self.mesh_mut();
        for (f, k) in targets {
            mesh.faces[f].uvs[k] = (mesh.faces[f].uvs[k] * tex).round() / tex;
        }
    }

    pub fn uv_editor(&mut self, ui: &mut egui::Ui) {
        let active = self.active_faces();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.uv.mode, UvMode::Vertex, "Vertex (V)");
            ui.selectable_value(&mut self.uv.mode, UvMode::Face, "Face (F)");
            ui.separator();
            ui.selectable_value(&mut self.select_tool, SelectTool::Click, "Click");
            ui.selectable_value(&mut self.select_tool, SelectTool::Box, "Box");
            ui.selectable_value(&mut self.select_tool, SelectTool::Circle, "Circle");
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.uv.snap, "Snap to pixels");
            if ui.small_button("Snap selection now").on_hover_text("Round the selected UVs to the nearest pixel corners").clicked() {
                self.uv_snap_to_pixels(&active);
            }
            if ui.small_button("Fit view").clicked() {
                self.uv.fit_view();
            }
        });
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(34, 36, 40));

        let tex = self.tex_size();
        let (mods, mouse, press, primary_down) =
            ui.input(|i| (i.modifiers, i.pointer.latest_pos(), i.pointer.press_origin(), i.pointer.primary_down()));

        // ---- view navigation
        let panning = resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary);
        self.uv.navigate(ui, &resp, rect, panning);
        let view = self.uv.view(rect, tex);
        let zoom = self.uv.zoom;
        let to_screen = |uv: Vec2| view.to_screen(uv);
        let to_uv = |p: Pos2| view.to_uv(p);

        // The UV selection only ever refers to faces currently shown.
        let face_count = self.mesh().faces.len();
        self.uv.sel.retain(|&(f, _)| f < face_count && active.contains(&f));
        let valid: Vec<Corner> = self.uv.sel.iter().copied().filter(|&(f, k)| k < self.mesh().faces[f].uvs.len()).collect();
        self.uv.sel = valid.into_iter().collect();

        // ---- queries used by the tools (working on a copy of the shown UVs, so they do not hold on to `self`)
        let mode = self.uv.mode;
        let face_uvs: std::collections::BTreeMap<usize, Vec<Vec2>> = active.iter().map(|&f| (f, self.mesh().faces[f].uvs.clone())).collect();
        let all_corners: Vec<(Corner, Vec2)> = face_uvs.iter()
            .flat_map(|(&f, uvs)| uvs.iter().enumerate().map(move |(k, &uv)| ((f, k), uv)))
            .collect();
        let point_at = |pos: Pos2| -> Option<Vec2> {
            all_corners.iter()
                .map(|&(_, uv)| (uv, to_screen(uv).distance(pos)))
                .filter(|(_, d)| *d <= 9.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(uv, _)| uv)
        };
        // Corners sharing a UV position are treated as one vertex, so stitched faces stay stitched.
        let same_uv = |uv: Vec2| -> BTreeSet<Corner> { all_corners.iter().filter(|(_, u)| u.distance(uv) < 1e-5).map(|(c, _)| *c).collect() };
        let face_corners = |f: usize| -> BTreeSet<Corner> { (0..face_uvs[&f].len()).map(|k| (f, k)).collect() };
        let face_at = |pos: Pos2| -> Option<usize> {
            let p = to_uv(pos);
            face_uvs.iter().find(|(_, uvs)| point_in_polygon(p, uvs)).map(|(&f, _)| f)
        };
        // What a click at `pos` refers to, as a set of corners.
        let element_at = |pos: Pos2| -> Option<BTreeSet<Corner>> {
            match mode {
                UvMode::Vertex => point_at(pos).map(same_uv),
                UvMode::Face => face_at(pos).map(face_corners),
            }
        };
        let region_hits = |inside: &dyn Fn(Pos2) -> bool, centre: Pos2| -> BTreeSet<Corner> {
            match mode {
                UvMode::Vertex => all_corners.iter().filter(|(_, uv)| inside(to_screen(*uv))).map(|(c, _)| *c).collect(),
                UvMode::Face => {
                    let mut hits = BTreeSet::new();
                    for (&f, uvs) in &face_uvs {
                        let centroid = uvs.iter().copied().sum::<Vec2>() / uvs.len() as f32;
                        if inside(to_screen(centroid)) || point_in_polygon(to_uv(centre), uvs) {
                            hits.extend(face_corners(f));
                        }
                    }
                    hits
                }
            }
        };

        // Scale handles on the selection's bounding box: (screen position, uv position, fixed uv point, axes it scales).
        // The handles sit on a frame drawn a little outside the selection so they never cover a vertex.
        const FRAME_PAD: f32 = 12.0;
        let mut handles: Vec<(Pos2, Vec2, Vec2, [bool; 2])> = Vec::new();
        let sel_bounds = self.uv_sel_bounds();
        let frame = sel_bounds
            .filter(|_| self.uv.sel.len() > 1)
            .map(|(lo, hi)| Rect::from_two_pos(to_screen(lo), to_screen(hi)).expand(FRAME_PAD));
        if let (Some((lo, hi)), Some(frame), true) = (sel_bounds, frame, self.uv.drag.is_none()) {
            let size = hi - lo;
            for (ix, iy) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 0.5), (1.0, 0.5), (0.0, 1.0), (0.5, 1.0), (1.0, 1.0)] {
                let axes = [ix != 0.5, iy != 0.5];
                if (axes[0] && size.x < 1e-6) || (axes[1] && size.y < 1e-6) {
                    continue;
                }
                let at = lo + size * Vec2::new(ix, iy);
                let anchor = lo + size * Vec2::new(1.0 - ix, 1.0 - iy);
                // Screen Y runs downwards, UV V upwards.
                let on_frame = Pos2::new(frame.left() + frame.width() * ix, frame.bottom() - frame.height() * iy);
                handles.push((on_frame, at, anchor, axes));
            }
        }
        let handle_at = |pos: Pos2| handles.iter().find(|h| h.0.distance(pos) <= 8.0).copied();

        // Scale arrows from the middle of the selection: U to the right, V upwards. Each stretches one axis only.
        const ARROW: f32 = 46.0;
        let mut arrows: Vec<(usize, Pos2, Pos2)> = Vec::new(); // (axis, centre, tip)
        if let (Some((lo, hi)), true, true) = (sel_bounds, self.uv.sel.len() > 1, self.uv.drag.is_none()) {
            let c = to_screen((lo + hi) * 0.5);
            if hi.x - lo.x > 1e-6 {
                arrows.push((0, c, c + egui::vec2(ARROW, 0.0)));
            }
            if hi.y - lo.y > 1e-6 {
                arrows.push((1, c, c + egui::vec2(0.0, -ARROW)));
            }
        }
        let arrow_at = |pos: Pos2| {
            arrows.iter().find(|(_, c, tip)| {
                // The box at the tip, or the shaft away from the middle.
                tip.distance(pos) <= 9.0 || (pos.distance(*c) > 10.0 && dist_to_segment(pos, *c, *tip) <= 5.0)
            }).map(|a| a.0)
        };

        // ---- input
        let keyboard_free = !ui.ctx().egui_wants_keyboard_input();
        let key = |k: Key| keyboard_free && ui.input(|i| i.key_pressed(k) && !i.modifiers.command);
        let op = if mods.command { Op::Subtract } else if mods.shift { Op::Add } else { Op::Replace };
        let circle = |c: Pos2, r: f32| move |p: Pos2| p.distance(c) <= r;
        let radius = self.circle_radius;
        let mut box_rect: Option<Rect> = None;

        // Decisions are collected here and applied after the closures above are done borrowing.
        enum Action {
            None,
            Select(BTreeSet<Corner>),
            StartDrag { select: Option<BTreeSet<Corner>>, kind: DragKind, anchor: Vec2, start: Option<Pos2>, modal: bool },
            StartRegion(Region, Option<BTreeSet<Corner>>),
        }
        let mut action = Action::None;

        if self.uv.drag.is_none() {
            if resp.hovered() {
                if key(Key::V) {
                    self.uv.mode = UvMode::Vertex;
                } else if key(Key::F) {
                    self.uv.mode = UvMode::Face;
                } else if key(Key::Q) {
                    self.select_tool = SelectTool::Click;
                } else if key(Key::B) {
                    self.select_tool = SelectTool::Box;
                } else if key(Key::C) {
                    self.select_tool = SelectTool::Circle;
                } else if key(Key::OpenBracket) {
                    self.circle_radius = (self.circle_radius - 5.0).max(5.0);
                } else if key(Key::CloseBracket) {
                    self.circle_radius = (self.circle_radius + 5.0).min(300.0);
                } else if key(Key::A) {
                    action = Action::Select(if mods.alt { BTreeSet::new() } else { all_corners.iter().map(|(c, _)| *c).collect() });
                } else if let (true, Some((lo, hi))) = (key(Key::G), sel_bounds) {
                    let _ = hi;
                    action = Action::StartDrag { select: None, kind: DragKind::Move, anchor: lo, start: None, modal: true };
                } else if let (true, Some((lo, hi))) = (key(Key::S), sel_bounds) {
                    let kind = DragKind::Scale { pivot: (lo + hi) * 0.5, half: (hi - lo) * 0.5 };
                    action = Action::StartDrag { select: None, kind, anchor: lo, start: None, modal: true };
                }
            }

            let tool = self.select_tool;
            if let (true, Some(start)) = (resp.drag_started_by(PointerButton::Primary), press) {
                let element = element_at(start);
                let selected = element.as_ref().is_some_and(|e| e.is_subset(&self.uv.sel));
                if let (Some(axis), Some((lo, hi))) = (arrow_at(start), sel_bounds) {
                    let kind = DragKind::Axis { pivot: (lo + hi) * 0.5, half: (hi - lo) * 0.5, axis };
                    action = Action::StartDrag { select: None, kind, anchor: lo, start: Some(start), modal: false };
                } else if let Some((_, at, anchor, axes)) = handle_at(start) {
                    action = Action::StartDrag { select: None, kind: DragKind::Handle { anchor, handle: at, axes }, anchor: at, start: Some(start), modal: false };
                } else if let (Some((lo, _)), true) = (sel_bounds, !selected && op == Op::Replace && frame.is_some_and(|f| f.contains(start))) {
                    // Dragging anywhere inside the selection frame moves the whole group.
                    action = Action::StartDrag { select: None, kind: DragKind::Move, anchor: lo, start: Some(start), modal: false };
                } else if let (Some(e), true) = (&element, selected || tool == SelectTool::Click) {
                    // Drag a selected element to move the selection; in click mode an unselected one is picked up first.
                    let select = (!selected).then(|| if op == Op::Add { combine(&self.uv.sel, e, Op::Add) } else { e.clone() });
                    let moving = select.as_ref().unwrap_or(&self.uv.sel);
                    let anchor = match self.uv.mode {
                        UvMode::Vertex => self.uv_of(*e.iter().next().unwrap()),
                        UvMode::Face => bounds(moving.iter().map(|&c| self.uv_of(c))).map(|b| b.0).unwrap_or_default(),
                    };
                    action = Action::StartDrag { select, kind: DragKind::Move, anchor, start: Some(start), modal: false };
                } else if tool != SelectTool::Click {
                    let mut base = self.uv.sel.clone();
                    let mut first = None;
                    if tool == SelectTool::Circle {
                        if op == Op::Replace {
                            base.clear();
                        }
                        let hits = region_hits(&circle(start, radius), start);
                        base = combine(&base, &hits, if op == Op::Subtract { Op::Subtract } else { Op::Add });
                        first = Some(base.clone());
                    }
                    action = Action::StartRegion(Region { start, op, base }, first);
                }
            } else if resp.clicked_by(PointerButton::Primary) {
                if let Some(pos) = resp.interact_pointer_pos() {
                    if handle_at(pos).is_none() && arrow_at(pos).is_none() {
                        let hits = if tool == SelectTool::Circle { region_hits(&circle(pos, radius), pos) } else { element_at(pos).unwrap_or_default() };
                        action = Action::Select(match op {
                            Op::Replace => hits,
                            // Shift+click toggles.
                            Op::Add if !hits.is_empty() && hits.is_subset(&self.uv.sel) => combine(&self.uv.sel, &hits, Op::Subtract),
                            other => combine(&self.uv.sel, &hits, other),
                        });
                    }
                }
            }

            // Continue a box / circle selection.
            if let (Some(region), Some(mouse)) = (&self.uv.region, mouse) {
                match tool {
                    SelectTool::Box => {
                        let r = Rect::from_two_pos(region.start, mouse);
                        box_rect = Some(r);
                        let hits = region_hits(&|p| r.contains(p), r.center());
                        action = Action::Select(combine(&region.base, &hits, region.op));
                    }
                    SelectTool::Circle => {
                        let hits = region_hits(&circle(mouse, radius), mouse);
                        action = Action::Select(combine(&self.uv.sel, &hits, if region.op == Op::Subtract { Op::Subtract } else { Op::Add }));
                    }
                    SelectTool::Click => {}
                }
            }
        }
        if !primary_down {
            self.uv.region = None;
        }

        match action {
            Action::None => {}
            Action::Select(sel) => self.uv.sel = sel,
            Action::StartDrag { select, kind, anchor, start, modal } => {
                if let Some(sel) = select {
                    self.uv.sel = sel;
                }
                self.uv_start_drag(kind, anchor, start, modal);
            }
            Action::StartRegion(region, first) => {
                if let Some(sel) = first {
                    self.uv.sel = sel;
                }
                self.uv.region = Some(region);
            }
        }

        // ---- move / scale in progress
        if let Some(mut drag) = self.uv.drag.take() {
            let mut finished = false;
            if drag.modal {
                for (k, axis) in [(Key::X, 0), (Key::Y, 1)] {
                    if key(k) {
                        drag.lock = if drag.lock == Some(axis) { None } else { Some(axis) };
                    }
                }
            }
            if let Some(mouse) = mouse {
                let start = *drag.start.get_or_insert(mouse);
                let delta = to_uv(mouse) - to_uv(start);
                let snap_px = |p: Vec2| (p * tex).round() / tex;
                let keep = |axis: usize| drag.lock.is_none_or(|l| l == axis);
                // Every transform is "anchor + (orig - anchor) * scale + shift".
                let (pivot, scale, shift) = match drag.kind {
                    DragKind::Move => {
                        let mut d = delta;
                        if self.uv.snap {
                            // Snap so the anchor lands exactly on a pixel corner.
                            d = snap_px(drag.anchor + d) - drag.anchor;
                        }
                        let d = Vec2::new(if keep(0) { d.x } else { 0.0 }, if keep(1) { d.y } else { 0.0 });
                        drag.readout = format!("Move {:+.1}, {:+.1} px", d.x * tex.x, -d.y * tex.y);
                        (Vec2::ZERO, Vec2::ONE, d)
                    }
                    DragKind::Handle { anchor, handle, axes } => {
                        let mut h = handle + delta;
                        if self.uv.snap {
                            h = snap_px(h);
                        }
                        let mut s = Vec2::ONE;
                        for i in 0..2 {
                            if axes[i] && (handle[i] - anchor[i]).abs() > 1e-6 {
                                s[i] = (h[i] - anchor[i]) / (handle[i] - anchor[i]);
                            }
                        }
                        // Shift on a corner handle keeps the proportions.
                        if mods.shift && axes[0] && axes[1] {
                            let u = if s.x.abs() > s.y.abs() { s.x } else { s.y };
                            s = Vec2::splat(u);
                        }
                        let size = ((handle - anchor) * s * tex).abs();
                        drag.readout = format!("Size {:.1} x {:.1} px", size.x, size.y);
                        (anchor, s, Vec2::ZERO)
                    }
                    DragKind::Scale { pivot, half } => {
                        let centre = to_screen(pivot);
                        let f = centre.distance(mouse) / centre.distance(start).max(1.0);
                        let mut s = Vec2::new(if keep(0) { f } else { 1.0 }, if keep(1) { f } else { 1.0 });
                        if self.uv.snap {
                            // Snap so the selection's edges land on pixel boundaries.
                            for i in 0..2 {
                                if keep(i) && half[i] > 1e-6 {
                                    let edge = ((pivot[i] + half[i] * s[i]) * tex[i]).round() / tex[i];
                                    s[i] = (edge - pivot[i]) / half[i];
                                }
                            }
                        }
                        let size = (half * 2.0 * s * tex).abs();
                        drag.readout = format!("Size {:.1} x {:.1} px", size.x, size.y);
                        (pivot, s, Vec2::ZERO)
                    }
                    DragKind::Axis { pivot, half, axis } => {
                        // How far along the arrow the cursor is, relative to where it was grabbed.
                        let centre = to_screen(pivot);
                        let along = |p: Pos2| if axis == 0 { p.x - centre.x } else { centre.y - p.y };
                        let mut f = if along(start).abs() > 1.0 { (along(mouse) / along(start)).max(0.02) } else { 1.0 };
                        if self.uv.snap && half[axis] > 1e-6 {
                            // Snap so the selection's edge lands on a pixel boundary.
                            let edge = ((pivot[axis] + half[axis] * f) * tex[axis]).round() / tex[axis];
                            f = ((edge - pivot[axis]) / half[axis]).max(0.0);
                        }
                        let mut s = Vec2::ONE;
                        s[axis] = f;
                        let size = (half * 2.0 * s * tex).abs();
                        drag.readout = format!("{} size {:.1} px", if axis == 0 { "U" } else { "V" }, size[axis]);
                        (pivot, s, Vec2::ZERO)
                    }
                };
                let mesh = self.mesh_mut();
                for &(f, k, orig) in &drag.corners {
                    if f < mesh.faces.len() && k < mesh.faces[f].uvs.len() {
                        mesh.faces[f].uvs[k] = pivot + (orig - pivot) * scale + shift;
                    }
                }
            }
            let cancel = drag.modal && (key(Key::Escape) || resp.clicked_by(PointerButton::Secondary));
            if cancel {
                // Undo restores the mesh; keep the UV selection.
                let sel = self.uv.sel.clone();
                self.revert_last();
                self.uv.sel = sel;
                finished = true;
            } else if drag.modal {
                finished = key(Key::Enter) || resp.clicked_by(PointerButton::Primary);
            } else if !primary_down {
                finished = true;
            }
            if !finished {
                self.uv.drag = Some(drag);
                ui.ctx().request_repaint();
            }
        }

        // ---- texture and pixel grid
        let sheet = view.sheet();
        match &self.texture {
            Some(t) => {
                painter.rect_filled(sheet, 0.0, Color32::from_gray(70));
                painter.image(t.handle.id(), sheet, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                painter.rect_filled(sheet, 0.0, Color32::from_gray(58));
            }
        }
        if zoom / tex.x.max(tex.y) >= 6.0 {
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

        // ---- faces, points, selection (drawn from the mesh as it is after this frame's edits)
        let mesh = self.mesh();
        let orange = Color32::from_rgb(255, 150, 40);
        let bright = Color32::from_rgb(255, 245, 170);
        if self.uv.show_all {
            let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 170, 255, 70));
            for (fi, face) in mesh.faces.iter().enumerate() {
                if !active.contains(&fi) {
                    painter.add(egui::Shape::closed_line(face.uvs.iter().map(|&uv| to_screen(uv)).collect(), stroke));
                }
            }
        }
        for &fi in &active {
            let pts: Vec<Pos2> = mesh.faces[fi].uvs.iter().map(|&uv| to_screen(uv)).collect();
            let whole = (0..pts.len()).all(|k| self.uv.sel.contains(&(fi, k)));
            if whole && self.uv.mode == UvMode::Face {
                painter.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(255, 170, 60, 28), Stroke::NONE));
            }
            painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5, orange)));
        }
        for &fi in &active {
            for (k, &uv) in mesh.faces[fi].uvs.iter().enumerate() {
                if self.uv.sel.contains(&(fi, k)) {
                    painter.circle_filled(to_screen(uv), 4.5, bright);
                    painter.circle_stroke(to_screen(uv), 4.5, Stroke::new(1.0, Color32::BLACK));
                } else if self.uv.mode == UvMode::Vertex {
                    painter.circle_filled(to_screen(uv), 3.0, orange);
                }
            }
        }
        // Selection frame with scale handles; drag inside it to move the group.
        let frame_now = self.uv_sel_bounds()
            .filter(|_| self.uv.sel.len() > 1)
            .map(|(lo, hi)| Rect::from_two_pos(to_screen(lo), to_screen(hi)).expand(FRAME_PAD));
        if let Some(r) = frame_now {
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 245, 170, 150)), egui::StrokeKind::Middle);
            let hover = resp.hover_pos();
            let mut over_handle = false;
            for (pos, ..) in &handles {
                let hot = hover.is_some_and(|h| h.distance(*pos) <= 8.0);
                over_handle |= hot;
                let square = Rect::from_center_size(*pos, egui::Vec2::splat(if hot { 10.0 } else { 7.0 }));
                painter.rect_filled(square, 1.0, if hot { Color32::WHITE } else { bright });
                painter.rect_stroke(square, 1.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Middle);
            }
            // One-axis scale arrows.
            for &(axis, c, tip) in &arrows {
                let hot = hover.is_some_and(|h| arrow_at(h) == Some(axis));
                over_handle |= hot;
                let colour = if hot {
                    Color32::from_rgb(255, 235, 120)
                } else if axis == 0 {
                    Color32::from_rgb(235, 80, 80)
                } else {
                    Color32::from_rgb(110, 205, 70)
                };
                painter.line_segment([c, tip], Stroke::new(4.5, Color32::from_black_alpha(150)));
                painter.line_segment([c, tip], Stroke::new(2.5, colour));
                let end = Rect::from_center_size(tip, egui::Vec2::splat(11.0));
                painter.rect_filled(end, 1.0, colour);
                painter.rect_stroke(end, 1.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Middle);
                painter.text(tip + (tip - c).normalized() * 12.0, egui::Align2::CENTER_CENTER, if axis == 0 { "U" } else { "V" }, egui::FontId::proportional(11.0), colour);
            }
            if let Some(&(_, c, _)) = arrows.first() {
                painter.circle_filled(c, 3.0, Color32::WHITE);
                painter.circle_stroke(c, 3.0, Stroke::new(1.0, Color32::BLACK));
            }
            // Show what a drag here would do.
            if self.uv.drag.is_some() || over_handle {
                ui.ctx().set_cursor_icon(if over_handle { egui::CursorIcon::Crosshair } else { egui::CursorIcon::Grabbing });
            } else if hover.is_some_and(|h| r.contains(h)) && !mods.shift && !mods.command {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
        }

        // ---- tool overlays and readouts
        let tool_color = Color32::from_rgb(255, 190, 90);
        if let Some(r) = box_rect {
            painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(255, 170, 60, 25));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, tool_color), egui::StrokeKind::Middle);
        }
        if self.select_tool == SelectTool::Circle && self.uv.drag.is_none() {
            if let Some(p) = resp.hover_pos() {
                painter.circle_stroke(p, self.circle_radius, Stroke::new(1.0, tool_color));
            }
        }
        let font = egui::FontId::proportional(12.0);
        let text = match &self.uv.drag {
            Some(d) if d.modal => {
                let lock = match d.lock {
                    Some(0) => "  (U only)",
                    Some(1) => "  (V only)",
                    _ => "",
                };
                format!("{}{lock}    [X/Y] lock axis   [Click/Enter] confirm   [Esc/Right-click] cancel", d.readout)
            }
            Some(d) => d.readout.clone(),
            None => match resp.hover_pos() {
                Some(p) => {
                    let uv = to_uv(p);
                    format!("px {:.0}, {:.0}    drag in the frame to move · arrows scale one axis · handles or S scale", (uv.x * tex.x).floor(), ((1.0 - uv.y) * tex.y).floor())
                }
                None => String::new(),
            },
        };
        painter.text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, text, font.clone(), Color32::from_gray(200));
        if active.is_empty() {
            painter.text(rect.center_top() + egui::vec2(0.0, 10.0), egui::Align2::CENTER_TOP, "Nothing to show: pick an object, or faces in face mode", font, Color32::from_gray(170));
        }
    }
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
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
