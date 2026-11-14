//! 3D viewport: navigation, picking, the move-arrow gizmo, modal
//! move/rotate/scale and frame building.

use crate::app::{App, SelMode, SelectTool};
use crate::render::{Batch, FrameData};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Sense, Stroke};
use eframe::egui_glow;
use glam::{Quat, Vec3};
use pixelmesh::mesh::{EdgeRing, Mesh};
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Grab,
    Rotate,
    Scale,
}

/// A transform in progress. Started from the keyboard it is Blender-style
/// (move the mouse, click to confirm); started from a gizmo arrow it is a
/// plain drag that ends when the button is released.
pub struct Modal {
    kind: ModalKind,
    /// Constraint direction (world axis, or the normal when extruding).
    axis: Option<Vec3>,
    axis_name: &'static str,
    /// (object, vertex) pairs being moved, parallel to `orig`.
    targets: Vec<(usize, u32)>,
    orig: Vec<Vec3>,
    pivot: Vec3,
    start_mouse: Option<Pos2>,
    readout: String,
    /// True for gizmo drags: confirm on mouse release.
    drag: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Inset,
    Bevel,
}

/// An inset or bevel being sized: every frame the operation is re-applied to
/// `base` (the mesh as it was) with the amount taken from the mouse.
pub struct OpModal {
    kind: OpKind,
    base: Mesh,
    faces: BTreeSet<usize>,
    edges: BTreeSet<(u32, u32)>,
    start_mouse: Option<Pos2>,
    value: f32,
}

impl OpModal {
    pub fn new(kind: OpKind, base: Mesh, faces: BTreeSet<usize>, edges: BTreeSet<(u32, u32)>) -> Self {
        OpModal { kind, base, faces, edges, start_mouse: None, value: 0.0 }
    }
}

/// A loop cut being placed: first hover an edge and click, then (for a single cut) slide it and click again.
pub struct LoopCut {
    cuts: usize,
    /// Set once the ring is chosen: the ring and the edge the slide is measured along.
    slide: Option<(EdgeRing, (u32, u32))>,
    t: f32,
}

impl LoopCut {
    pub fn new() -> Self {
        LoopCut { cuts: 1, slide: None, t: 0.5 }
    }
}

/// Lines and points to draw over the scene (loop cut preview).
#[derive(Default)]
struct Preview {
    lines: Vec<[Vec3; 2]>,
    points: Vec<Vec3>,
}

/// A selection of whatever the current mode selects.
#[derive(Clone, Default)]
pub struct Sel {
    objects: BTreeSet<usize>,
    verts: BTreeSet<u32>,
    faces: BTreeSet<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelOp {
    Replace,
    Add,
    Subtract,
}

/// A box or circle selection drag in progress.
pub struct Region {
    start: Pos2,
    op: SelOp,
    /// Selection when the drag began (the box is re-applied to it every frame).
    base: Sel,
}

fn combine(base: &Sel, hits: &Sel, op: SelOp) -> Sel {
    fn set<T: Ord + Copy>(a: &BTreeSet<T>, b: &BTreeSet<T>, op: SelOp) -> BTreeSet<T> {
        match op {
            SelOp::Replace => b.clone(),
            SelOp::Add => a.union(b).copied().collect(),
            SelOp::Subtract => a.difference(b).copied().collect(),
        }
    }
    Sel { objects: set(&base.objects, &hits.objects, op), verts: set(&base.verts, &hits.verts, op), faces: set(&base.faces, &hits.faces, op) }
}

/// Screen-space layout of the three move arrows.
struct Gizmo {
    centre: Pos2,
    /// Arrow tips for X, Y, Z; None when the axis points (almost) straight at the camera.
    tips: [Option<Pos2>; 3],
}

const ORANGE: [f32; 4] = [1.0, 0.55, 0.1, 1.0];
const AXES: [(Vec3, &str, Color32); 3] = [
    (Vec3::X, "X", Color32::from_rgb(235, 80, 80)),
    (Vec3::Y, "Y", Color32::from_rgb(110, 205, 70)),
    (Vec3::Z, "Z", Color32::from_rgb(80, 140, 250)),
];
const ARROW_PX: f32 = 85.0;

impl App {
    pub fn start_modal(&mut self, kind: ModalKind, axis: Option<Vec3>) {
        if self.transform_targets().is_empty() {
            self.status = "Nothing selected.".into();
            return;
        }
        self.push_undo();
        self.start_modal_no_undo(kind, axis);
    }

