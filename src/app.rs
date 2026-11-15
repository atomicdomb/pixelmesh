//! Application state, menus, tool panel and file handling.

use crate::camera::Camera;
use crate::render::Renderer;
use crate::uv_editor::UvEditor;
use crate::viewport::Modal;
use eframe::egui::{self, Key};
use glam::{Vec2, Vec3};
use pixelmesh::fbx;
use pixelmesh::mesh::Mesh;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SelMode {
    /// Whole objects: click picks an object, transforms move all of it.
    Object,
    Vertex,
    Edge,
    Face,
}

/// An edge as its two vertex indices, smaller first.
pub type Edge = (u32, u32);

pub fn edge_key(a: u32, b: u32) -> Edge {
    (a.min(b), a.max(b))
}

/// How left-click / left-drag selects in the viewport.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SelectTool {
    Click,
    /// Drag a rectangle.
    Box,
    /// Paint with a circle around the cursor.
    Circle,
}

/// One named mesh in the scene. Transforms are baked straight into the vertices.
#[derive(Clone, Serialize, Deserialize)]
pub struct Object {
    pub name: String,
    pub mesh: Mesh,
    #[serde(default = "yes")]
    pub visible: bool,
    /// The object's pivot (world space). Object-mode rotation and scaling turn about it,
    /// and it becomes the model's position in exported FBX files.
    #[serde(default)]
    pub origin: Vec3,
    /// Orientation as XYZ Euler angles in degrees. The vertices are stored already
    /// rotated; this records by how much, so it can be shown, edited and exported.
    #[serde(default)]
    pub rotation: Vec3,
}

impl Object {
    pub fn new(name: impl Into<String>, mesh: Mesh) -> Self {
        Object { name: name.into(), mesh, visible: true, origin: Vec3::ZERO, rotation: Vec3::ZERO }
    }

    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = *self.mesh.positions.first()?;
        Some(self.mesh.positions.iter().fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))))
    }
}

/// Where "set origin" puts an object's pivot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OriginTo {
    Centre,
    Base,
    World,
    Selection,
}

fn yes() -> bool {
    true
}

pub struct Snapshot {
    objects: Vec<Object>,
    active: usize,
    sel_objects: BTreeSet<usize>,
    sel_edges: BTreeSet<Edge>,
    sel_verts: BTreeSet<u32>,
    sel_faces: BTreeSet<usize>,
}

/// What the left panel shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeftTab {
    Uv,
    Paint,
}

/// One step on the undo stack: either the scene or the texture's pixels as they were.
enum UndoStep {
    Scene(Snapshot),
    Pixels { rgba: Arc<Vec<u8>>, size: [u32; 2] },
}

pub struct Texture {
    pub path: PathBuf,
    pub size: [u32; 2],
    pub rgba: Arc<Vec<u8>>,
    pub file_bytes: Vec<u8>,
    pub handle: egui::TextureHandle,
    pub version: u64,
    mtime: Option<SystemTime>,
    /// Painted on since it was last loaded or saved.
    pub dirty: bool,
    /// False for a new canvas that has never been saved.
    pub on_disk: bool,
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    version: u32,
    #[serde(default)]
    objects: Vec<Object>,
    /// Version 1 files stored a single mesh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mesh: Option<Mesh>,
    texture: Option<String>,
    px_per_unit: f32,
}

pub struct App {
    pub objects: Vec<Object>,
    /// Index of the object being edited / moved.
    pub active: usize,
    /// Objects selected in object mode (moved, deleted, duplicated or merged together).
    pub sel_objects: BTreeSet<usize>,
    pub select_tool: SelectTool,
    /// Radius of the circle-select brush in screen pixels.
    pub circle_radius: f32,
    /// Box / circle select skip vertices and faces hidden behind geometry.
    pub only_visible: bool,
    pub region: Option<crate::viewport::Region>,
    /// Inset / bevel being sized with the mouse.
    pub op: Option<crate::viewport::OpModal>,
    /// Loop cut being placed.
    pub loop_cut: Option<crate::viewport::LoopCut>,
    empty: Mesh,
    pub sel_mode: SelMode,
    last_edit_mode: SelMode,
    pub sel_verts: BTreeSet<u32>,
    pub sel_edges: BTreeSet<Edge>,
    pub sel_faces: BTreeSet<usize>,
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
    pub left_tab: LeftTab,
    pub paint: crate::paint::Paint,
    /// Draw data per distinct mesh, and the two cached layers built from it (see `build_frame`).
    pub geom_cache: std::collections::HashMap<u64, Arc<crate::viewport::ObjGeom>>,
    pub layers: [crate::render::Layer; crate::render::LAYERS],
    /// Prefix groups currently unfolded in the object list.
    pub expanded_groups: BTreeSet<String>,
    /// Interface icons, uploaded once at start-up.
    pub icons: std::collections::HashMap<pixelmesh::icons::Icon, egui::TextureHandle>,

    pub camera: Camera,
    pub renderer: Option<Arc<Mutex<Renderer>>>,
    pub modal: Option<Modal>,
    pub viewport_rect: egui::Rect,
    pub uv: UvEditor,

    pub texture: Option<Texture>,
    tex_counter: u64,
    last_mtime_check: f64,

