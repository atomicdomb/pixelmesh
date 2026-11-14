//! Application state, menus, tool panel and file handling.

use crate::camera::Camera;
use crate::render::Renderer;
use crate::uv_editor::UvEditor;
use crate::viewport::Modal;
use eframe::egui::{self, Key};
use glam::{Vec2, Vec3};
use pixelmesh::fbx;
use pixelmesh::mesh::{Mesh, UvOp};
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
    Face,
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
}

fn yes() -> bool {
    true
}

pub struct Snapshot {
    objects: Vec<Object>,
    active: usize,
    sel_objects: BTreeSet<usize>,
    sel_verts: BTreeSet<u32>,
    sel_faces: BTreeSet<usize>,
}

pub struct Texture {
    pub path: PathBuf,
    pub size: [u32; 2],
    pub rgba: Arc<Vec<u8>>,
    pub file_bytes: Vec<u8>,
    pub handle: egui::TextureHandle,
    pub version: u64,
    mtime: Option<SystemTime>,
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
    empty: Mesh,
    pub sel_mode: SelMode,
    last_edit_mode: SelMode,
    pub sel_verts: BTreeSet<u32>,
    pub sel_faces: BTreeSet<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,

    pub camera: Camera,
    pub renderer: Option<Arc<Mutex<Renderer>>>,
    pub modal: Option<Modal>,
    pub viewport_rect: egui::Rect,
    pub uv: UvEditor,

    pub texture: Option<Texture>,
    tex_counter: u64,
    last_mtime_check: f64,