    pub fn start_modal_no_undo(&mut self, kind: ModalKind, axis: Option<Vec3>) {
        let targets = self.transform_targets();
        if targets.is_empty() {
            return;
        }
        let orig: Vec<Vec3> = targets.iter().map(|&(o, v)| self.objects[o].mesh.positions[v as usize]).collect();
        let pivot = self.pivot_of(&orig);
        self.modal = Some(Modal {
            kind,
            axis,
            axis_name: if axis.is_some() { "normal" } else { "" },
            targets,
            orig,
            pivot,
            start_mouse: None,
            readout: String::new(),
            drag: false,
        });
    }

    /// Pivot for a set of points: the bounding-box centre for whole objects
    /// (so it sits in the middle of the shape), the average for vertices / faces.
    fn pivot_of(&self, pts: &[Vec3]) -> Vec3 {
        if self.sel_mode == SelMode::Object {
            let lo = pts.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
            let hi = pts.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
            (lo + hi) * 0.5
        } else {
            pts.iter().copied().sum::<Vec3>() / pts.len().max(1) as f32
        }
    }

    /// Centre of whatever the transform tools would act on.
    fn selection_pivot(&self) -> Option<Vec3> {
        let targets = self.transform_targets();
        if targets.is_empty() || !self.objects.get(self.active).is_some_and(|o| o.visible) && self.sel_mode != SelMode::Object {
            return None;
        }
        let pts: Vec<Vec3> = targets.iter().map(|&(o, v)| self.objects[o].mesh.positions[v as usize]).collect();
        Some(self.pivot_of(&pts))
    }

    fn gizmo(&self, rect: egui::Rect) -> Option<Gizmo> {
        let pivot = self.selection_pivot()?;
        let centre = self.camera.project(rect, pivot)?;
        let len = self.camera.units_per_pixel(rect, pivot) * ARROW_PX;
        let tips = AXES.map(|(axis, _, _)| {
            self.camera.project(rect, pivot + axis * len).filter(|tip| tip.distance(centre) > 14.0)
        });
        Some(Gizmo { centre, tips })
    }

    fn update_modal(&mut self, rect: egui::Rect, mouse: Pos2, snap: bool) {
        let Some(mut m) = self.modal.take() else { return };
        if let Some(new) = self.modal_positions(&mut m, rect, mouse, snap) {
            for (i, &(o, v)) in m.targets.iter().enumerate() {
                self.objects[o].mesh.positions[v as usize] = new[i];
            }
        }
        self.modal = Some(m);
    }

    fn modal_positions(&self, m: &mut Modal, rect: egui::Rect, mouse: Pos2, snap: bool) -> Option<Vec<Vec3>> {
        let start = *m.start_mouse.get_or_insert(mouse);
        let cam = &self.camera;
        let toward_viewer = cam.back();
        let step = self.grid_snap;

        Some(match m.kind {
            ModalKind::Grab => {
                let delta = match m.axis {
                    Some(a) => {
                        let t = line_param(m.pivot, a, cam.ray(rect, mouse)) - line_param(m.pivot, a, cam.ray(rect, start));
                        let t = if snap { (t / step).round() * step } else { t };
                        m.readout = format!("{t:.3}");
                        a * t
                    }
                    None => {
                        let hit = |p: Pos2| plane_hit(m.pivot, toward_viewer, cam.ray(rect, p));
                        let mut d = hit(mouse) - hit(start);
                        if snap {
                            d = (d / step).round() * step;
                        }
                        m.readout = format!("{:.3}, {:.3}, {:.3}", d.x, d.y, d.z);
                        d
                    }
                };
                m.orig.iter().map(|&p| p + delta).collect()
            }
            ModalKind::Rotate => {
                let centre = cam.project(rect, m.pivot)?;
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
                m.orig.iter().map(|&p| m.pivot + q * (p - m.pivot)).collect()
            }
            ModalKind::Scale => {
                let centre = cam.project(rect, m.pivot)?;
                let mut s = centre.distance(mouse) / centre.distance(start).max(1.0);
                if snap {
                    s = (s * 10.0).round() / 10.0;
                }
                m.readout = format!("×{s:.3}");
                m.orig.iter().map(|&p| {
                    let d = p - m.pivot;
                    m.pivot + match m.axis {
                        Some(a) => d + a * d.dot(a) * (s - 1.0),
                        None => d * s,
                    }
                }).collect()
            }
        })
    }