    /// Object mode: moving affects only the origin, not the geometry.
    pub origin_only: bool,
    pub snap_on: bool,
    pub grid_snap: f32,
    pub show_uv: bool,
    pub show_gizmo: bool,
    pub px_per_unit: f32,
    pub segments: u32,
    pub embed_texture: bool,
    project_path: Option<PathBuf>,
    pub status: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: &[String]) -> Self {
        let renderer = match cc.gl.as_ref().map(|gl| Renderer::new(gl)) {
            Some(Ok(r)) => Some(Arc::new(Mutex::new(r))),
            Some(Err(e)) => {
                eprintln!("renderer init failed: {e}");
                None
            }
            None => None,
        };
        crate::sidebar::apply_theme(&cc.egui_ctx);
        let mut app = App {
            objects: vec![Object::new("Cube", Mesh::cube())],
            active: 0,
            sel_objects: [0].into(),
            select_tool: SelectTool::Click,
            circle_radius: 40.0,
            only_visible: true,
            region: None,
            op: None,
            loop_cut: None,
            empty: Mesh::default(),
            sel_mode: SelMode::Object,
            last_edit_mode: SelMode::Face,
            sel_verts: BTreeSet::new(),
            sel_edges: BTreeSet::new(),
            sel_faces: BTreeSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            left_tab: LeftTab::Uv,
            paint: crate::paint::Paint::default(),
            geom_cache: std::collections::HashMap::new(),
            layers: Default::default(),
            expanded_groups: BTreeSet::new(),
            icons: {
                // Icons come from assets/icons.png (compiled into the program); anything
                // that image does not contain falls back to the built-in drawing.
                let atlas = image::load_from_memory(include_bytes!("../assets/icons.png")).ok().map(|i| i.to_rgba8());
                pixelmesh::icons::ALL.iter().map(|&icon| {
                    let size = pixelmesh::icons::SIZE as usize;
                    let px = atlas.as_ref()
                        .and_then(|a| pixelmesh::icons::from_atlas(a.as_raw(), a.width(), a.height(), icon))
                        .unwrap_or_else(|| pixelmesh::icons::pixels(icon));
                    let img = egui::ColorImage::from_rgba_unmultiplied([size, size], &px);
                    (icon, cc.egui_ctx.load_texture(format!("icon-{icon:?}"), img, egui::TextureOptions::NEAREST))
                }).collect()
            },
            camera: Camera::default(),
            renderer,
            modal: None,
            viewport_rect: egui::Rect::NOTHING,
            uv: UvEditor::default(),
            texture: None,
            tex_counter: 0,
            last_mtime_check: 0.0,
            origin_only: false,
            snap_on: false,
            grid_snap: 0.25,
            show_uv: true,
            show_gizmo: true,
            px_per_unit: 16.0,
            segments: 8,
            embed_texture: false,
            project_path: None,
            status: "Welcome. Add meshes, drag the arrows to move them, press Tab to edit faces.".into(),
        };
        for a in args {
            let p = Path::new(a);
            match p.extension().and_then(|e| e.to_str()) {
                Some("png" | "jpg" | "jpeg" | "bmp" | "tga") => app.load_texture(&cc.egui_ctx, p),
                Some("fbx" | "FBX") => {
                    // Opening an FBX directly: start from just that file.
                    if app.undo.is_empty() {
                        app.objects.clear();
                        app.sel_objects.clear();
                    }
                    app.import_fbx(&cc.egui_ctx, p);
                }
                Some("pmesh") => app.open_project(&cc.egui_ctx, p),
                _ => {}
            }
        }
        app
    }

    // --------------------------------------------------------------- objects

    /// True while a tool that follows the mouse is running; menus and panels are locked meanwhile.
    pub fn busy(&self) -> bool {
        self.modal.is_some() || self.op.is_some() || self.loop_cut.is_some()
    }

    /// Mesh of the active object (an empty mesh when the scene has no objects).
    pub fn mesh(&self) -> &Mesh {
        self.objects.get(self.active).map_or(&self.empty, |o| &o.mesh)
    }

    pub fn mesh_mut(&mut self) -> &mut Mesh {
        if self.active < self.objects.len() {
            &mut self.objects[self.active].mesh
        } else {
            self.empty = Mesh::default();
            &mut self.empty
        }
    }

    pub fn set_active(&mut self, index: usize) {
        if index != self.active && index < self.objects.len() {
            self.active = index;
            self.clear_selection();
        }
    }

    /// Select an object. With `toggle` (Shift/Ctrl+click) it is added to or removed from the selection.
    pub fn select_object(&mut self, index: usize, toggle: bool) {
        if !toggle {
            self.sel_objects = [index].into();
            self.set_active(index);
        } else if self.sel_objects.remove(&index) {
            // Deselected the active one: hand "active" to another selected object.
            if index == self.active {
                if let Some(&other) = self.sel_objects.iter().next() {
                    self.set_active(other);
                }
            }
        } else {
            self.sel_objects.insert(index);
            self.set_active(index);
        }
    }

    /// (object, vertex) pairs the transform tools act on.
    pub fn transform_targets(&self) -> Vec<(usize, u32)> {
        match self.sel_mode {
            SelMode::Object if self.origin_only => Vec::new(),
            SelMode::Object => self.sel_objects.iter()
                .filter(|&&i| self.objects.get(i).is_some_and(|o| o.visible))
                .flat_map(|&i| (0..self.objects[i].mesh.positions.len() as u32).map(move |v| (i, v)))
                .collect(),
            _ => self.active_verts().into_iter().map(|v| (self.active, v)).collect(),
        }
    }

    /// Selected, visible objects (object mode only): the ones whose origin follows a transform.
    pub fn transform_objects(&self) -> Vec<usize> {
        if self.sel_mode != SelMode::Object {
            return Vec::new();
        }
        self.sel_objects.iter().copied().filter(|&i| self.objects.get(i).is_some_and(|o| o.visible)).collect()
    }

    /// Move the origin of the selected objects (object mode) or the edited object without touching the geometry.
    pub fn set_origin(&mut self, to: OriginTo) {
        let targets: Vec<usize> = if self.sel_mode == SelMode::Object {
            self.sel_objects.iter().copied().collect()
        } else if self.active < self.objects.len() {
            vec![self.active]
        } else {
            Vec::new()
        };
        let selection = self.active_verts();
        let mut moves = Vec::new();
        for i in targets {
            let obj = &self.objects[i];
            let target = match to {
                OriginTo::World => Some(Vec3::ZERO),
                OriginTo::Centre => obj.bounds().map(|(lo, hi)| (lo + hi) * 0.5),
                OriginTo::Base => obj.bounds().map(|(lo, hi)| Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5)),
                OriginTo::Selection if i == self.active && !selection.is_empty() => {
                    Some(selection.iter().map(|&v| obj.mesh.positions[v as usize]).sum::<Vec3>() / selection.len() as f32)
                }
                OriginTo::Selection => None,
            };
            moves.extend(target.map(|t| (i, t)));
        }
        if moves.is_empty() {
            self.status = "Nothing to set the origin from.".into();
            return;
        }
        self.push_undo();
        for (i, t) in moves {
            self.objects[i].origin = t;
        }
        self.status = "Origin moved (geometry unchanged).".into();
    }

    /// Join every selected object into the active one.
    pub fn merge_selected(&mut self) {
        if self.sel_mode != SelMode::Object || self.sel_objects.len() < 2 {
            self.status = "Merge needs two or more objects selected in object mode (Shift+click to add).".into();
            return;
        }
        let target = if self.sel_objects.contains(&self.active) { self.active } else { *self.sel_objects.iter().next().unwrap() };
        self.push_undo();
        let others: Vec<usize> = self.sel_objects.iter().copied().filter(|&i| i != target).collect();
        for &i in &others {
            let mesh = self.objects[i].mesh.clone();
            self.objects[target].mesh.append(&mesh);
        }
        for &i in others.iter().rev() {
            self.objects.remove(i);
        }
        let target = target - others.iter().filter(|&&i| i < target).count();
        self.active = target;
        self.sel_objects = [target].into();
        self.clear_selection();
        self.status = format!("Merged {} objects into \"{}\"", others.len() + 1, self.objects[target].name);
    }

    pub fn unique_name(&self, base: &str) -> String {
        let taken = |n: &str| self.objects.iter().any(|o| o.name == n);
        if !taken(base) {
            return base.to_string();
        }
        (1..).map(|i| format!("{base}.{i:03}")).find(|n| !taken(n)).unwrap()
    }

    // ------------------------------------------------------------ selection

    /// Vertices the transform tools act on.
    pub fn active_verts(&self) -> BTreeSet<u32> {
        match self.sel_mode {
            SelMode::Object => (0..self.mesh().positions.len() as u32).collect(),
            SelMode::Vertex => self.sel_verts.clone(),
            SelMode::Edge => self.sel_edges.iter().flat_map(|&(a, b)| [a, b]).collect(),
            SelMode::Face => self.mesh().verts_of_faces(&self.sel_faces),
        }
    }

    fn face_edges(&self, f: usize) -> Vec<Edge> {
        let v = &self.mesh().faces[f].verts;
        (0..v.len()).map(|i| edge_key(v[i], v[(i + 1) % v.len()])).collect()
    }

    /// Faces the face / UV tools act on. In vertex mode: faces whose corners are all
    /// selected; in edge mode: faces whose edges are all selected.
    pub fn active_faces(&self) -> BTreeSet<usize> {
        let count = self.mesh().faces.len();
        match self.sel_mode {
            SelMode::Object => (0..count).collect(),
            SelMode::Face => self.sel_faces.clone(),
            SelMode::Vertex => (0..count)
                .filter(|&f| self.mesh().faces[f].verts.iter().all(|v| self.sel_verts.contains(v)))
                .collect(),
            SelMode::Edge => (0..count)
                .filter(|&f| self.face_edges(f).iter().all(|e| self.sel_edges.contains(e)))
                .collect(),
        }
    }

    /// Select these faces in whatever edit mode is current (their vertices / edges in those modes).
    pub fn select_faces(&mut self, faces: BTreeSet<usize>) {
        self.sel_verts = self.mesh().verts_of_faces(&faces);
        self.sel_edges = faces.iter().flat_map(|&f| self.face_edges(f)).collect();
        self.sel_faces = faces;
    }

    pub fn set_mode(&mut self, mode: SelMode) {
        // Carry the selection over so switching between edit modes does not lose it.
        match (self.sel_mode, mode) {
            (SelMode::Face, SelMode::Vertex) => self.sel_verts = self.mesh().verts_of_faces(&self.sel_faces),
            (SelMode::Face, SelMode::Edge) => self.sel_edges = self.sel_faces.iter().flat_map(|&f| self.face_edges(f)).collect(),
            (SelMode::Edge, SelMode::Vertex) => self.sel_verts = self.active_verts(),
            (SelMode::Vertex, SelMode::Edge) => {
                self.sel_edges = self.mesh().edges().into_iter().filter(|(a, b)| self.sel_verts.contains(a) && self.sel_verts.contains(b)).collect()
            }
            (SelMode::Vertex | SelMode::Edge, SelMode::Face) => self.sel_faces = self.active_faces(),
            _ => {}
        }
        if mode != SelMode::Object {
            self.last_edit_mode = mode;
        } else if self.sel_objects.is_empty() && self.active < self.objects.len() {
            // Coming back from editing: the edited object is the selected one.
            self.sel_objects.insert(self.active);
        }
        self.region = None;
        self.sel_mode = mode;
    }

    /// Tab: switch between object mode and the last used edit mode.
    pub fn toggle_edit_mode(&mut self) {
        let mode = if self.sel_mode == SelMode::Object { self.last_edit_mode } else { SelMode::Object };
        self.set_mode(mode);
    }

    pub fn select_all(&mut self) {
        if self.sel_mode == SelMode::Object {
            self.sel_objects = (0..self.objects.len()).filter(|&i| self.objects[i].visible).collect();
            return;
        }
        self.sel_verts = (0..self.mesh().positions.len() as u32).collect();
        self.sel_edges = self.mesh().edges().into_iter().collect();
        self.sel_faces = (0..self.mesh().faces.len()).collect();
    }

    pub fn clear_selection(&mut self) {
        self.sel_verts.clear();
        self.sel_edges.clear();
        self.sel_faces.clear();
        self.uv.sel.clear();
    }

    /// Deselect everything in the current mode.
    pub fn deselect_all(&mut self) {
        if self.sel_mode == SelMode::Object {
            self.sel_objects.clear();
        } else {
            self.clear_selection();
        }
    }

    pub fn select_linked(&mut self) {
        let mesh = self.mesh();
        let seed: BTreeSet<usize> = match self.sel_mode {
            SelMode::Object => return,
            SelMode::Face => self.sel_faces.clone(),
            SelMode::Vertex | SelMode::Edge => {
                let verts = self.active_verts();
                (0..mesh.faces.len()).filter(|&f| mesh.faces[f].verts.iter().any(|v| verts.contains(v))).collect()
            }
        };
        let faces = mesh.linked_faces(&seed);
        self.select_faces(faces);
    }

    // ----------------------------------------------------------------- undo

    fn snapshot(&self) -> Snapshot {
        Snapshot { objects: self.objects.clone(), active: self.active, sel_objects: self.sel_objects.clone(), sel_edges: self.sel_edges.clone(), sel_verts: self.sel_verts.clone(), sel_faces: self.sel_faces.clone() }
    }

    fn restore(&mut self, s: Snapshot) {
        self.objects = s.objects;
        self.active = s.active;
        self.sel_objects = s.sel_objects;
        // UV selections refer to corners of the mesh as it was; do not carry them across an undo.
        self.uv.sel.clear();
        self.sel_verts = s.sel_verts;
        self.sel_edges = s.sel_edges;
        self.sel_faces = s.sel_faces;
    }

    /// Call before every change to the mesh.
    pub fn push_undo(&mut self) {
        self.push_step(UndoStep::Scene(self.snapshot()));
    }

    /// Record the texture's pixels as they were before a paint stroke.
    pub fn push_pixels_undo(&mut self, rgba: Arc<Vec<u8>>, size: [u32; 2]) {
        self.push_step(UndoStep::Pixels { rgba, size });
    }

    fn push_step(&mut self, step: UndoStep) {
        self.undo.push(step);
        // Keep at most 200 steps, and no more than about 256 MB: a snapshot of a big scene is not small.
        let size = |s: &UndoStep| match s {
            UndoStep::Pixels { rgba, .. } => rgba.len(),
            UndoStep::Scene(snap) => snap.objects.iter().map(|o| {
                o.mesh.positions.len() * 12 + o.mesh.faces.iter().map(|f| 48 + f.verts.len() * 12).sum::<usize>()
            }).sum(),
        };
        let mut total: usize = self.undo.iter().map(size).sum();
        while self.undo.len() > 200 || (total > 256 << 20 && self.undo.len() > 4) {
            total -= size(&self.undo.remove(0));
        }
        self.redo.clear();
    }

    /// Apply a step and return the step that reverses it.
    fn swap_step(&mut self, step: UndoStep) -> Option<UndoStep> {
        match step {
            UndoStep::Scene(s) => {
                let inverse = UndoStep::Scene(self.snapshot());
                self.restore(s);
                Some(inverse)
            }
            UndoStep::Pixels { rgba, size } => {
                let t = self.texture.as_mut()?;
                let inverse = UndoStep::Pixels { rgba: t.rgba.clone(), size: t.size };
                t.rgba = rgba;
                t.size = size;
                self.texture_changed();
                Some(inverse)
            }
        }
    }

    pub fn undo(&mut self) {
        if let Some(step) = self.undo.pop() {
            if let Some(inverse) = self.swap_step(step) {
                self.redo.push(inverse);
            }
        }
    }

    pub fn redo(&mut self) {
        if let Some(step) = self.redo.pop() {
            if let Some(inverse) = self.swap_step(step) {
                self.undo.push(inverse);
            }
        }
    }

    /// Throw away the most recent undo step and go back to it (used when a modal tool is cancelled).
    pub fn revert_last(&mut self) {
        if let Some(UndoStep::Scene(s)) = self.undo.pop() {
            self.restore(s);
        }
    }

    // ------------------------------------------------------------- mesh ops

    /// In object mode (or an empty scene) a primitive becomes a new object;
    /// in vertex / face mode it is added to the object being edited.
    pub fn add_primitive(&mut self, name: &str, prim: Mesh) {
        self.push_undo();
        if self.sel_mode == SelMode::Object || self.objects.is_empty() {
            let name = self.unique_name(name);
            self.objects.push(Object::new(name, prim));
            self.active = self.objects.len() - 1;
            self.sel_objects = [self.active].into();
            self.clear_selection();
            self.sel_mode = SelMode::Object;
            self.status = "Added object. Drag an arrow or press G to move it.".into();
        } else {
            let new_faces = self.mesh_mut().append(&prim);
            self.sel_mode = SelMode::Face;
            self.sel_verts.clear();
            self.sel_faces = new_faces;
            self.status = "Added to the current object (new faces selected).".into();
        }
    }

    pub fn delete_selection(&mut self) {
        match self.sel_mode {
            SelMode::Object => {
                if self.sel_objects.is_empty() {
                    return;
                }
                self.push_undo();
                for &i in self.sel_objects.iter().rev() {
                    self.objects.remove(i);
                }
                self.sel_objects.clear();
                self.active = self.active.min(self.objects.len().saturating_sub(1));
            }
            SelMode::Face if !self.sel_faces.is_empty() => {
                self.push_undo();
                let faces = self.sel_faces.clone();
                self.mesh_mut().delete_faces(&faces);
            }
            SelMode::Vertex if !self.sel_verts.is_empty() => {
                self.push_undo();
                let verts = self.sel_verts.clone();
                self.mesh_mut().delete_verts(&verts);
            }
            SelMode::Edge if !self.sel_edges.is_empty() => {
                // Deleting an edge takes the faces on either side of it with it.
                let faces: BTreeSet<usize> = (0..self.mesh().faces.len())
                    .filter(|&f| self.face_edges(f).iter().any(|e| self.sel_edges.contains(e)))
                    .collect();
                self.push_undo();
                self.mesh_mut().delete_faces(&faces);
            }
            _ => return,
        }
        self.clear_selection();
    }

    /// Returns true when something was duplicated (the caller then starts a move).
    pub fn duplicate_selection(&mut self) -> bool {
        if self.sel_mode == SelMode::Object {
            if self.sel_objects.is_empty() {
                return false;
            }
            self.push_undo();
            let first_new = self.objects.len();
            for i in self.sel_objects.clone() {
                let src = self.objects[i].clone();
                let name = self.unique_name(&src.name);
                self.objects.push(Object { name, ..src });
            }
            self.sel_objects = (first_new..self.objects.len()).collect();
            self.active = self.objects.len() - 1;
            return true;
        }
        let faces = self.active_faces();
        if faces.is_empty() {
            self.status = "Duplicate needs at least one whole face selected.".into();
            return false;
        }
        self.push_undo();
        let new_faces = self.mesh_mut().duplicate_faces(&faces);
        self.select_faces(new_faces);
        true
    }

    pub fn tex_size(&self) -> Vec2 {
        match &self.texture {
            Some(t) => Vec2::new(t.size[0] as f32, t.size[1] as f32),
            None => Vec2::splat(64.0),
        }
    }

    pub fn uv_op(&mut self, f: impl FnOnce(&mut Mesh, &BTreeSet<usize>)) {
        let faces = self.active_faces();
        if faces.is_empty() {
            self.status = "Select one or more faces first.".into();
            return;
        }
        self.push_undo();
        f(self.mesh_mut(), &faces);
    }

    // ---------------------------------------------------------------- files

    pub fn load_texture(&mut self, ctx: &egui::Context, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                self.set_texture(ctx, path, bytes);
            }
            Err(e) => self.status = format!("Could not load {}: {e}", path.display()),
        }
    }

    /// Use an image file's bytes as the texture. `path` is where it lives (or would live) on disk.
    /// Returns false when the bytes are not an image this build can decode.
    fn set_texture(&mut self, ctx: &egui::Context, path: &Path, file_bytes: Vec<u8>) -> bool {
        match image::load_from_memory(&file_bytes) {
            Ok(img) => {
                let img = img.to_rgba8();
                let size = [img.width(), img.height()];
                let color = egui::ColorImage::from_rgba_unmultiplied([size[0] as usize, size[1] as usize], img.as_raw());
                let handle = ctx.load_texture("sheet", color, egui::TextureOptions::NEAREST);
                self.tex_counter += 1;
                self.texture = Some(Texture {
                    path: path.to_path_buf(),
                    size,
                    rgba: Arc::new(img.into_raw()),
                    file_bytes,
                    handle,
                    version: self.tex_counter,
                    mtime: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
                    dirty: false,
                    on_disk: path.is_file(),
                });
                self.status = format!("Texture: {} ({}x{})", path.display(), size[0], size[1]);
                true
            }
            Err(e) => {
                self.status = format!("Could not load {}: {e}", path.display());
                false
            }
        }
    }

    /// The texture's pixels were edited: refresh both the 3D view's and the 2D panels' copies.
    pub fn texture_changed(&mut self) {
        self.tex_counter += 1;
        if let Some(t) = &mut self.texture {
            t.version = self.tex_counter;
            t.dirty = true;
            let color = egui::ColorImage::from_rgba_unmultiplied([t.size[0] as usize, t.size[1] as usize], &t.rgba);
            t.handle.set(color, egui::TextureOptions::NEAREST);
        }
    }

    /// Start a blank image of the given size.
    pub fn new_canvas(&mut self, ctx: &egui::Context, width: u32, height: u32, fill: [u8; 4]) {
        let (width, height) = (width.clamp(1, 4096), height.clamp(1, 4096));
        let rgba: Vec<u8> = fill.iter().copied().cycle().take((width * height * 4) as usize).collect();
        let color = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
        let handle = ctx.load_texture("sheet", color, egui::TextureOptions::NEAREST);
        self.tex_counter += 1;
        self.texture = Some(Texture {
            path: PathBuf::from("untitled.png"),
            size: [width, height],
            rgba: Arc::new(rgba),
            file_bytes: Vec::new(),
            handle,
            version: self.tex_counter,
            mtime: None,
            dirty: true,
            on_disk: false,
        });
        // Old pixel undo steps belong to the previous image.
        self.undo.retain(|s| matches!(s, UndoStep::Scene(_)));
        self.redo.clear();
        self.uv.fit_view();
        self.status = format!("New {width}x{height} canvas (not saved yet)");
    }

    fn encode_png(t: &Texture) -> Result<Vec<u8>, String> {
        let img = image::RgbaImage::from_raw(t.size[0], t.size[1], t.rgba.to_vec()).ok_or("pixel buffer has the wrong size")?;
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok(out.into_inner())
    }

    /// Write the texture to `path` as a PNG and make that its file from now on.
    pub fn save_texture_to(&mut self, path: &Path) {
        let Some(t) = &mut self.texture else { return };
        let path = path.with_extension("png");
        let result = Self::encode_png(t).and_then(|bytes| std::fs::write(&path, &bytes).map(|_| bytes).map_err(|e| e.to_string()));
        match result {
            Ok(bytes) => {
                t.file_bytes = bytes;
                t.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                t.dirty = false;
                t.on_disk = true;
                t.path = path.clone();
                self.status = format!("Saved image {}", path.display());
            }
            Err(e) => self.status = format!("Could not save {}: {e}", path.display()),
        }
    }

    /// Save the texture where it came from, or ask for a place if it has none (or is not a PNG).
    pub fn save_texture(&mut self) {
        let Some(t) = &self.texture else { return };
        let is_png = t.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
        if t.on_disk && is_png {
            let path = t.path.clone();
            self.save_texture_to(&path);
        } else {
            self.save_texture_as();
        }
    }

    pub fn save_texture_as(&mut self) {
        let Some(t) = &self.texture else { return };
        let name = t.path.with_extension("png").file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "untitled.png".into());
        if let Some(p) = rfd::FileDialog::new().add_filter("PNG image", &["png"]).set_file_name(name).save_file() {
            self.save_texture_to(&p);
        }
    }

    /// Add the meshes of an FBX file to the scene as new objects.
    pub fn import_fbx(&mut self, ctx: &egui::Context, path: &Path) {
        let imported = match pixelmesh::fbx_import::import_file(path) {
            Ok(i) => i,
            Err(e) => {
                self.status = format!("Could not import {}: {e}", path.display());
                return;
            }
        };
        self.push_undo();
        let first_new = self.objects.len();
        for obj in imported.objects {
            let name = self.unique_name(&obj.name);
            self.objects.push(Object { origin: obj.origin, rotation: obj.rotation, ..Object::new(name, obj.mesh) });
        }
        let count = self.objects.len() - first_new;
        self.sel_mode = SelMode::Object;
        self.sel_objects = (first_new..self.objects.len()).collect();
        self.active = self.objects.len() - 1;
        self.clear_selection();
        self.frame_all();

        // Pick up the file's texture, unless one is already loaded.
        let mut notes = Vec::new();
        if let Some(tex) = imported.texture {
            let name = tex.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if self.texture.is_some() {
                notes.push(format!("kept the current texture (file uses {name})"));
            } else {
                let loaded = match tex.embedded {
                    Some(bytes) => self.set_texture(ctx, &tex.path, bytes),
                    None => match std::fs::read(&tex.path) {
                        Ok(bytes) => self.set_texture(ctx, &tex.path, bytes),
                        Err(_) => false,
                    },
                };
                notes.push(if loaded { format!("texture {name}") } else { format!("texture {name} not found or unreadable") });
            }
        }
        if imported.faces_without_uvs > 0 {
            notes.push(format!("{} faces had no UVs", imported.faces_without_uvs));
        }
        let notes = if notes.is_empty() { String::new() } else { format!(" ({})", notes.join("; ")) };
        self.status = format!("Imported {count} object(s) from {}{notes}", path.display());
    }

    /// Reload the texture when the file changes on disk, so edits in a paint program show up live.
    fn poll_texture(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        ctx.request_repaint_after(std::time::Duration::from_millis(700));
        if now - self.last_mtime_check < 0.5 {
            return;
        }
        self.last_mtime_check = now;
        if let Some(t) = &self.texture {
            // Never reload over unsaved paint work.
            if t.dirty || !t.on_disk {
                return;
            }
            let mtime = std::fs::metadata(&t.path).and_then(|m| m.modified()).ok();
            if mtime.is_some() && mtime != t.mtime {
                let path = t.path.clone();
                self.load_texture(ctx, &path);
            }
        }
    }

    fn save_project(&mut self, path: &Path) {
        let file = ProjectFile {
            version: 2,
            objects: self.objects.clone(),
            mesh: None,
            texture: self.texture.as_ref().map(|t| t.path.to_string_lossy().into_owned()),
            px_per_unit: self.px_per_unit,
        };
        let result = serde_json::to_vec(&file).map_err(|e| e.to_string())
            .and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.project_path = Some(path.to_path_buf());
                self.status = format!("Saved {}", path.display());
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    pub fn open_project(&mut self, ctx: &egui::Context, path: &Path) {
        let result = std::fs::read(path).map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<ProjectFile>(&bytes).map_err(|e| e.to_string()));
        match result {
            Ok(mut file) => {
                if let Some(mesh) = file.mesh.take() {
                    file.objects.push(Object::new("Object", mesh));
                }
                let valid = file.objects.iter().all(|o| {
                    o.mesh.faces.iter().all(|f| {
                        f.verts.len() >= 3 && f.verts.len() == f.uvs.len()
                            && f.verts.iter().all(|&v| (v as usize) < o.mesh.positions.len())
                    })
                });
                if !valid {
                    self.status = format!("{} is not a valid project file", path.display());
                    return;
                }
                self.objects = file.objects;
                self.active = 0;
                self.sel_objects = if self.objects.is_empty() { BTreeSet::new() } else { [0].into() };
                self.sel_mode = SelMode::Object;
                self.px_per_unit = file.px_per_unit;
                self.clear_selection();
                self.undo.clear();
                self.redo.clear();
                self.texture = None;
                self.project_path = Some(path.to_path_buf());
                self.status = format!("Opened {}", path.display());
                if let Some(tex) = file.texture {
                    self.load_texture(ctx, Path::new(&tex));
                }
            }
            Err(e) => self.status = format!("Open failed: {e}"),
        }
    }

    fn export_fbx(&mut self, path: &Path) {
        // Painted but unsaved pixels still have to end up in the exported texture.
        if let Some(t) = &mut self.texture {
            if t.dirty || t.file_bytes.is_empty() {
                match Self::encode_png(t) {
                    Ok(bytes) => t.file_bytes = bytes,
                    Err(e) => {
                        self.status = format!("Export failed: could not encode the texture ({e})");
                        return;
                    }
                }
            }
        }
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Mesh").to_string();
        let tex_name = self.texture.as_ref()
            .and_then(|t| t.path.file_name().and_then(|n| n.to_str()).map(str::to_string));
        // Objects without faces would only produce empty nodes.
        let objects: Vec<fbx::FbxObject> = self.objects.iter()
            .filter(|o| !o.mesh.faces.is_empty())
            .map(|o| fbx::FbxObject { name: &o.name, mesh: &o.mesh, origin: o.origin, rotation: o.rotation })
            .collect();
        if objects.is_empty() {
            self.status = "Nothing to export: the scene has no faces.".into();
            return;
        }
        let count = objects.len();
        let opt = fbx::FbxOptions {
            material_name: &name,
            scale: 1.0,
            texture: match (&self.texture, &tex_name) {
                (Some(t), Some(n)) => Some(fbx::FbxTexture {
                    file_name: n,
                    embed: self.embed_texture.then_some(t.file_bytes.as_slice()),
                }),
                _ => None,
            },
        };
        match fbx::export_file(path, &objects, &opt) {
            Ok(()) => {
                let mut msg = format!("Exported {count} object(s) to {}", path.display());
                // The FBX refers to the texture by file name, so make sure a copy sits next to it.
                if let (Some(t), Some(n), Some(dir)) = (&self.texture, &tex_name, path.parent()) {
                    let dest = dir.join(n);
                    if !self.embed_texture && !dest.exists() {
                        match std::fs::write(&dest, &t.file_bytes) {
                            Ok(()) => msg += &format!(" (+ wrote {n})"),
                            Err(e) => msg += &format!(" (could not write {n}: {e})"),
                        }
                    } else if !self.embed_texture && t.dirty {
                        msg += &format!(" (note: {n} next to it is older than your unsaved paint changes; save the image)");
                    }
                }
                self.status = msg;
            }
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    // ------------------------------------------------------------------- ui

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    self.push_undo();
                    self.objects.clear();
                    self.sel_objects.clear();
                    self.active = 0;
                    self.sel_mode = SelMode::Object;
                    self.clear_selection();
                    self.project_path = None;
                }
                if ui.button("Open…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("PixelMesh project", &["pmesh"]).pick_file() {
                        self.open_project(&ctx, &p);
                    }
                }
                if ui.button("Save").clicked() {
                    match self.project_path.clone() {
                        Some(p) => self.save_project(&p),
                        None => self.save_as(),
                    }
                }
                if ui.button("Save as…").clicked() {
                    self.save_as();
                }
                ui.separator();
                if ui.button("New image…").clicked() {
                    self.show_uv = true;
                    self.left_tab = LeftTab::Paint;
                    self.paint.open_new_dialog(self.texture.as_ref().map(|t| t.size));
                }
                if ui.button("Load image…").clicked() {
                    self.pick_texture(&ctx);
                }
                if ui.add_enabled(self.texture.is_some(), egui::Button::new("Save image   Ctrl+S")).clicked() {
                    self.save_texture();
                }
                ui.separator();
                if ui.button("Import FBX…").on_hover_text("Add the meshes of an FBX file to this scene").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("FBX", &["fbx"]).pick_file() {
                        self.import_fbx(&ctx, &p);
                    }
                }
                if ui.button("Export FBX…").clicked() {
                    self.pick_export();
                }
                ui.checkbox(&mut self.embed_texture, "Embed texture in FBX");
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo   Ctrl+Z")).clicked() {
                    self.undo();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo   Ctrl+Shift+Z")).clicked() {
                    self.redo();
                }
            });
            ui.menu_button("View", |ui| {
                for (label, yaw, pitch) in [("Front   1", 0.0, 0.0), ("Right   3", 1.5708, 0.0), ("Top   7", 0.0, 1.5707)] {
                    if ui.button(label).clicked() {
                        self.camera.yaw = yaw;
                        self.camera.pitch = pitch;
                    }
                }
                ui.checkbox(&mut self.camera.ortho, "Orthographic   5");
                ui.separator();
                ui.checkbox(&mut self.show_uv, "UV / Paint panel   U");
                ui.checkbox(&mut self.show_gizmo, "Move arrows");
                if ui.button("Frame all   Home").clicked() {
                    self.frame_all();
                }
            });
            ui.separator();
            ui.toggle_value(&mut self.show_uv, "UV / Paint panel").on_hover_text("Show or hide the UV editor and paint panel (U)");
        });
    }

    fn save_as(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("PixelMesh project", &["pmesh"]).set_file_name("model.pmesh").save_file() {
            self.save_project(&p);
        }
    }

    pub fn pick_texture(&mut self, ctx: &egui::Context) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "bmp", "tga"]).pick_file() {
            self.load_texture(ctx, &p);
        }
    }

    pub fn pick_export(&mut self) {
        let default = self.project_path.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()).unwrap_or("model").to_string();
        if let Some(p) = rfd::FileDialog::new().add_filter("FBX", &["fbx"]).set_file_name(format!("{default}.fbx")).save_file() {
            self.export_fbx(&p);
        }
    }

    pub fn frame_all(&mut self) {
        let pts: Vec<Vec3> = self.objects.iter().filter(|o| o.visible).flat_map(|o| o.mesh.positions.iter().copied()).collect();
        if pts.is_empty() {
            self.camera = Camera::default();
            return;
        }
        let lo = pts.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
        let hi = pts.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
        self.camera.target = (lo + hi) * 0.5;
        self.camera.distance = ((hi - lo).length() * 1.6).max(1.0);
    }

    /// X/Y/Z fields for the centre of the selection; editing them moves it.
    pub fn position_fields(&mut self, ui: &mut egui::Ui) {
        let object_mode = self.sel_mode == SelMode::Object;
        let targets = self.transform_targets();
        let objs = self.transform_objects();
        // What the numbers show: an object's origin, or the middle of the selected geometry.
        let centre = if objs.len() == 1 {
            self.objects[objs[0]].origin
        } else if !objs.is_empty() && self.origin_only {
            objs.iter().map(|&i| self.objects[i].origin).sum::<Vec3>() / objs.len() as f32
        } else if !targets.is_empty() {
            targets.iter().map(|&(o, v)| self.objects[o].mesh.positions[v as usize]).sum::<Vec3>() / targets.len() as f32
        } else {
            return;
        };
        let muted = egui::Color32::from_rgb(138, 144, 156);
        let mut edited = centre;
        ui.horizontal(|ui| {
            let label = if object_mode && self.origin_only { "Origin" } else { "Pos" };
            ui.label(egui::RichText::new(label).color(muted)).on_hover_text(if object_mode {
                "Position of the object's origin"
            } else {
                "Middle of the selection"
            });
            for (axis, label) in ["X ", "Y ", "Z "].into_iter().enumerate() {
                let r = ui.add(egui::DragValue::new(&mut edited[axis]).speed(0.01).prefix(label).max_decimals(3));
                if r.drag_started() || r.gained_focus() {
                    self.push_undo();
                }
            }
        });
        let delta = edited - centre;
        if delta != Vec3::ZERO {
            for (o, v) in targets {
                self.objects[o].mesh.positions[v as usize] += delta;
            }
            for &i in &objs {
                self.objects[i].origin += delta;
            }
        }

        // Rotation of the active object, about its origin.
        if !(object_mode && self.sel_objects.contains(&self.active) && self.active < self.objects.len()) {
            return;
        }
        let old = self.objects[self.active].rotation;
        let mut new = old;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Rot").color(muted)).on_hover_text("Rotation of the active object about its origin, in degrees (applied X, then Y, then Z)");
            for (axis, label) in ["X ", "Y ", "Z "].into_iter().enumerate() {
                let r = ui.add(egui::DragValue::new(&mut new[axis]).speed(1.0).prefix(label).suffix("°").max_decimals(2));
                if r.drag_started() || r.gained_focus() {
                    self.push_undo();
                }
            }
        });
        if new != old {
            let turn = pixelmesh::xform::euler_quat(new) * pixelmesh::xform::euler_quat(old).inverse();
            let origin_only = self.origin_only;
            let obj = &mut self.objects[self.active];
            // With "origin only" on, just the recorded orientation changes; the geometry stays put.
            if !origin_only {
                for p in &mut obj.mesh.positions {
                    *p = obj.origin + turn * (*p - obj.origin);
                }
            }
            obj.rotation = new;
        }
    }

    pub fn extrude(&mut self) {
        if self.sel_mode == SelMode::Object {
            self.status = "Extrude works on faces: press Tab or F, select faces, then E.".into();
            return;
        }
        let faces = self.active_faces();
        if faces.is_empty() {
            self.status = "Extrude needs at least one whole face selected.".into();
            return;
        }
        self.push_undo();
        let normal = self.mesh_mut().extrude_faces(&faces);
        self.select_faces(faces);
        // The undo step already exists, so the modal must not add another one.
        self.start_modal_no_undo(crate::viewport::ModalKind::Grab, Some(normal));
    }

    pub fn start_inset(&mut self) {
        if self.sel_mode == SelMode::Object {
            self.status = "Inset works on faces: press Tab or F, select faces, then I.".into();
            return;
        }
        let faces = self.active_faces();
        if faces.is_empty() {
            self.status = "Inset needs at least one whole face selected.".into();
            return;
        }
        self.push_undo();
        self.op = Some(crate::viewport::OpModal::new(crate::viewport::OpKind::Inset, self.mesh().clone(), faces, BTreeSet::new()));
    }

    pub fn start_bevel(&mut self) {
        let mesh = self.mesh();
        let edges: BTreeSet<(u32, u32)> = match self.sel_mode {
            SelMode::Object => {
                self.status = "Bevel works on edges: press Tab, select faces or vertices, then Ctrl+B.".into();
                return;
            }
            // Face mode: every edge of the selected faces.
            SelMode::Face => self.sel_faces.iter().flat_map(|&f| {
                let v = &mesh.faces[f].verts;
                (0..v.len()).map(|i| (v[i].min(v[(i + 1) % v.len()]), v[i].max(v[(i + 1) % v.len()]))).collect::<Vec<_>>()
            }).collect(),
            SelMode::Edge => self.sel_edges.clone(),
            // Vertex mode: every edge with both ends selected.
            SelMode::Vertex => mesh.edges().into_iter()
                .filter(|(a, b)| self.sel_verts.contains(a) && self.sel_verts.contains(b))
                .collect(),
        };
        if edges.is_empty() {
            self.status = "Bevel needs edges: select faces, or the vertices at both ends of an edge.".into();
            return;
        }
        self.push_undo();
        self.op = Some(crate::viewport::OpModal::new(crate::viewport::OpKind::Bevel, self.mesh().clone(), BTreeSet::new(), edges));
    }

    /// Join the selected faces into single polygons (one per connected group).
    pub fn merge_selected_faces(&mut self) {
        if self.sel_mode == SelMode::Object {
            self.status = "Merge faces works in an edit mode: press Tab, select two or more adjoining faces.".into();
            return;
        }
        let faces = self.active_faces();
        if faces.len() < 2 {
            self.status = "Merge faces needs two or more adjoining faces selected.".into();
            return;
        }
        self.push_undo();
        let (new, skipped) = self.mesh_mut().merge_faces(&faces);
        if new.is_empty() {
            self.revert_last();
            self.status = if skipped > 0 {
                "Could not merge: the selection's outline is not a single loop (it has a hole or only touches at a point).".into()
            } else {
                "Nothing to merge: the selected faces do not share an edge.".into()
            };
            return;
        }
        let count = new.len();
        self.select_faces(new);
        self.status = format!("Merged into {count} face(s){}", if skipped > 0 { format!(", {skipped} group(s) skipped") } else { String::new() });
    }

    /// Right-click menu of the 3D view.
    pub fn context_menu(&mut self, ui: &mut egui::Ui) {
        use crate::viewport::ModalKind;
        ui.set_min_width(170.0);
        if self.sel_mode == SelMode::Object {
            let n = self.sel_objects.len();
            if ui.add_enabled(n > 0, egui::Button::new("Edit faces   Tab")).clicked() {
                self.toggle_edit_mode();
            }
            ui.separator();
            if ui.add_enabled(n > 0, egui::Button::new("Duplicate   Shift+D")).clicked() && self.duplicate_selection() {
                self.start_modal_no_undo(ModalKind::Grab, None);
            }
            if ui.add_enabled(n >= 2, egui::Button::new("Merge objects   Ctrl+J")).clicked() {
                self.merge_selected();
            }
            ui.separator();
            if ui.add_enabled(n > 0, egui::Button::new(if n > 1 { "Delete objects   X" } else { "Delete object   X" })).clicked() {
                self.delete_selection();
            }
            return;
        }
        let faces = self.active_faces().len();
        let any = !self.active_verts().is_empty();
        let delete_label = match self.sel_mode {
            SelMode::Vertex => "Delete vertices   X",
            SelMode::Edge => "Delete edges   X",
            _ => "Delete faces   X",
        };
        if ui.add_enabled(any, egui::Button::new(delete_label)).clicked() {
            self.delete_selection();
        }
        if ui.add_enabled(faces >= 2, egui::Button::new("Merge faces   M")).clicked() {
            self.merge_selected_faces();
        }
        ui.separator();
        if ui.add_enabled(faces > 0, egui::Button::new("Extrude   E")).clicked() {
            self.extrude();
        }
        if ui.add_enabled(faces > 0, egui::Button::new("Inset   I")).clicked() {
            self.start_inset();
        }
        if ui.add_enabled(any, egui::Button::new("Bevel   Ctrl+B")).clicked() {
            self.start_bevel();
        }
        if ui.button("Loop cut   Ctrl+R").clicked() {
            self.start_loop_cut();
        }
        ui.separator();
        if ui.add_enabled(faces > 0, egui::Button::new("Duplicate   Shift+D")).clicked() && self.duplicate_selection() {
            self.start_modal_no_undo(ModalKind::Grab, None);
        }
        if ui.add_enabled(faces > 0, egui::Button::new("Flip normals")).clicked() {
            let f = self.active_faces();
            self.push_undo();
            self.mesh_mut().flip_faces(&f);
        }
        if ui.add_enabled(any, egui::Button::new("Select linked   L")).clicked() {
            self.select_linked();
        }
    }

    pub fn start_loop_cut(&mut self) {
        if self.mesh().faces.is_empty() {
            self.status = "Loop cut needs an object with faces.".into();
            return;
        }
        self.loop_cut = Some(crate::viewport::LoopCut::new());
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.busy() || self.paint.is_painting() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::S)) {
            // Save whatever has somewhere to go: the image, and the project if it has a file.
            if self.texture.as_ref().is_some_and(|t| t.dirty) {
                self.save_texture();
            }
            if let Some(p) = self.project_path.clone() {
                self.save_project(&p);
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::J)) {
            self.merge_selected();
        }
        let (undo, redo) = ctx.input_mut(|i| {
            let redo = i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Z)
                || i.consume_key(egui::Modifiers::COMMAND, Key::Y);
            (i.consume_key(egui::Modifiers::COMMAND, Key::Z), redo)
        });
        if redo {
            self.redo();
        } else if undo {
            self.undo();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // PIXELMESH_PROFILE=1 prints how long building each frame takes (and repaints continuously).
        let profile_start = std::time::Instant::now();
        let profiling = std::env::var_os("PIXELMESH_PROFILE").is_some();
        if profiling {
            ctx.request_repaint();
        }
        let _report = Defer(Some(move || {
            if profiling {
                use std::sync::atomic::{AtomicU64, Ordering};
                static FRAMES: AtomicU64 = AtomicU64::new(0);
                static MICROS: AtomicU64 = AtomicU64::new(0);
                let total = MICROS.fetch_add(profile_start.elapsed().as_micros() as u64, Ordering::Relaxed);
                let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 20 == 0 {
                    eprintln!("ui: {:.2} ms/frame over the last 20 frames", (total + profile_start.elapsed().as_micros() as u64) as f64 / 20_000.0);
                    MICROS.store(0, Ordering::Relaxed);
                }
            }
        }));
        self.poll_texture(&ctx);
        self.shortcuts(&ctx);

        // Menus and tools are locked while a move / rotate / scale is in progress.
        let idle = !self.busy();
        egui::Panel::top("menu").show(ui, |ui| ui.add_enabled_ui(idle, |ui| self.menu_bar(ui)));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
            });
        });
        egui::Panel::right("tools").resizable(false).exact_size(272.0).show(ui, |ui| ui.add_enabled_ui(idle, |ui| self.tool_panel(ui)));
        if self.show_uv {
            egui::Panel::left("uv").resizable(true).default_size(440.0).min_size(200.0).show(ui, |ui| {
                ui.add_enabled_ui(idle, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.left_tab, LeftTab::Uv, "UV editor");
                        ui.selectable_value(&mut self.left_tab, LeftTab::Paint, "Paint");
                        if let Some(t) = &self.texture {
                            let name = t.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.weak(format!("{name}{}  {}x{}", if t.dirty { " *" } else { "" }, t.size[0], t.size[1]));
                        }
                    });
                    ui.separator();
                    match self.left_tab {
                        LeftTab::Uv => self.uv_editor(ui),
                        LeftTab::Paint => self.paint_editor(ui),
                    }
                })
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));
    }

    /// Tab switches object / edit mode. It has to be removed from the input before egui
    /// sees it, otherwise egui also moves keyboard focus to the next widget.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let is_tab = |e: &egui::Event| matches!(e, egui::Event::Key { key: Key::Tab, .. });
        let pressed = raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Tab, pressed: true, repeat: false, .. }));
        raw.events.retain(|e| !is_tab(e));
        if pressed && !self.busy() && !ctx.egui_wants_keyboard_input() {
            self.toggle_edit_mode();
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let (Some(gl), Some(r)) = (gl, &self.renderer) {
            r.lock().unwrap().destroy(gl);
        }
    }
}

/// Runs a closure when dropped (end of scope).
struct Defer<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for Defer<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}
