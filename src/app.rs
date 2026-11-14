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
    Vertex,
    Face,
}

pub struct Snapshot {
    mesh: Mesh,
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
    mesh: Mesh,
    texture: Option<String>,
    px_per_unit: f32,
}

pub struct App {
    pub mesh: Mesh,
    pub sel_mode: SelMode,
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

    pub grid_snap: f32,
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
            mesh: Mesh::cube(),
            sel_mode: SelMode::Face,
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
            grid_snap: 0.25,
            px_per_unit: 16.0,
            segments: 8,
            embed_texture: false,
            project_path: None,
            status: "Welcome. Add a mesh, load a texture, then drag faces around in the UV editor.".into(),
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

    // ------------------------------------------------------------ selection

    /// Vertices the transform tools act on.
    pub fn active_verts(&self) -> BTreeSet<u32> {
        match self.sel_mode {
            SelMode::Vertex => self.sel_verts.clone(),
            SelMode::Face => self.mesh.verts_of_faces(&self.sel_faces),
        }
    }

    /// Faces the face / UV tools act on. In vertex mode: faces whose corners are all selected.
    pub fn active_faces(&self) -> BTreeSet<usize> {
        match self.sel_mode {
            SelMode::Face => self.sel_faces.clone(),
            SelMode::Vertex => (0..self.mesh.faces.len())
                .filter(|&f| self.mesh.faces[f].verts.iter().all(|v| self.sel_verts.contains(v)))
                .collect(),
        }
    }

    pub fn set_mode(&mut self, mode: SelMode) {
        if mode == self.sel_mode {
            return;
        }
        // Carry the selection over so switching modes does not lose it.
        match mode {
            SelMode::Vertex => self.sel_verts = self.mesh.verts_of_faces(&self.sel_faces),
            SelMode::Face => self.sel_faces = self.active_faces(),
        }
        self.sel_mode = mode;
    }

    pub fn select_all(&mut self) {
        self.sel_verts = (0..self.mesh.positions.len() as u32).collect();
        self.sel_faces = (0..self.mesh.faces.len()).collect();
    }

    pub fn clear_selection(&mut self) {
        self.sel_verts.clear();
        self.sel_faces.clear();
    }

    pub fn select_linked(&mut self) {
        let seed: BTreeSet<usize> = match self.sel_mode {
            SelMode::Face => self.sel_faces.clone(),
            SelMode::Vertex => (0..self.mesh.faces.len())
                .filter(|&f| self.mesh.faces[f].verts.iter().any(|v| self.sel_verts.contains(v)))
                .collect(),
        };
        self.sel_faces = self.mesh.linked_faces(&seed);
        self.sel_verts = self.mesh.verts_of_faces(&self.sel_faces);
    }

    // ----------------------------------------------------------------- undo

    fn snapshot(&self) -> Snapshot {
        Snapshot { mesh: self.mesh.clone(), sel_verts: self.sel_verts.clone(), sel_faces: self.sel_faces.clone() }
    }