    /// Inset / bevel: amount follows the mouse (right = more).
    fn op_step(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, mouse: Option<Pos2>, snap: bool, keys: bool) {
        let Some(mut op) = self.op.take() else { return };
        let key = |k: Key| keys && ui.input(|i| i.key_pressed(k));
        if key(Key::Escape) || resp.clicked_by(PointerButton::Secondary) {
            self.revert_last();
            return;
        }
        if let Some(mouse) = mouse {
            let start = *op.start_mouse.get_or_insert(mouse);
            // Scale mouse travel by the zoom level at the selection so the feel is the same near and far.
            let verts: BTreeSet<u32> = op.base.verts_of_faces(&op.faces).into_iter().chain(op.edges.iter().flat_map(|&(a, b)| [a, b])).collect();
            let pivot = verts.iter().map(|&v| op.base.positions[v as usize]).sum::<Vec3>() / verts.len().max(1) as f32;
            let mut value = ((mouse.x - start.x) * self.camera.units_per_pixel(rect, pivot)).max(0.0);
            if snap {
                value = (value / self.grid_snap).round() * self.grid_snap;
            }
            op.value = value;
            let mut mesh = op.base.clone();
            let result_faces = match op.kind {
                OpKind::Inset => {
                    mesh.inset_faces(&op.faces, value);
                    op.faces.clone()
                }
                OpKind::Bevel => mesh.bevel_edges(&op.edges, value),
            };
            // Keep the selection valid for the rebuilt mesh: the inset faces, or the new bevel faces.
            let fallback = if op.kind == OpKind::Inset || value > 0.0 { result_faces } else { BTreeSet::new() };
            self.sel_verts = mesh.verts_of_faces(&fallback);
            self.sel_faces = fallback;
            *self.mesh_mut() = mesh;
        }
        if key(Key::Enter) || resp.clicked_by(PointerButton::Primary) {
            if op.value <= 0.0 {
                // Nothing was changed: drop the undo step again.
                self.revert_last();
            }
            return;
        }
        self.op = Some(op);
    }

    /// The edge a loop cut should start from: the nearest edge of the face under the cursor,
    /// or failing that the nearest edge on screen.
    fn pick_edge(&self, rect: egui::Rect, pos: Pos2) -> Option<(u32, u32)> {
        let mesh = self.mesh();
        let dist = |&(a, b): &(u32, u32)| -> Option<f32> {
            let pa = self.camera.project(rect, mesh.positions[a as usize])?;
            let pb = self.camera.project(rect, mesh.positions[b as usize])?;
            Some(segment_distance(pos, pa, pb))
        };
        let nearest = |edges: Vec<(u32, u32)>, limit: f32| {
            edges.into_iter()
                .filter_map(|e| dist(&e).map(|d| (e, d)))
                .filter(|(_, d)| *d <= limit)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(e, _)| e)
        };
        let (o, d) = self.camera.ray(rect, pos);
        match mesh.raycast(o, d) {
            Some((f, _)) => {
                let v = &mesh.faces[f].verts;
                nearest((0..v.len()).map(|i| (v[i], v[(i + 1) % v.len()])).collect(), f32::MAX)
            }
            None => nearest(mesh.edges(), 14.0),
        }
    }