    pub snap_on: bool,
    pub grid_snap: f32,
    pub show_uv: bool,
    pub show_gizmo: bool,
    pub px_per_unit: f32,
    segments: u32,
    embed_texture: bool,
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
        let mut app = App {
            objects: vec![Object { name: "Cube".into(), mesh: Mesh::cube(), visible: true }],
            active: 0,
            sel_objects: [0].into(),
            select_tool: SelectTool::Click,
            circle_radius: 40.0,
            only_visible: true,
            region: None,
            empty: Mesh::default(),
            sel_mode: SelMode::Object,
            last_edit_mode: SelMode::Face,
            sel_verts: BTreeSet::new(),
            sel_faces: BTreeSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            camera: Camera::default(),
            renderer,
            modal: None,
            viewport_rect: egui::Rect::NOTHING,
            uv: UvEditor::default(),
            texture: None,
            tex_counter: 0,
            last_mtime_check: 0.0,
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
                Some("png") => app.load_texture(&cc.egui_ctx, p),
                Some("pmesh") => app.open_project(&cc.egui_ctx, p),
                _ => {}
            }
        }
        app
    }

    // --------------------------------------------------------------- objects

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
            SelMode::Object => self.sel_objects.iter()
                .filter(|&&i| self.objects.get(i).is_some_and(|o| o.visible))
                .flat_map(|&i| (0..self.objects[i].mesh.positions.len() as u32).map(move |v| (i, v)))
                .collect(),
            _ => self.active_verts().into_iter().map(|v| (self.active, v)).collect(),
        }
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

    fn unique_name(&self, base: &str) -> String {
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
            SelMode::Face => self.mesh().verts_of_faces(&self.sel_faces),
        }
    }

    /// Faces the face / UV tools act on. In vertex mode: faces whose corners are all selected.
    pub fn active_faces(&self) -> BTreeSet<usize> {
        match self.sel_mode {
            SelMode::Object => (0..self.mesh().faces.len()).collect(),
            SelMode::Face => self.sel_faces.clone(),
            SelMode::Vertex => (0..self.mesh().faces.len())
                .filter(|&f| self.mesh().faces[f].verts.iter().all(|v| self.sel_verts.contains(v)))
                .collect(),
        }
    }

    pub fn set_mode(&mut self, mode: SelMode) {
        // Carry the selection over so switching between vertex and face mode does not lose it.
        match (self.sel_mode, mode) {
            (SelMode::Face, SelMode::Vertex) => self.sel_verts = self.mesh().verts_of_faces(&self.sel_faces),
            (SelMode::Vertex, SelMode::Face) => self.sel_faces = self.active_faces(),
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
        self.sel_faces = (0..self.mesh().faces.len()).collect();
    }

    pub fn clear_selection(&mut self) {
        self.sel_verts.clear();
        self.sel_faces.clear();
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
            SelMode::Vertex => (0..mesh.faces.len())
                .filter(|&f| mesh.faces[f].verts.iter().any(|v| self.sel_verts.contains(v)))
                .collect(),
        };
        let faces = mesh.linked_faces(&seed);
        self.sel_verts = mesh.verts_of_faces(&faces);
        self.sel_faces = faces;
    }

    // ----------------------------------------------------------------- undo

    fn snapshot(&self) -> Snapshot {
        Snapshot { objects: self.objects.clone(), active: self.active, sel_objects: self.sel_objects.clone(), sel_verts: self.sel_verts.clone(), sel_faces: self.sel_faces.clone() }
    }

    fn restore(&mut self, s: Snapshot) {
        self.objects = s.objects;
        self.active = s.active;
        self.sel_objects = s.sel_objects;
        self.sel_verts = s.sel_verts;
        self.sel_faces = s.sel_faces;
    }

    /// Call before every change to the mesh.
    pub fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(s);
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(s);
        }
    }

    /// Throw away the most recent undo step and go back to it (used when a modal tool is cancelled).
    pub fn revert_last(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.restore(s);
        }
    }

    // ------------------------------------------------------------- mesh ops

    /// In object mode (or an empty scene) a primitive becomes a new object;
    /// in vertex / face mode it is added to the object being edited.
    fn add_primitive(&mut self, name: &str, prim: Mesh) {
        self.push_undo();
        if self.sel_mode == SelMode::Object || self.objects.is_empty() {
            let name = self.unique_name(name);
            self.objects.push(Object { name, mesh: prim, visible: true });
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
        self.sel_verts = self.mesh().verts_of_faces(&new_faces);
        self.sel_faces = new_faces;
        true
    }

    pub fn tex_size(&self) -> Vec2 {
        match &self.texture {
            Some(t) => Vec2::new(t.size[0] as f32, t.size[1] as f32),
            None => Vec2::splat(64.0),
        }
    }

    fn uv_op(&mut self, f: impl FnOnce(&mut Mesh, &BTreeSet<usize>)) {
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
        let result = std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| {
            let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
            Ok((bytes, img))
        });
        match result {
            Ok((file_bytes, img)) => {
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
                });
                self.status = format!("Texture: {} ({}x{})", path.display(), size[0], size[1]);
            }
            Err(e) => self.status = format!("Could not load {}: {e}", path.display()),
        }
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
                    file.objects.push(Object { name: "Object".into(), mesh, visible: true });
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
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Mesh").to_string();
        let tex_name = self.texture.as_ref()
            .and_then(|t| t.path.file_name().and_then(|n| n.to_str()).map(str::to_string));
        // Objects without faces would only produce empty nodes.
        let objects: Vec<fbx::FbxObject> = self.objects.iter()
            .filter(|o| !o.mesh.faces.is_empty())
            .map(|o| fbx::FbxObject { name: &o.name, mesh: &o.mesh })
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
                            Ok(()) => msg += &format!(" (+ copied {n})"),
                            Err(e) => msg += &format!(" (could not copy {n}: {e})"),
                        }
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
                if ui.button("Load texture…").clicked() {
                    self.pick_texture(&ctx);
                }
                ui.separator();
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
                ui.checkbox(&mut self.show_uv, "UV editor   U");
                ui.checkbox(&mut self.show_gizmo, "Move arrows");
                if ui.button("Frame all   Home").clicked() {
                    self.frame_all();
                }
            });
            ui.separator();
            ui.toggle_value(&mut self.show_uv, "UV editor").on_hover_text("Show or hide the UV editor (U)");
        });
    }

    fn save_as(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("PixelMesh project", &["pmesh"]).set_file_name("model.pmesh").save_file() {
            self.save_project(&p);
        }
    }

    fn pick_texture(&mut self, ctx: &egui::Context) {
        if let Some(p) = rfd::FileDialog::new().add_filter("PNG image", &["png"]).pick_file() {
            self.load_texture(ctx, &p);
        }
    }

    fn pick_export(&mut self) {
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

    /// The scene outliner: every object with a visibility box; click a name to select it.
    fn object_list(&mut self, ui: &mut egui::Ui) {
        ui.strong("Objects");
        if self.objects.is_empty() {
            ui.weak("Empty scene. Add a mesh below.");
            return;
        }
        let object_mode = self.sel_mode == SelMode::Object;
        let mut clicked = None;
        for (i, obj) in self.objects.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut obj.visible, "").on_hover_text("Visible");
                let mut label = egui::RichText::new(format!("{}  ({} faces)", obj.name, obj.mesh.faces.len()));
                if i == self.active {
                    label = label.strong();
                }
                // In edit modes only the object being edited is highlighted.
                let selected = if object_mode { self.sel_objects.contains(&i) } else { i == self.active };
                let r = ui.selectable_label(selected, label);
                if r.clicked() {
                    clicked = Some((i, ui.input(|inp| inp.modifiers.shift || inp.modifiers.command)));
                }
            });
        }
        if let Some((i, toggle)) = clicked {
            self.set_mode(SelMode::Object);
            self.select_object(i, toggle);
        }
        ui.weak("Shift+click to select several");
        let active = self.active;
        if let Some(obj) = self.objects.get_mut(active) {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut obj.name).desired_width(150.0));
            });
        }
        ui.horizontal(|ui| {
            if ui.button("Duplicate").clicked() {
                self.set_mode(SelMode::Object);
                if self.duplicate_selection() {
                    self.start_modal_no_undo(crate::viewport::ModalKind::Grab, None);
                }
            }
            if ui.button("Delete").clicked() {
                self.set_mode(SelMode::Object);
                self.delete_selection();
            }
            let can_merge = object_mode && self.sel_objects.len() >= 2;
            if ui.add_enabled(can_merge, egui::Button::new("Merge"))
                .on_hover_text("Join the selected objects into the active (bold) one  ·  Ctrl+J")
                .on_disabled_hover_text("Select two or more objects first (Shift+click)")
                .clicked()
            {
                self.merge_selected();
            }
        });
    }

    fn tool_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);
            self.object_list(ui);

            ui.separator();
            ui.strong("Mode");
            ui.horizontal(|ui| {
                let mut mode = self.sel_mode;
                ui.selectable_value(&mut mode, SelMode::Object, "Object (O)");
                ui.selectable_value(&mut mode, SelMode::Vertex, "Vertex (V)");
                ui.selectable_value(&mut mode, SelMode::Face, "Face (F)");
                if mode != self.sel_mode {
                    self.set_mode(mode);
                }
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.select_tool, SelectTool::Click, "Click (Q)");
                ui.selectable_value(&mut self.select_tool, SelectTool::Box, "Box (B)");
                ui.selectable_value(&mut self.select_tool, SelectTool::Circle, "Circle (C)");
            });
            if self.select_tool == SelectTool::Circle {
                ui.horizontal(|ui| {
                    ui.label("Circle radius");
                    ui.add(egui::DragValue::new(&mut self.circle_radius).range(5.0..=300.0).suffix(" px"))
                        .on_hover_text("Also [ and ] in the viewport");
                });
            }
            if self.select_tool != SelectTool::Click {
                ui.checkbox(&mut self.only_visible, "Only visible")
                    .on_hover_text("Box / circle skip vertices and faces hidden behind geometry. Untick to select straight through.");
                ui.weak("Drag selects · Shift adds · Ctrl removes");
            }
            ui.horizontal(|ui| {
                if ui.button("All (A)").clicked() {
                    self.select_all();
                }
                if ui.button("None").clicked() {
                    self.deselect_all();
                }
                if ui.button("Linked (L)").clicked() {
                    self.select_linked();
                }
            });

            ui.separator();
            ui.strong("Add mesh");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Plane").clicked() {
                    self.add_primitive("Plane", Mesh::plane());
                }
                if ui.button("Cube").clicked() {
                    self.add_primitive("Cube", Mesh::cube());
                }
                if ui.button("Cylinder").clicked() {
                    self.add_primitive("Cylinder", Mesh::cylinder(self.segments, 0.5, 0.5, 1.0));
                }
                if ui.button("Cone").clicked() {
                    self.add_primitive("Cone", Mesh::cylinder(self.segments, 0.5, 0.0, 1.0));
                }
                if ui.button("Sphere").clicked() {
                    self.add_primitive("Sphere", Mesh::sphere(self.segments, (self.segments / 2).max(3), 0.5));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Segments");
                ui.add(egui::DragValue::new(&mut self.segments).range(3..=64));
            });

            ui.separator();
            ui.strong("Transform");
            ui.horizontal(|ui| {
                if ui.button("Move (G)").clicked() {
                    self.start_modal(crate::viewport::ModalKind::Grab, None);
                }
                if ui.button("Rotate (R)").clicked() {
                    self.start_modal(crate::viewport::ModalKind::Rotate, None);
                }
                if ui.button("Scale (S)").clicked() {
                    self.start_modal(crate::viewport::ModalKind::Scale, None);
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.snap_on, "Snap every")
                    .on_hover_text("Moves jump in steps of this distance. Holding Ctrl flips this setting while you drag.");
                ui.add(egui::DragValue::new(&mut self.grid_snap).speed(0.0125).range(0.001..=10.0).max_decimals(4));
            });
            self.position_fields(ui);

            ui.separator();
            ui.strong("Mesh");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Extrude (E)").clicked() {
                    self.extrude();
                }
                if ui.button("Duplicate (Shift+D)").clicked() && self.duplicate_selection() {
                    self.start_modal(crate::viewport::ModalKind::Grab, None);
                }
                if ui.button("Delete (X)").clicked() {
                    self.delete_selection();
                }
                if ui.button("Flip normals").clicked() {
                    let faces = self.active_faces();
                    if !faces.is_empty() {
                        self.push_undo();
                        self.mesh_mut().flip_faces(&faces);
                    }
                }
                if ui.button("Merge overlapping verts").clicked() {
                    self.push_undo();
                    let n = self.mesh_mut().merge_by_distance(1e-4);
                    self.clear_selection();
                    self.status = format!("Merged {n} vertices");
                }
            });

            ui.separator();
            ui.strong("UV mapping");
            let tex = self.tex_size();
            ui.horizontal_wrapped(|ui| {
                if ui.button("Fit each face").on_hover_text("Stretch every selected face over the whole texture").clicked() {
                    self.uv_op(|m, f| m.uv_fit_each(f));
                }
                if ui.button("Box project").on_hover_text("Project along the nearest axis at the texel density below").clicked() {
                    let ppu = self.px_per_unit;
                    self.uv_op(|m, f| m.uv_box_project(f, ppu, tex));
                }
                if ui.button("Project from view").clicked() {
                    let view = self.camera.view();
                    self.uv_op(|m, f| m.uv_project_view(f, view));
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Rotate 90°").clicked() {
                    self.uv_op(|m, f| m.uv_transform(f, UvOp::Rotate90));
                }
                if ui.button("Flip U").clicked() {
                    self.uv_op(|m, f| m.uv_transform(f, UvOp::FlipU));
                }
                if ui.button("Flip V").clicked() {
                    self.uv_op(|m, f| m.uv_transform(f, UvOp::FlipV));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Pixels per unit");
                ui.add(egui::DragValue::new(&mut self.px_per_unit).speed(0.25).range(1.0..=512.0));
            });
            ui.checkbox(&mut self.uv.snap, "Snap UVs to pixels");
            ui.checkbox(&mut self.uv.show_all, "Show unselected faces");

            ui.separator();
            ui.strong("Texture");
            ui.horizontal(|ui| {
                if ui.button("Load PNG…").clicked() {
                    self.pick_texture(&ctx);
                }
                if self.texture.is_some() && ui.button("Remove").clicked() {
                    self.texture = None;
                }
            });
            match &self.texture {
                Some(t) => {
                    let name = t.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(format!("{name}  {}×{}", t.size[0], t.size[1]));
                    ui.weak("Reloads automatically when the file changes.");
                }
                None => {
                    ui.weak("No texture (64×64 grid assumed).");
                }
            }

            ui.separator();
            if ui.button("Export FBX…").clicked() {
                self.pick_export();
            }
            ui.add_space(6.0);
            ui.weak(format!("{} objects · {} verts · {} faces", self.objects.len(),
                self.objects.iter().map(|o| o.mesh.positions.len()).sum::<usize>(),
                self.objects.iter().map(|o| o.mesh.faces.len()).sum::<usize>()));
        });
    }

    /// X/Y/Z fields for the centre of the selection; editing them moves it.
    fn position_fields(&mut self, ui: &mut egui::Ui) {
        let targets = self.transform_targets();
        if targets.is_empty() {
            return;
        }
        let centre = targets.iter().map(|&(o, v)| self.objects[o].mesh.positions[v as usize]).sum::<Vec3>() / targets.len() as f32;
        let mut edited = centre;
        ui.horizontal(|ui| {
            ui.label("Pos");
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
        self.sel_verts = self.mesh().verts_of_faces(&faces);
        self.sel_faces = faces;
        // The undo step already exists, so the modal must not add another one.
        self.start_modal_no_undo(crate::viewport::ModalKind::Grab, Some(normal));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.modal.is_some() {
            return;
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
        self.poll_texture(&ctx);
        self.shortcuts(&ctx);

        // Menus and tools are locked while a move / rotate / scale is in progress.
        let idle = self.modal.is_none();
        egui::Panel::top("menu").show(ui, |ui| ui.add_enabled_ui(idle, |ui| self.menu_bar(ui)));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
            });
        });
        egui::Panel::right("tools").resizable(false).exact_size(250.0).show(ui, |ui| ui.add_enabled_ui(idle, |ui| self.tool_panel(ui)));
        if self.show_uv {
            egui::Panel::left("uv").resizable(true).default_size(440.0).min_size(200.0).show(ui, |ui| self.uv_editor(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));
    }

    /// Tab switches object / edit mode. It has to be removed from the input before egui
    /// sees it, otherwise egui also moves keyboard focus to the next widget.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let is_tab = |e: &egui::Event| matches!(e, egui::Event::Key { key: Key::Tab, .. });
        let pressed = raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Tab, pressed: true, repeat: false, .. }));
        raw.events.retain(|e| !is_tab(e));
        if pressed && self.modal.is_none() && !ctx.egui_wants_keyboard_input() {
            self.toggle_edit_mode();
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let (Some(gl), Some(r)) = (gl, &self.renderer) {
            r.lock().unwrap().destroy(gl);
        }
    }
}