    fn restore(&mut self, s: Snapshot) {
        self.mesh = s.mesh;
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

    fn add_primitive(&mut self, prim: Mesh) {
        self.push_undo();
        let new_faces = self.mesh.append(&prim);
        self.sel_mode = SelMode::Face;
        self.sel_verts.clear();
        self.sel_faces = new_faces;
        self.status = "Added mesh (selected). Press G to move it.".into();
    }

    pub fn delete_selection(&mut self) {
        if self.active_verts().is_empty() {
            return;
        }
        self.push_undo();
        match self.sel_mode {
            SelMode::Face => self.mesh.delete_faces(&self.sel_faces.clone()),
            SelMode::Vertex => self.mesh.delete_verts(&self.sel_verts.clone()),
        }
        self.clear_selection();
    }

    pub fn duplicate_selection(&mut self) -> bool {
        let faces = self.active_faces();
        if faces.is_empty() {
            self.status = "Duplicate needs at least one whole face selected.".into();
            return false;
        }
        self.push_undo();
        let new_faces = self.mesh.duplicate_faces(&faces);
        self.sel_verts = self.mesh.verts_of_faces(&new_faces);
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
        f(&mut self.mesh, &faces);
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
            version: 1,
            mesh: self.mesh.clone(),
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
            Ok(file) => {
                let valid = file.mesh.faces.iter().all(|f| {
                    f.verts.len() >= 3 && f.verts.len() == f.uvs.len()
                        && f.verts.iter().all(|&v| (v as usize) < file.mesh.positions.len())
                });
                if !valid {
                    self.status = format!("{} is not a valid project file", path.display());
                    return;
                }
                self.mesh = file.mesh;
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
        let opt = fbx::FbxOptions {
            name: &name,
            scale: 1.0,
            texture: match (&self.texture, &tex_name) {
                (Some(t), Some(n)) => Some(fbx::FbxTexture {
                    file_name: n,
                    embed: self.embed_texture.then_some(t.file_bytes.as_slice()),
                }),
                _ => None,
            },
        };
        match fbx::export_file(path, &self.mesh, &opt) {
            Ok(()) => {
                let mut msg = format!("Exported {}", path.display());
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
                    self.mesh = Mesh::default();
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
                if ui.button("Frame all   Home").clicked() {
                    self.frame_all();
                }
            });
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
        if self.mesh.positions.is_empty() {
            self.camera = Camera::default();
            return;
        }
        let lo = self.mesh.positions.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
        let hi = self.mesh.positions.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
        self.camera.target = (lo + hi) * 0.5;
        self.camera.distance = ((hi - lo).length() * 1.6).max(1.0);
    }

    fn tool_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);
            ui.strong("Select");
            ui.horizontal(|ui| {
                let mut mode = self.sel_mode;
                ui.selectable_value(&mut mode, SelMode::Vertex, "Vertex (V)");
                ui.selectable_value(&mut mode, SelMode::Face, "Face (F)");
                self.set_mode(mode);
            });
            ui.horizontal(|ui| {
                if ui.button("All (A)").clicked() {
                    self.select_all();
                }
                if ui.button("None").clicked() {
                    self.clear_selection();
                }
                if ui.button("Linked (L)").clicked() {
                    self.select_linked();
                }
            });

            ui.separator();
            ui.strong("Add mesh");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Plane").clicked() {
                    self.add_primitive(Mesh::plane());
                }
                if ui.button("Cube").clicked() {
                    self.add_primitive(Mesh::cube());
                }
                if ui.button("Cylinder").clicked() {
                    self.add_primitive(Mesh::cylinder(self.segments, 0.5, 0.5, 1.0));
                }
                if ui.button("Cone").clicked() {
                    self.add_primitive(Mesh::cylinder(self.segments, 0.5, 0.0, 1.0));
                }
                if ui.button("Sphere").clicked() {
                    self.add_primitive(Mesh::sphere(self.segments, (self.segments / 2).max(3), 0.5));
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
                ui.label("Grid snap (Ctrl)");
                ui.add(egui::DragValue::new(&mut self.grid_snap).speed(0.0125).range(0.001..=10.0));
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
                        self.mesh.flip_faces(&faces);
                    }
                }
                if ui.button("Merge overlapping verts").clicked() {
                    self.push_undo();
                    let n = self.mesh.merge_by_distance(1e-4);
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
            ui.weak(format!("{} verts · {} faces", self.mesh.positions.len(), self.mesh.faces.len()));
        });
    }

    /// X/Y/Z fields for the centre of the selection; editing them moves it.
    fn position_fields(&mut self, ui: &mut egui::Ui) {
        let verts = self.active_verts();
        if verts.is_empty() {
            return;
        }
        let centre = verts.iter().map(|&v| self.mesh.positions[v as usize]).sum::<Vec3>() / verts.len() as f32;
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
            for v in verts {
                self.mesh.positions[v as usize] += delta;
            }
        }
    }

    pub fn extrude(&mut self) {
        let faces = self.active_faces();
        if faces.is_empty() {
            self.status = "Extrude needs at least one whole face selected.".into();
            return;
        }
        self.push_undo();
        let normal = self.mesh.extrude_faces(&faces);
        self.sel_verts = self.mesh.verts_of_faces(&faces);
        self.sel_faces = faces;
        // The undo step already exists, so the modal must not add another one.
        self.start_modal_no_undo(crate::viewport::ModalKind::Grab, Some(normal));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.modal.is_some() {
            return;
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

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
            });
        });
        egui::Panel::right("tools").resizable(false).exact_size(250.0).show(ui, |ui| self.tool_panel(ui));
        egui::Panel::left("uv").resizable(true).default_size(440.0).min_size(200.0).show(ui, |ui| self.uv_editor(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let (Some(gl), Some(r)) = (gl, &self.renderer) {
            r.lock().unwrap().destroy(gl);
        }
    }
}