    fn loop_cut_step(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, mouse: Option<Pos2>, snap: bool, keys: bool) -> Preview {
        let Some(mut lc) = self.loop_cut.take() else { return Preview::default() };
        let key = |k: Key| keys && ui.input(|i| i.key_pressed(k));
        if key(Key::Escape) || resp.clicked_by(PointerButton::Secondary) {
            return Preview::default();
        }
        let confirm = resp.clicked_by(PointerButton::Primary);
        let enter = key(Key::Enter);
        let preview_of = |mesh: &Mesh, ring: &EdgeRing, fractions: &[f32]| {
            let mut pv = Preview::default();
            for &t in fractions {
                let pts: Vec<Vec3> = ring.edges.iter().map(|&(a, b)| mesh.positions[a as usize].lerp(mesh.positions[b as usize], t)).collect();
                pv.lines.extend(pts.windows(2).map(|w| [w[0], w[1]]));
                if ring.closed && pts.len() > 2 {
                    pv.lines.push([pts[pts.len() - 1], pts[0]]);
                }
                pv.points.extend(pts);
            }
            pv
        };

        match lc.slide.take() {
            // Phase 1: choose the ring by hovering an edge; number keys set how many cuts.
            None => {
                let digits = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
                if let Some(n) = digits.iter().position(|&k| key(k)) {
                    lc.cuts = n + 1;
                }
                let hover = mouse.filter(|m| rect.contains(*m)).and_then(|m| self.pick_edge(rect, m));
                let mut preview = Preview::default();
                if let Some((a, b)) = hover {
                    let ring = self.mesh().edge_ring(a, b);
                    let fractions: Vec<f32> = (1..=lc.cuts).map(|j| j as f32 / (lc.cuts + 1) as f32).collect();
                    preview = preview_of(self.mesh(), &ring, &fractions);
                    if confirm && lc.cuts == 1 {
                        lc.slide = Some((ring, (a, b)));
                        lc.t = 0.5;
                    } else if confirm || enter {
                        self.apply_loop_cut(&ring, &fractions);
                        return Preview::default();
                    }
                }
                self.loop_cut = Some(lc);
                preview
            }
            // Phase 2 (single cut): slide it along the chosen edge.
            Some((ring, (a, b))) => {
                let (pa, pb) = (self.mesh().positions[a as usize], self.mesh().positions[b as usize]);
                if let (Some(m), Some(sa), Some(sb)) = (mouse, self.camera.project(rect, pa), self.camera.project(rect, pb)) {
                    let mut t = ((m - sa).dot(sb - sa) / (sb - sa).length_sq().max(1e-6)).clamp(0.02, 0.98);
                    if snap {
                        // Snap the distance from the start of the edge to the grid step.
                        let len = pa.distance(pb);
                        let d = (t * len / self.grid_snap).round() * self.grid_snap;
                        if d > 1e-5 && d < len - 1e-5 {
                            t = d / len;
                        }
                    }
                    lc.t = t;
                }
                if confirm || enter {
                    self.apply_loop_cut(&ring, &[lc.t]);
                    return Preview::default();
                }
                let preview = preview_of(self.mesh(), &ring, &[lc.t]);
                lc.slide = Some((ring, (a, b)));
                self.loop_cut = Some(lc);
                preview
            }
        }
    }

    fn apply_loop_cut(&mut self, ring: &EdgeRing, fractions: &[f32]) {
        self.push_undo();
        let new = self.mesh_mut().loop_cut(ring, fractions);
        // Leave the new loop selected so it can be moved straight away.
        self.set_mode(SelMode::Vertex);
        self.sel_verts = new.into_iter().collect();
        self.status = "Loop cut added (new vertices selected).".into();
    }

