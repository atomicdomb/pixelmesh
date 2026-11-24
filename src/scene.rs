//! Scene-level features: settings and recent files, autosave and recovery, hiding
//! objects, export variants, reference images, and a few selection-based mesh commands.

use crate::app::{edge_key, App, Edge, ProjectFile, SelMode};
use eframe::egui::{self, Color32, Key, RichText};
use glam::{Quat, Vec2, Vec3};
use pixelmesh::fbx;
use pixelmesh::mesh::Mesh;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MUTED: Color32 = Color32::from_rgb(138, 144, 156);

/// Where settings and the recovery file live. `PIXELMESH_CONFIG_DIR` overrides it.
pub fn config_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = if let Some(dir) = std::env::var_os("PIXELMESH_CONFIG_DIR") {
        PathBuf::from(dir)
    } else if cfg!(target_os = "macos") {
        home?.join("Library/Application Support/PixelMesh")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("PixelMesh")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or(home.map(|h| h.join(".config")))?.join("pixelmesh")
    };
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Things remembered between sessions.
#[derive(Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub recent: Vec<String>,
    #[serde(default)]
    pub palette: Option<String>,
}

impl Settings {
    pub fn load() -> Self {
        config_dir()
            .and_then(|d| std::fs::read(d.join("settings.json")).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let (Some(dir), Ok(bytes)) = (config_dir(), serde_json::to_vec_pretty(self)) {
            let _ = std::fs::write(dir.join("settings.json"), bytes);
        }
    }
}

/// Periodic safety copy of unsaved work.
#[derive(Default)]
pub struct Autosave {
    dir: Option<PathBuf>,
    last: f64,
    /// (scene edit count, dirty texture version) that the recovery file holds.
    saved: (u64, u64),
    /// A recovery file from a session that did not exit cleanly, waiting for the user's decision.
    pub pending: Option<PathBuf>,
    writing: Arc<AtomicBool>,
    /// The user has confirmed quitting with unsaved changes.
    pub quitting: bool,
}

impl Autosave {
    pub fn project_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("recovery.pmesh"))
    }

    fn texture_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("recovery.png"))
    }

    /// Look for a recovery file left behind by a crash.
    pub fn find_recovery(&mut self) {
        self.dir = config_dir();
        self.pending = self.project_path().filter(|p| p.is_file());
    }

    /// Remove the recovery file: the work is saved, or the user does not want it.
    pub fn clear(&self) {
        for p in [self.project_path(), self.texture_path()].into_iter().flatten() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// A picture shown behind (or over) the model in one of the straight-on views, to model against.
pub struct RefImage {
    pub path: PathBuf,
    pub handle: egui::TextureHandle,
    pub size: [u32; 2],
    /// Height in scene units; the width follows from the picture's proportions.
    pub height: f32,
    pub offset: Vec2,
    pub opacity: f32,
    pub in_front: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RefSave {
    pub slot: usize,
    pub path: String,
    pub height: f32,
    pub offset: [f32; 2],
    pub opacity: f32,
    pub in_front: bool,
}

pub const REF_SLOTS: [&str; 3] = ["Front", "Side", "Top"];

/// Copy / cut / paste pressed this frame. The platform layer turns Ctrl+C, X and V into
/// clipboard events rather than key presses, so both are checked.
pub fn clipboard_keys(ctx: &egui::Context) -> (bool, bool, bool) {
    ctx.input(|i| {
        let mut r = (false, false, false);
        for e in &i.events {
            match e {
                egui::Event::Copy => r.0 = true,
                egui::Event::Cut => r.1 = true,
                egui::Event::Paste(_) => r.2 = true,
                _ => {}
            }
        }
        let cmd = i.modifiers.command && !i.modifiers.shift;
        (r.0 || (cmd && i.key_pressed(Key::C)), r.1 || (cmd && i.key_pressed(Key::X)), r.2 || (cmd && i.key_pressed(Key::V)))
    })
}

impl App {
    // ------------------------------------------------------- recent files

    /// Put a file at the top of the recent list.
    pub fn remember(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().into_owned();
        self.settings.recent.retain(|p| *p != path);
        self.settings.recent.insert(0, path);
        self.settings.recent.truncate(10);
        self.settings.save();
    }

    pub fn recent_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let recent = self.settings.recent.clone();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button("Open recent", |ui| {
                for entry in &recent {
                    let path = Path::new(entry);
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| entry.clone());
                    let is_fbx = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fbx"));
                    let label = if is_fbx { format!("{name}   (import)") } else { name };
                    if ui.add_enabled(path.is_file(), egui::Button::new(label)).on_hover_text(entry).on_disabled_hover_text(format!("{entry}\n(no longer there)")).clicked() {
                        if is_fbx {
                            self.import_fbx(ctx, path);
                        } else {
                            self.open_project(ctx, path);
                        }
                    }
                }
                ui.separator();
                if ui.button("Clear list").clicked() {
                    self.settings.recent.clear();
                    self.settings.save();
                }
            });
        });
    }

    // ----------------------------------------------------------- autosave

    /// True when quitting now would lose something.
    pub fn unsaved(&self) -> bool {
        self.edits != self.saved_edits || self.texture.as_ref().is_some_and(|t| t.dirty)
    }

    /// Once a minute, write whatever is unsaved to the recovery file (on a background thread).
    pub fn autosave_tick(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let interval = std::env::var("PIXELMESH_AUTOSAVE_SECS").ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(60.0);
        if self.autosave.pending.is_some() || self.busy() || self.paint.is_painting() || now - self.autosave.last < interval {
            return;
        }
        self.autosave.last = now;
        let tex_version = self.texture.as_ref().map_or(0, |t| if t.dirty { t.version } else { 0 });
        let state = (self.edits, tex_version);
        if state == self.autosave.saved || self.autosave.writing.load(Ordering::Relaxed) {
            return;
        }
        if !self.unsaved() {
            self.autosave.clear();
            self.autosave.saved = state;
            return;
        }
        let (Some(project), Some(tex_path)) = (self.autosave.project_path(), self.autosave.texture_path()) else { return };
        let mut file = self.project_file();
        file.recovered_project = Some(self.project_path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        let mut png = None;
        if let Some(t) = self.texture.as_ref().filter(|t| t.dirty || !t.on_disk) {
            // Unsaved pixels go along as their own image.
            match Self::encode_png(t) {
                Ok(bytes) => {
                    png = Some(bytes);
                    file.recovered_texture = file.texture.take();
                    file.texture = Some(tex_path.to_string_lossy().into_owned());
                }
                Err(_) => return,
            }
        }
        self.autosave.saved = state;
        let writing = self.autosave.writing.clone();
        writing.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            // Written under another name first, so a crash mid-write cannot leave half a file.
            let write = |path: &Path, bytes: &[u8]| {
                let tmp = path.with_extension("tmp");
                std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path))
            };
            if let Some(bytes) = png {
                let _ = write(&tex_path, &bytes);
            }
            if let Ok(bytes) = serde_json::to_vec(&file) {
                let _ = write(&project, &bytes);
            }
            writing.store(false, Ordering::Relaxed);
        });
    }

    /// Called after a recovery file was opened: point the project and texture back at their real files.
    pub fn finish_recovery(&mut self, file: &ProjectFile) {
        self.project_path = file.recovered_project.as_ref().filter(|p| !p.is_empty()).map(PathBuf::from);
        if let (Some(t), Some(original)) = (&mut self.texture, &file.recovered_texture) {
            t.path = PathBuf::from(original);
            t.on_disk = t.path.is_file();
            t.mtime = std::fs::metadata(&t.path).and_then(|m| m.modified()).ok();
            t.dirty = true;
        }
        // Recovered work has not been saved anywhere yet.
        self.edits += 1;
        self.status = "Recovered your unsaved work. Save it to keep it.".into();
    }

    /// The recovery prompt at start-up and the "unsaved changes" prompt on quitting.
    pub fn scene_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.autosave.pending.clone() {
            let when = std::fs::metadata(&path).and_then(|m| m.modified()).ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| match d.as_secs() {
                    s if s < 120 => "a moment ago".to_string(),
                    s if s < 7200 => format!("{} minutes ago", s / 60),
                    s if s < 172_800 => format!("{} hours ago", s / 3600),
                    s => format!("{} days ago", s / 86_400),
                })
                .unwrap_or_else(|| "earlier".into());
            let mut choice = None;
            egui::Window::new("Recover unsaved work?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!("PixelMesh did not close properly last time.\nAn automatic copy of your work from {when} was kept."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(false);
                    }
                });
            });
            if let Some(recover) = choice {
                self.autosave.pending = None;
                if recover {
                    self.open_project(ctx, &path);
                } else {
                    self.autosave.clear();
                }
            }
        }

        // Closing the window with unsaved work asks first.
        if ctx.input(|i| i.viewport().close_requested()) && self.unsaved() && !self.autosave.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_quit = true;
        }
        if self.confirm_quit {
            let mut choice = None;
            egui::Window::new("Unsaved changes").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                let mut what = Vec::new();
                if self.edits != self.saved_edits {
                    what.push("the scene");
                }
                if self.texture.as_ref().is_some_and(|t| t.dirty) {
                    what.push("the image");
                }
                ui.label(format!("You have unsaved changes to {}.", what.join(" and ")));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save and quit").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Quit without saving").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    if self.texture.as_ref().is_some_and(|t| t.dirty) {
                        self.save_texture();
                    }
                    if self.edits != self.saved_edits {
                        match self.project_path.clone() {
                            Some(p) => self.save_project(&p),
                            None => self.save_as(),
                        }
                    }
                    self.confirm_quit = false;
                    // Still unsaved means a save dialog was cancelled: stay open.
                    if !self.unsaved() {
                        self.autosave.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Some(1) => {
                    self.confirm_quit = false;
                    self.autosave.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(_) => self.confirm_quit = false,
                None => {}
            }
        }
    }

    // -------------------------------------------------------- hide / show

    pub fn hide_selected(&mut self) {
        if self.sel_mode != SelMode::Object {
            self.status = "Hide works on whole objects: press Tab to go back to object mode first.".into();
            return;
        }
        let n = self.sel_objects.len();
        for &i in &self.sel_objects {
            self.objects[i].visible = false;
        }
        self.sel_objects.clear();
        self.status = format!("Hid {n} object(s). Alt+H shows everything again.");
    }

    /// Hide everything except the selection (in an edit mode: except the object being edited).
    pub fn isolate_selected(&mut self) {
        let keep: BTreeSet<usize> = if self.sel_mode == SelMode::Object { self.sel_objects.clone() } else { [self.active].into() };
        if keep.is_empty() {
            self.status = "Select the objects to keep in view first.".into();
            return;
        }
        for (i, obj) in self.objects.iter_mut().enumerate() {
            obj.visible = keep.contains(&i);
        }
        self.status = format!("Showing only {} object(s). Alt+H shows everything again.", keep.len());
    }

    pub fn show_all(&mut self) {
        for obj in &mut self.objects {
            obj.visible = true;
        }
        self.status = "Everything is visible.".into();
    }

    // -------------------------------------------------------------- export

    /// Write the given objects to an FBX file. Returns how many were written and a note about the texture.
    fn write_fbx(&mut self, path: &Path, ids: &[usize]) -> Result<(usize, String), String> {
        // Painted but unsaved pixels still have to end up in the exported texture.
        if let Some(t) = &mut self.texture {
            if t.dirty || t.file_bytes.is_empty() {
                t.file_bytes = Self::encode_png(t).map_err(|e| format!("could not encode the texture ({e})"))?;
            }
        }
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Mesh").to_string();
        let tex_name = self.texture.as_ref().and_then(|t| t.path.file_name().and_then(|n| n.to_str()).map(str::to_string));
        // Objects without faces would only produce empty nodes. Modifiers are applied in the exported geometry.
        let chosen: Vec<&crate::app::Object> = ids.iter().filter_map(|&i| self.objects.get(i)).filter(|o| !o.mesh.faces.is_empty()).collect();
        let meshes: Vec<std::borrow::Cow<Mesh>> = chosen.iter().map(|o| o.display_mesh()).collect();
        let objects: Vec<fbx::FbxObject> = chosen.iter().zip(&meshes)
            .map(|(o, mesh)| fbx::FbxObject { name: &o.name, mesh, origin: o.origin, rotation: o.rotation })
            .collect();
        if objects.is_empty() {
            return Err("nothing to export: no faces".into());
        }
        let opt = fbx::FbxOptions {
            material_name: &name,
            scale: 1.0,
            texture: match (&self.texture, &tex_name) {
                (Some(t), Some(n)) => Some(fbx::FbxTexture { file_name: n, embed: self.embed_texture.then_some(t.file_bytes.as_slice()) }),
                _ => None,
            },
        };
        fbx::export_file(path, &objects, &opt).map_err(|e| e.to_string())?;
        let mut note = String::new();
        // The FBX refers to the texture by file name, so make sure a copy sits next to it.
        if let (Some(t), Some(n), Some(dir)) = (&self.texture, &tex_name, path.parent()) {
            let dest = dir.join(n);
            if !self.embed_texture && !dest.exists() {
                match std::fs::write(&dest, &t.file_bytes) {
                    Ok(()) => note = format!(" (+ wrote {n})"),
                    Err(e) => note = format!(" (could not write {n}: {e})"),
                }
            } else if !self.embed_texture && t.dirty {
                note = format!(" (note: {n} next to it is older than your unsaved paint changes; save the image)");
            }
        }
        Ok((objects.len(), note))
    }

    pub fn export_fbx(&mut self, path: &Path) {
        let all: Vec<usize> = (0..self.objects.len()).collect();
        let ids: Vec<usize> = if !self.export_selected {
            all.clone()
        } else if self.sel_mode == SelMode::Object {
            self.sel_objects.iter().copied().collect()
        } else {
            vec![self.active]
        };
        match self.write_fbx(path, &ids) {
            Ok((count, note)) => {
                if ids == all {
                    // A full export counts as having saved the scene.
                    self.saved_edits = self.edits;
                }
                let which = if ids == all { "" } else { "selected " };
                self.status = format!("Exported {count} {which}object(s) to {}{note}", path.display());
            }
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    pub fn pick_export(&mut self) {
        let default = self.project_path.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()).unwrap_or("model").to_string();
        if let Some(p) = rfd::FileDialog::new().add_filter("FBX", &["fbx"]).set_file_name(format!("{default}.fbx")).save_file() {
            self.export_fbx(&p);
        }
    }

    /// One FBX per name group, written into `dir`.
    pub fn export_groups(&mut self, dir: &Path) {
        let mut groups: std::collections::BTreeMap<String, Vec<usize>> = std::collections::BTreeMap::new();
        for (i, obj) in self.objects.iter().enumerate() {
            if let Some(prefix) = crate::sidebar::group_prefix(&obj.name) {
                groups.entry(prefix.to_string()).or_default().push(i);
            }
        }
        // A prefix only one object has is not a group.
        groups.retain(|_, ids| ids.len() >= 2);
        let grouped: BTreeSet<usize> = groups.values().flatten().copied().collect();
        let other: Vec<usize> = (0..self.objects.len()).filter(|i| !grouped.contains(i)).collect();
        if !other.is_empty() {
            let name = if groups.contains_key("Other") { "Ungrouped" } else { "Other" };
            groups.insert(name.to_string(), other);
        }
        let (mut files, mut objects, mut errors) = (0, 0, Vec::new());
        for (name, ids) in groups {
            let safe: String = name.chars().map(|c| if c.is_alphanumeric() || "-_ .".contains(c) { c } else { '_' }).collect();
            match self.write_fbx(&dir.join(format!("{safe}.fbx")), &ids) {
                Ok((count, _)) => {
                    files += 1;
                    objects += count;
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        self.status = if errors.is_empty() {
            self.saved_edits = self.edits;
            format!("Exported {objects} object(s) into {files} file(s) in {}", dir.display())
        } else {
            format!("Exported {files} file(s); problems: {}", errors.join("; "))
        };
    }

    pub fn pick_export_groups(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Folder for the group FBX files").pick_folder() {
            self.export_groups(&dir);
        }
    }

    // ---------------------------------------------------- reference images

    pub fn refs_to_save(&self) -> Vec<RefSave> {
        self.refs.iter().enumerate().filter_map(|(slot, r)| r.as_ref().map(|r| RefSave {
            slot,
            path: r.path.to_string_lossy().into_owned(),
            height: r.height,
            offset: r.offset.to_array(),
            opacity: r.opacity,
            in_front: r.in_front,
        })).collect()
    }

    pub fn load_reference(&mut self, ctx: &egui::Context, save: &RefSave) -> bool {
        if save.slot >= self.refs.len() {
            return false;
        }
        let img = match image::open(&save.path) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                self.status = format!("Could not load reference {}: {e}", save.path);
                return false;
            }
        };
        let size = [img.width(), img.height()];
        let color = egui::ColorImage::from_rgba_unmultiplied([size[0] as usize, size[1] as usize], img.as_raw());
        let handle = ctx.load_texture(format!("reference-{}", save.slot), color, egui::TextureOptions::NEAREST);
        self.refs[save.slot] = Some(RefImage {
            path: PathBuf::from(&save.path),
            handle,
            size,
            height: save.height,
            offset: Vec2::from_array(save.offset),
            opacity: save.opacity,
            in_front: save.in_front,
        });
        true
    }

    /// World-space corners of a reference image: top-left, top-right, bottom-right, bottom-left.
    fn reference_corners(slot: usize, r: &RefImage) -> [Vec3; 4] {
        let (right, up) = match slot {
            0 => (Vec3::X, Vec3::Y),
            // Seen from the right-hand side (+X), "to the right" is towards -Z.
            1 => (-Vec3::Z, Vec3::Y),
            _ => (Vec3::X, -Vec3::Z),
        };
        let (h, w) = (r.height, r.height * r.size[0] as f32 / r.size[1].max(1) as f32);
        // Front and side pictures stand on the ground; the top picture is centred.
        let lift = if slot == 2 { 0.0 } else { h * 0.5 };
        let c = right * r.offset.x + up * (r.offset.y + lift);
        [c - right * w * 0.5 + up * h * 0.5, c + right * w * 0.5 + up * h * 0.5, c + right * w * 0.5 - up * h * 0.5, c - right * w * 0.5 - up * h * 0.5]
    }

    /// Draw the reference image belonging to the current straight-on view, if there is one.
    pub fn draw_references(&self, painter: &egui::Painter, rect: egui::Rect, in_front: bool) {
        if !self.camera.ortho {
            return;
        }
        let slot = match self.view_name() {
            Some("Front" | "Back") => 0,
            Some("Right" | "Left") => 1,
            Some("Top" | "Bottom") => 2,
            _ => return,
        };
        let Some(r) = self.refs[slot].as_ref().filter(|r| r.in_front == in_front) else { return };
        let corners = Self::reference_corners(slot, r).map(|p| self.camera.project(rect, p));
        let [Some(a), Some(b), Some(c), Some(d)] = corners else { return };
        let tint = Color32::from_rgba_unmultiplied(255, 255, 255, (r.opacity.clamp(0.0, 1.0) * 255.0) as u8);
        let mut mesh = egui::Mesh::with_texture(r.handle.id());
        for (pos, uv) in [(a, (0.0, 0.0)), (b, (1.0, 0.0)), (c, (1.0, 1.0)), (d, (0.0, 1.0))] {
            mesh.vertices.push(egui::epaint::Vertex { pos, uv: egui::pos2(uv.0, uv.1), color: tint });
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        painter.with_clip_rect(rect).add(egui::Shape::mesh(mesh));
    }

    pub fn reference_card(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            for (i, name) in REF_SLOTS.iter().enumerate() {
                let label = if self.refs[i].is_some() { format!("{name} •") } else { name.to_string() };
                ui.selectable_value(&mut self.ref_slot, i, label);
            }
        });
        let slot = self.ref_slot;
        let mut remove = false;
        ui.horizontal(|ui| {
            if ui.small_button("Load…").on_hover_text("Choose a picture to model against. It shows in this straight-on view (and its opposite) only.").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "bmp", "tga"]).pick_file() {
                    let previous = self.refs[slot].as_ref();
                    let save = RefSave {
                        slot,
                        path: p.to_string_lossy().into_owned(),
                        height: previous.map_or(2.0, |r| r.height),
                        offset: previous.map_or([0.0, 0.0], |r| r.offset.to_array()),
                        opacity: previous.map_or(0.5, |r| r.opacity),
                        in_front: previous.is_some_and(|r| r.in_front),
                    };
                    if self.load_reference(&ctx, &save) {
                        // Go and look at it.
                        let view = crate::app::VIEWS[slot * 2];
                        self.set_view(view.1, view.2);
                    }
                }
            }
            if ui.add_enabled(self.refs[slot].is_some(), egui::Button::new("Remove").small()).clicked() {
                remove = true;
            }
        });
        if remove {
            self.refs[slot] = None;
        }
        let Some(r) = &mut self.refs[slot] else {
            ui.label(RichText::new("No picture for this view.").size(10.5).color(MUTED));
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Height").color(MUTED));
            ui.add(egui::DragValue::new(&mut r.height).speed(0.02).range(0.01..=1000.0).max_decimals(3)).on_hover_text("Height of the picture in scene units");
            ui.label(RichText::new("Opacity").color(MUTED));
            let mut percent = (r.opacity * 100.0).round();
            if ui.add(egui::DragValue::new(&mut percent).range(5.0..=100.0).suffix("%")).changed() {
                r.opacity = percent / 100.0;
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Offset").color(MUTED));
            ui.add(egui::DragValue::new(&mut r.offset.x).speed(0.01).prefix("X ").max_decimals(3));
            ui.add(egui::DragValue::new(&mut r.offset.y).speed(0.01).prefix("Y ").max_decimals(3));
            ui.checkbox(&mut r.in_front, "In front").on_hover_text("Draw the picture over the model instead of behind it");
        });
    }

    // ------------------------------------------------------ transform size

    /// X/Y/Z size of the active object (along its own axes) or of the selection; editing it scales.
    pub fn size_fields(&mut self, ui: &mut egui::Ui) {
        let object_mode = self.sel_mode == SelMode::Object;
        let Some(obj) = self.objects.get(self.active).filter(|o| o.visible) else { return };
        let (verts, frame, fixed): (Vec<u32>, Quat, Option<Vec3>) = if object_mode {
            if !self.sel_objects.contains(&self.active) || self.origin_only {
                return;
            }
            ((0..obj.mesh.positions.len() as u32).collect(), pixelmesh::xform::euler_quat(obj.rotation), Some(obj.origin))
        } else {
            (self.active_verts().into_iter().collect(), Quat::IDENTITY, None)
        };
        if verts.len() < 2 {
            return;
        }
        let inverse = frame.inverse();
        let local = |p: Vec3| inverse * p;
        let (lo, hi) = verts.iter().map(|&v| local(obj.mesh.positions[v as usize]))
            .fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
        let size = hi - lo;
        // Objects scale about their origin, a selection about its middle.
        let pivot = fixed.map_or((lo + hi) * 0.5, local);
        let mut edited = size;
        let mut started = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Size").color(MUTED)).on_hover_text(if object_mode {
                "Size of the active object along its own axes. Editing a number scales the object about its origin."
            } else {
                "Size of the selection. Editing a number scales it about its middle."
            });
            for (axis, label) in ["X ", "Y ", "Z "].into_iter().enumerate() {
                let r = ui.add_enabled(size[axis] > 1e-6, egui::DragValue::new(&mut edited[axis]).speed(0.01).range(0.0..=100_000.0).prefix(label).max_decimals(3));
                started |= r.drag_started() || r.gained_focus();
            }
        });
        if started {
            self.push_undo();
        }
        let edited = edited.max(Vec3::splat(0.001));
        if (edited - size).abs().max_element() > 1e-7 && edited.cmpne(size.max(Vec3::splat(0.001))).any() {
            let factor = Vec3::new(
                if size.x > 1e-6 { edited.x / size.x } else { 1.0 },
                if size.y > 1e-6 { edited.y / size.y } else { 1.0 },
                if size.z > 1e-6 { edited.z / size.z } else { 1.0 },
            );
            let mesh = &mut self.objects[self.active].mesh;
            for v in verts {
                let p = &mut mesh.positions[v as usize];
                *p = frame * (pivot + (inverse * *p - pivot) * factor);
            }
        }
    }

    // --------------------------------------------------------- UV copy/paste

    pub fn copy_uvs(&mut self) {
        let faces = self.active_faces();
        if faces.is_empty() {
            self.status = "Copy UVs: select the faces to copy from first.".into();
            return;
        }
        self.uv_clipboard = faces.iter().map(|&f| self.mesh().faces[f].uvs.clone()).collect();
        // Something has to be on the system clipboard for a later Ctrl+V to reach the program.
        self.clipboard_marker = Some("PixelMesh UVs");
        self.status = format!("Copied the UVs of {} face(s). Select other faces and paste (Ctrl+V).", faces.len());
    }

    /// Give the selected faces the copied UVs. Each face takes them from a copied face with the
    /// same number of corners; with several of those, they are handed out in order.
    pub fn paste_uvs(&mut self) {
        if self.uv_clipboard.is_empty() {
            self.status = "Nothing copied yet: select faces and copy their UVs first (Ctrl+C).".into();
            return;
        }
        let faces = self.active_faces();
        if faces.is_empty() {
            self.status = "Paste UVs: select the faces to paste onto first.".into();
            return;
        }
        let clipboard = self.uv_clipboard.clone();
        let mut used: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        let mut plan = Vec::new();
        for &f in &faces {
            let corners = self.mesh().faces[f].uvs.len();
            let sources: Vec<&Vec<Vec2>> = clipboard.iter().filter(|uvs| uvs.len() == corners).collect();
            if sources.is_empty() {
                continue;
            }
            let k = used.entry(corners).or_default();
            plan.push((f, sources[*k % sources.len()].clone()));
            *k += 1;
        }
        if plan.is_empty() {
            self.status = "Could not paste: none of the copied faces has the same number of corners as the selected ones.".into();
            return;
        }
        self.push_undo();
        let (done, total) = (plan.len(), faces.len());
        for (f, uvs) in plan {
            self.mesh_mut().faces[f].uvs = uvs;
        }
        self.status = if done == total {
            format!("Pasted UVs onto {done} face(s). If one comes out turned, use Rotate in UV Mapping.")
        } else {
            format!("Pasted UVs onto {done} of {total} faces (the rest have a different number of corners).")
        };
    }

    // ------------------------------------------- subdivide / bridge / fill

    /// Edges the edge tools act on, whatever the current edit mode.
    fn selected_edges(&self) -> BTreeSet<Edge> {
        match self.sel_mode {
            SelMode::Object => BTreeSet::new(),
            SelMode::Edge => self.sel_edges.clone(),
            SelMode::Vertex => self.mesh().edges().into_iter()
                .filter(|(a, b)| self.sel_verts.contains(a) && self.sel_verts.contains(b))
                .map(|(a, b)| edge_key(a, b))
                .collect(),
            // The outline of the selected faces.
            SelMode::Face => {
                let mut count: std::collections::HashMap<Edge, usize> = std::collections::HashMap::new();
                for &f in &self.sel_faces {
                    let v = &self.mesh().faces[f].verts;
                    for i in 0..v.len() {
                        *count.entry(edge_key(v[i], v[(i + 1) % v.len()])).or_default() += 1;
                    }
                }
                count.into_iter().filter(|(_, n)| *n == 1).map(|(e, _)| e).collect()
            }
        }
    }

    pub fn subdivide_selection(&mut self) {
        let faces = if self.sel_mode == SelMode::Object { BTreeSet::new() } else { self.active_faces() };
        if faces.is_empty() {
            self.status = "Subdivide works on faces: press Tab, select faces, then Subdivide.".into();
            return;
        }
        self.push_undo();
        let new = self.mesh_mut().subdivide_faces(&faces);
        let count = new.len();
        self.select_faces(new);
        self.status = format!("Subdivided {} face(s) into {count}.", faces.len());
    }

    fn finish_face_op(&mut self, result: Result<BTreeSet<usize>, &'static str>, what: &str) {
        match result {
            Ok(new) => {
                let count = new.len();
                self.set_mode(SelMode::Face);
                self.select_faces(new);
                self.status = format!("{what}: {count} new face(s), selected.");
            }
            Err(e) => {
                self.revert_last();
                self.status = format!("{what}: {e}.");
            }
        }
    }

    /// Join two edge loops (or two faces, which are removed) with a band of faces.
    pub fn bridge_selection(&mut self) {
        if self.sel_mode == SelMode::Object {
            self.status = "Bridge works in an edit mode: select two edge loops, or two faces.".into();
            return;
        }
        self.push_undo();
        let result = match (self.sel_mode, self.sel_faces.len()) {
            (SelMode::Face, 2) => {
                let mut it = self.sel_faces.iter().copied();
                let (a, b) = (it.next().unwrap(), it.next().unwrap());
                if self.mesh().faces[a].verts.len() != self.mesh().faces[b].verts.len() {
                    Err("the two faces need the same number of corners")
                } else {
                    self.mesh_mut().bridge_faces(a, b)
                }
            }
            (SelMode::Face, _) => Err("select exactly two faces (or two edge loops in edge mode)"),
            _ => {
                let edges = self.selected_edges();
                self.mesh_mut().bridge_edges(&edges)
            }
        };
        self.finish_face_op(result, "Bridge");
    }

    /// The vertices "make face" would use: the selected ones, or the ends of the selected edges.
    fn face_candidates(&self) -> BTreeSet<u32> {
        match self.sel_mode {
            SelMode::Vertex => self.sel_verts.clone(),
            SelMode::Edge => self.sel_edges.iter().flat_map(|&(a, b)| [a, b]).collect(),
            _ => BTreeSet::new(),
        }
    }

    /// True when F should make a face instead of switching to face mode:
    /// three or more vertices are selected and they are not a face already.
    pub fn can_make_face(&self) -> bool {
        let verts = self.face_candidates();
        verts.len() >= 3 && !self.mesh().faces.iter().any(|f| f.verts.len() == verts.len() && f.verts.iter().all(|v| verts.contains(v)))
    }

    /// Make a face from the selected vertices.
    pub fn make_face(&mut self) {
        if !matches!(self.sel_mode, SelMode::Vertex | SelMode::Edge) {
            self.status = "Make face works in vertex or edge mode: select three or more vertices, then F.".into();
            return;
        }
        let verts = self.face_candidates();
        let toward = self.camera.back();
        self.push_undo();
        let result = self.mesh_mut().face_from_verts(&verts, toward).map(|f| BTreeSet::from([f]));
        self.finish_face_op(result, "Make face");
    }

    /// Knife: click points on edges; each click cuts the face between it and the last one.
    pub fn start_knife(&mut self) {
        if self.sel_mode == SelMode::Object {
            self.status = "The knife works in an edit mode: press Tab first.".into();
            return;
        }
        if self.busy() {
            return;
        }
        self.push_undo();
        self.sel_verts.clear();
        self.sel_edges.clear();
        self.sel_faces.clear();
        self.knife = Some(crate::viewport::Knife::default());
    }

    /// Close the hole outlined by the selected edges with a new face.
    pub fn fill_selection(&mut self) {
        if self.sel_mode == SelMode::Object {
            self.status = "Fill works in an edit mode: select the edges around a hole.".into();
            return;
        }
        let edges = if self.sel_mode == SelMode::Face { BTreeSet::new() } else { self.selected_edges() };
        self.push_undo();
        let result = if edges.is_empty() {
            Err("select the edges (or vertices) around a hole")
        } else {
            self.mesh_mut().fill_edges(&edges)
        };
        self.finish_face_op(result, "Fill");
    }
}