    fn modal_keys(&mut self, ui: &egui::Ui) {
        let Some(m) = &mut self.modal else { return };
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
        if !m.drag {
            for (axis, name, _) in AXES {
                let key = [Key::X, Key::Y, Key::Z][AXES.iter().position(|a| a.1 == name).unwrap()];
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
        let ctrl = |k: Key| ui.input(|i| i.key_pressed(k) && i.modifiers.command && !i.modifiers.shift);
        if ctrl(Key::B) {
            self.start_bevel();
        } else if ctrl(Key::R) {
            self.start_loop_cut();
        } else if pressed(Key::I) {
            self.start_inset();
        } else if pressed(Key::G) {
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
            if alt { self.deselect_all() } else { self.select_all() }
        } else if pressed(Key::L) {
            self.select_linked();
        } else if pressed(Key::O) {
            self.set_mode(SelMode::Object);
        } else if pressed(Key::V) {
            self.set_mode(SelMode::Vertex);
        } else if pressed(Key::F) {
            self.set_mode(SelMode::Face);
        } else if pressed(Key::Q) {
            self.select_tool = SelectTool::Click;
        } else if pressed(Key::B) {
            self.select_tool = SelectTool::Box;
        } else if pressed(Key::C) {
            self.select_tool = SelectTool::Circle;
        } else if pressed(Key::OpenBracket) {
            self.circle_radius = (self.circle_radius - 5.0).max(5.0);
        } else if pressed(Key::CloseBracket) {
            self.circle_radius = (self.circle_radius + 5.0).min(300.0);
        } else if pressed(Key::U) {
            self.show_uv = !self.show_uv;
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
            SelMode::Object => {
                // Nearest visible object under the cursor becomes the active one.
                let hit = self.objects.iter().enumerate()
                    .filter(|(_, o)| o.visible)
                    .filter_map(|(i, o)| o.mesh.raycast(origin, dir).map(|(_, t)| (i, t)))
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                match hit {
                    Some((i, _)) => self.select_object(i, additive),
                    None if !additive => self.sel_objects.clear(),
                    None => {}
                }
            }
            SelMode::Face => {
                let hit = self.mesh().raycast(origin, dir).map(|(f, _)| f);
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
                let mesh = self.mesh();
                let mut best: Option<(u32, f32)> = None;
                for (i, &p) in mesh.positions.iter().enumerate() {
                    let Some(sp) = self.camera.project(rect, p) else { continue };
                    let d = sp.distance(pos);
                    if d > 12.0 || best.is_some_and(|(_, bd)| bd <= d) {
                        continue;
                    }
                    if !self.point_visible(rect, p, sp, None) {
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

    /// Is the point `p` (projected at `sp`) of the active mesh not hidden behind one of its faces?
    /// `own_face` is ignored as an occluder (used for face centres).
    fn point_visible(&self, rect: egui::Rect, p: Vec3, sp: Pos2, own_face: Option<usize>) -> bool {
        let (o, dir) = self.camera.ray(rect, sp);
        let dist = (p - o).dot(dir);
        match self.mesh().raycast(o, dir) {
            Some((f, t)) => Some(f) == own_face || t >= dist - 1e-3 * dist.max(1.0),
            None => true,
        }
    }

    fn current_sel(&self) -> Sel {
        Sel { objects: self.sel_objects.clone(), verts: self.sel_verts.clone(), faces: self.sel_faces.clone() }
    }

    fn set_sel(&mut self, sel: Sel) {
        match self.sel_mode {
            SelMode::Object => {
                // Keep "active" on a selected object when possible.
                if !sel.objects.contains(&self.active) {
                    if let Some(&first) = sel.objects.iter().next() {
                        self.set_active(first);
                    }
                }
                self.sel_objects = sel.objects;
            }
            SelMode::Vertex => self.sel_verts = sel.verts,
            SelMode::Face => self.sel_faces = sel.faces,
        }
    }

    /// Everything of the current mode's kind inside a screen region.
    /// `centre` is also ray-cast so a region lying wholly inside one big face still catches it.
    fn region_hits(&self, rect: egui::Rect, inside: impl Fn(Pos2) -> bool, centre: Pos2) -> Sel {
        let mut hits = Sel::default();
        let centre_ray = rect.contains(centre).then(|| self.camera.ray(rect, centre));
        match self.sel_mode {
            SelMode::Object => {
                for (i, obj) in self.objects.iter().enumerate() {
                    if obj.visible && obj.mesh.positions.iter().any(|&p| self.camera.project(rect, p).is_some_and(&inside)) {
                        hits.objects.insert(i);
                    }
                }
                if let Some((o, d)) = centre_ray {
                    let nearest = self.objects.iter().enumerate()
                        .filter(|(_, obj)| obj.visible)
                        .filter_map(|(i, obj)| obj.mesh.raycast(o, d).map(|(_, t)| (i, t)))
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    hits.objects.extend(nearest.map(|(i, _)| i));
                }
            }
            SelMode::Vertex => {
                for (i, &p) in self.mesh().positions.iter().enumerate() {
                    let Some(sp) = self.camera.project(rect, p) else { continue };
                    if inside(sp) && (!self.only_visible || self.point_visible(rect, p, sp, None)) {
                        hits.verts.insert(i as u32);
                    }
                }
            }
            SelMode::Face => {
                let mesh = self.mesh();
                for f in 0..mesh.faces.len() {
                    let c = mesh.face_center(f);
                    let Some(sp) = self.camera.project(rect, c) else { continue };
                    if inside(sp) && (!self.only_visible || self.point_visible(rect, c, sp, Some(f))) {
                        hits.faces.insert(f);
                    }
                }
                if let Some((o, d)) = centre_ray {
                    hits.faces.extend(mesh.raycast(o, d).map(|(f, _)| f));
                }
            }
        }
        hits
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

        let active_verts = self.active_verts();
        for (oi, obj) in self.objects.iter().enumerate() {
            if !obj.visible {
                continue;
            }
            let mesh = &obj.mesh;
            let is_active = oi == self.active;
            let face_mode = is_active && self.sel_mode == SelMode::Face;
            for (fi, face) in mesh.faces.iter().enumerate() {
                let normal = mesh.face_normal(fi);
                let tint = if face_mode && self.sel_faces.contains(&fi) { [1.0, 0.55, 0.1, 0.35] } else { [0.0; 4] };
                for k in 1..face.verts.len() - 1 {
                    for c in [0, k, k + 1] {
                        tris.push(mesh.positions[face.verts[c] as usize], normal, face.uvs[c], tint);
                    }
                }
            }
            let object_mode = self.sel_mode == SelMode::Object;
            let object_selected = object_mode && self.sel_objects.contains(&oi);
            for (a, b) in mesh.edges() {
                // Selected objects are outlined whole (the active one brighter); in edit modes only selected edges light up.
                let selected = object_selected || (!object_mode && is_active && active_verts.contains(&a) && active_verts.contains(&b));
                let c = if !selected {
                    [0.04, 0.04, 0.06, 0.9]
                } else if object_mode && is_active {
                    [1.0, 0.8, 0.3, 1.0]
                } else {
                    ORANGE
                };
                lines.line(mesh.positions[a as usize], mesh.positions[b as usize], c);
            }
            if is_active && self.sel_mode == SelMode::Vertex {
                for (i, &p) in mesh.positions.iter().enumerate() {
                    let c = if self.sel_verts.contains(&(i as u32)) { ORANGE } else { [0.0, 0.0, 0.0, 1.0] };
                    points.point(p, c);
                }
            }
        }
        if let Some(m) = &self.modal {
            if let Some(a) = m.axis {
                lines.line(m.pivot - a * 50.0, m.pivot + a * 50.0, [1.0, 1.0, 1.0, 0.5]);
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

        let (mods, scroll, pinch, mouse, press, primary_down) = ui.input(|i| {
            (i.modifiers, i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.latest_pos(), i.pointer.press_origin(), i.pointer.primary_down())
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
        let mut box_rect: Option<egui::Rect> = None;
        let mut preview = Preview::default();
        // Snapping is on when the toggle is set; holding Ctrl flips it for the moment.
        let snap = self.snap_on != mods.command;
        let keyboard_free = !ui.ctx().egui_wants_keyboard_input();
        let gizmo_before = if self.show_gizmo && !self.busy() { self.gizmo(rect) } else { None };
        let over_gizmo = |p: Option<Pos2>| gizmo_before.as_ref().zip(p).and_then(|(g, p)| g.hit(p));

        if let Some(is_drag) = self.modal.as_ref().map(|m| m.drag) {
            if keyboard_free {
                self.modal_keys(ui);
            }
            if let Some(mouse) = mouse {
                self.update_modal(rect, mouse, snap);
            }
            if resp.clicked_by(PointerButton::Secondary) {
                self.cancel_modal();
            } else if (is_drag && !primary_down) || (!is_drag && resp.clicked_by(PointerButton::Primary)) {
                self.modal = None;
            }
            ui.ctx().request_repaint();
        } else if self.op.is_some() {
            self.op_step(ui, &resp, rect, mouse, snap, keyboard_free);
            ui.ctx().request_repaint();
        } else if self.loop_cut.is_some() {
            preview = self.loop_cut_step(ui, &resp, rect, mouse, snap, keyboard_free);
            ui.ctx().request_repaint();
        } else {
            if resp.hovered() && keyboard_free {
                self.viewport_keys(ui);
            }
            let grabbed = over_gizmo(press);
            let op = if mods.command { SelOp::Subtract } else if mods.shift { SelOp::Add } else { SelOp::Replace };
            let circle = |c: Pos2, r: f32| move |p: Pos2| p.distance(c) <= r;
            if resp.drag_started_by(PointerButton::Primary) && !mods.alt {
                if let (None, true, Some(start)) = (grabbed, self.select_tool != SelectTool::Click, press) {
                    // Begin a box or circle selection.
                    let mut base = self.current_sel();
                    if self.select_tool == SelectTool::Circle {
                        // A circle stroke paints: start from nothing unless adding / removing, then dab at the press point.
                        if op == SelOp::Replace {
                            base = Sel::default();
                        }
                        let hits = self.region_hits(rect, circle(start, self.circle_radius), start);
                        base = combine(&base, &hits, if op == SelOp::Subtract { SelOp::Subtract } else { SelOp::Add });
                        self.set_sel(base.clone());
                    }
                    self.region = Some(Region { start, op, base });
                }
                if let Some(axis) = grabbed {
                    // Pull an arrow: an axis-locked move that follows the drag.
                    self.start_modal(ModalKind::Grab, Some(AXES[axis].0));
                    if let Some(m) = &mut self.modal {
                        m.axis_name = AXES[axis].1;
                        m.start_mouse = press;
                        m.drag = true;
                    }
                }
            } else if resp.clicked_by(PointerButton::Primary) && !mods.alt && grabbed.is_none() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    if self.select_tool == SelectTool::Circle {
                        // A single dab of the circle brush.
                        let base = if op == SelOp::Replace { Sel::default() } else { self.current_sel() };
                        let hits = self.region_hits(rect, circle(pos, self.circle_radius), pos);
                        self.set_sel(combine(&base, &hits, if op == SelOp::Subtract { SelOp::Subtract } else { SelOp::Add }));
                    } else {
                        self.pick(rect, pos, mods.shift || mods.command);
                    }
                }
            }

            // Continue a box / circle drag.
            if let Some(region) = self.region.take() {
                if let Some(mouse) = mouse {
                    match self.select_tool {
                        SelectTool::Box => {
                            let r = egui::Rect::from_two_pos(region.start, mouse);
                            box_rect = Some(r);
                            let hits = self.region_hits(rect, |p| r.contains(p), r.center());
                            self.set_sel(combine(&region.base, &hits, region.op));
                        }
                        SelectTool::Circle => {
                            let hits = self.region_hits(rect, circle(mouse, self.circle_radius), mouse);
                            let cur = self.current_sel();
                            self.set_sel(combine(&cur, &hits, if region.op == SelOp::Subtract { SelOp::Subtract } else { SelOp::Add }));
                        }
                        SelectTool::Click => {}
                    }
                }
                if primary_down {
                    self.region = Some(region);
                }
            }
        }

        // ---- draw scene
        if let Some(renderer) = self.renderer.clone() {
            let frame = Arc::new(self.build_frame(rect));
            painter.add(egui::PaintCallback {
                rect,
                callback: Arc::new(egui_glow::CallbackFn::new(move |_info, painter| {
                    renderer.lock().unwrap().paint(painter.gl(), &frame);
                })),
            });
        }

        // ---- draw gizmo on top (recomputed so it follows the selection while dragging)
        let dragging_axis = self.modal.as_ref().filter(|m| m.drag).and_then(|m| AXES.iter().position(|a| Some(a.0) == m.axis));
        let show = self.show_gizmo && self.modal.as_ref().is_none_or(|m| m.drag) && self.op.is_none() && self.loop_cut.is_none();
        if let Some(g) = show.then(|| self.gizmo(rect)).flatten() {
            let hovered = dragging_axis.or_else(|| resp.hover_pos().and_then(|p| g.hit(p)));
            // Far arrows first so near ones overlap them.
            let back = self.camera.back();
            let mut order = [0, 1, 2];
            order.sort_by(|&a, &b| AXES[a].0.dot(back).total_cmp(&AXES[b].0.dot(back)));
            for i in order {
                let Some(tip) = g.tips[i] else { continue };
                if dragging_axis.is_some_and(|d| d != i) {
                    continue;
                }
                let color = if hovered == Some(i) { Color32::from_rgb(255, 235, 120) } else { AXES[i].2 };
                let dir = (tip - g.centre).normalized();
                let side = egui::vec2(-dir.y, dir.x);
                let neck = tip - dir * 14.0;
                painter.line_segment([g.centre, neck], Stroke::new(3.0, color));
                painter.add(egui::Shape::convex_polygon(vec![tip, neck + side * 6.0, neck - side * 6.0], color, Stroke::NONE));
                painter.text(tip + dir * 10.0, egui::Align2::CENTER_CENTER, AXES[i].1, egui::FontId::proportional(12.0), color);
            }
            painter.circle_filled(g.centre, 3.5, Color32::WHITE);
        }

        // ---- selection tool overlays
        let clip = painter.with_clip_rect(rect);
        if let Some(r) = box_rect {
            clip.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(255, 170, 60, 25));
            clip.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgb(255, 190, 90)), egui::StrokeKind::Middle);
        }
        let cut_color = Color32::from_rgb(255, 230, 70);
        for [a, b] in &preview.lines {
            if let (Some(a), Some(b)) = (self.camera.project(rect, *a), self.camera.project(rect, *b)) {
                clip.line_segment([a, b], Stroke::new(2.0, cut_color));
            }
        }
        for p in &preview.points {
            if let Some(p) = self.camera.project(rect, *p) {
                clip.circle_filled(p, 3.0, cut_color);
            }
        }
        if self.select_tool == SelectTool::Circle && !self.busy() {
            if let Some(p) = resp.hover_pos() {
                clip.circle_stroke(p, self.circle_radius, Stroke::new(1.0, Color32::from_rgb(255, 190, 90)));
            }
        }

        // ---- overlay text
        let font = egui::FontId::proportional(13.0);
        let snap_note = if snap { format!("   snap {}", self.grid_snap) } else { String::new() };
        let tool_hint = if let Some(op) = &self.op {
            let what = if op.kind == OpKind::Inset { "Inset" } else { "Bevel" };
            Some(format!("{what}:  {:.3}{snap_note}    move the mouse right for more   [Ctrl] flip snap   [Click/Enter] confirm   [Esc/Right-click] cancel", op.value))
        } else if let Some(lc) = &self.loop_cut {
            Some(if lc.slide.is_some() {
                format!("Loop cut: slide {:.0}%{snap_note}    [Click/Enter] confirm   [Ctrl] flip snap   [Esc/Right-click] cancel", lc.t * 100.0)
            } else {
                format!("Loop cut: hover an edge and click   cuts: {}   [1-9] number of cuts   [Esc/Right-click] cancel", lc.cuts)
            })
        } else {
            None
        };
        let hint = match &self.modal {
            _ if tool_hint.is_some() => tool_hint.unwrap(),
            Some(m) => {
                let what = match m.kind {
                    ModalKind::Grab => "Move",
                    ModalKind::Rotate => "Rotate",
                    ModalKind::Scale => "Scale",
                };
                let axis = if m.axis_name.is_empty() { String::new() } else { format!(" along {}", m.axis_name) };
                let snap_note = if snap { format!("   snap {}", self.grid_snap) } else { String::new() };
                if m.drag {
                    format!("{what}{axis}:  {}{snap_note}    [Ctrl] flip snap   [Esc/Right-click] cancel", m.readout)
                } else {
                    format!("{what}{axis}:  {}{snap_note}    [X/Y/Z] axis   [Ctrl] flip snap   [Click/Enter] confirm   [Esc/Right-click] cancel", m.readout)
                }
            }
            None => match self.sel_mode {
                _ if self.select_tool == SelectTool::Box => "Drag a box to select · Shift add · Ctrl remove · Q back to click select",
                _ if self.select_tool == SelectTool::Circle => "Paint with the circle to select · Shift add · Ctrl remove · [ ] radius · Q back to click select",
                SelMode::Object => "Click an object to select · Shift+click add · Drag an arrow to move · G/R/S transform · Tab to edit faces",
                _ => "Click select · Shift+click add · Drag an arrow to move · G/R/S transform · E extrude · Tab back to objects",
            }.to_string(),
        };
        painter.text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, hint, font.clone(), Color32::from_gray(200));
        let mode = match self.sel_mode {
            SelMode::Object => "Object mode",
            SelMode::Vertex => "Vertex mode",
            SelMode::Face => "Face mode",
        };
        let name = self.objects.get(self.active).map_or("no object", |o| o.name.as_str());
        let proj = if self.camera.ortho { "Ortho" } else { "Perspective" };
        let name = if self.sel_mode == SelMode::Object && self.sel_objects.len() != 1 {
            format!("{} objects selected", self.sel_objects.len())
        } else {
            name.to_string()
        };
        let tool = match self.select_tool {
            SelectTool::Click => "",
            SelectTool::Box => " · Box select",
            SelectTool::Circle => " · Circle select",
        };
        let snap_txt = if self.snap_on { format!(" · Snap {}", self.grid_snap) } else { String::new() };
        painter.text(rect.left_top() + egui::vec2(8.0, 8.0), egui::Align2::LEFT_TOP, format!("{mode} · {name} · {proj}{tool}{snap_txt}"), font, Color32::from_gray(200));
    }
}

impl Gizmo {
    /// Which arrow (0 = X, 1 = Y, 2 = Z) is under the screen position.
    fn hit(&self, p: Pos2) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, tip) in self.tips.iter().enumerate() {
            let Some(tip) = *tip else { continue };
            let d = segment_distance(p, self.centre, tip);
            if d <= 9.0 && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
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
