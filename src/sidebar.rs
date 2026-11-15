//! The right-hand tool panel: cards of icon tiles, plus the app's colour theme.

use crate::app::{App, LeftTab, OriginTo, SelMode, SelectTool};
use crate::viewport::ModalKind;
use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke};
use pixelmesh::icons::Icon;
use pixelmesh::mesh::{Mesh, UvOp};

pub const ACCENT: Color32 = Color32::from_rgb(255, 158, 44);
const CARD: Color32 = Color32::from_rgb(34, 36, 42);
const TILE: Color32 = Color32::from_rgb(46, 49, 57);
const TILE_HOVER: Color32 = Color32::from_rgb(60, 64, 74);
const TILE_ON: Color32 = Color32::from_rgb(96, 60, 22);
const MUTED: Color32 = Color32::from_rgb(138, 144, 156);
const GAP: f32 = 4.0;

/// Dark theme with the orange selection colour as its accent.
pub fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(24, 26, 30);
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::from_rgb(16, 17, 20);
    v.selection.bg_fill = Color32::from_rgb(150, 88, 24);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(255, 226, 180));
    v.hyperlink_color = ACCENT;
    v.widgets.inactive.weak_bg_fill = TILE;
    v.widgets.inactive.bg_fill = TILE;
    v.widgets.hovered.weak_bg_fill = TILE_HOVER;
    v.widgets.hovered.bg_fill = TILE_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(110, 116, 130));
    v.widgets.active.weak_bg_fill = Color32::from_rgb(76, 81, 92);
    v.widgets.active.bg_fill = Color32::from_rgb(76, 81, 92);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = egui::CornerRadius::same(5);
    }
    ctx.set_visuals(v);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 5.0);
        style.spacing.button_padding = egui::vec2(7.0, 3.0);
    });
}

/// A titled block of the side panel.
fn card(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(CARD).corner_radius(7).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).size(10.0).strong().color(MUTED));
        add(ui);
    });
    ui.add_space(2.0);
}

/// Width of each of `n` equal tiles filling the row.
fn tile_width(ui: &egui::Ui, n: usize) -> f32 {
    ((ui.available_width() - GAP * (n as f32 - 1.0)) / n as f32).floor()
}

/// Lay out a row of tiles with the standard gap.
fn row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        add(ui);
    });
}

/// One line of the object list.
enum ListRow {
    Group(String),
    /// Object index, and whether it is shown indented under its group.
    Object(usize, bool),
}

/// The part of an object's name before the first underscore: its group.
pub fn group_prefix(name: &str) -> Option<&str> {
    name.split_once('_').map(|(prefix, _)| prefix).filter(|p| !p.is_empty())
}

impl App {
    pub fn icon(&self, icon: Icon) -> egui::TextureId {
        self.icons[&icon].id()
    }

    /// A button with a pixel-art icon above a small label.
    pub fn tile(&self, ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool, width: f32) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 38.0), Sense::click());
        if ui.is_rect_visible(rect) {
            let enabled = ui.is_enabled();
            let fill = if selected {
                TILE_ON
            } else if resp.is_pointer_button_down_on() {
                Color32::from_rgb(76, 81, 92)
            } else if resp.hovered() {
                TILE_HOVER
            } else {
                TILE
            };
            let border = if selected { ACCENT } else if resp.hovered() { Color32::from_rgb(110, 116, 130) } else { Color32::from_rgb(56, 60, 69) };
            let painter = ui.painter();
            painter.rect(rect, 5.0, fill, Stroke::new(1.0, border), egui::StrokeKind::Inside);
            // Whole-pixel position so the icon's pixels stay crisp.
            let top_left = egui::pos2((rect.center().x - 8.0).round(), rect.top() + 5.0);
            let tint = if enabled { Color32::WHITE } else { Color32::from_gray(90) };
            painter.image(self.icon(icon), Rect::from_min_size(top_left, egui::vec2(16.0, 16.0)), Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), tint);
            let text = if !enabled { Color32::from_gray(100) } else if selected { Color32::from_rgb(255, 232, 200) } else { Color32::from_gray(205) };
            painter.text(egui::pos2(rect.center().x, rect.bottom() - 8.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), text);
        }
        resp
    }

    fn open_new_canvas(&mut self) {
        self.show_uv = true;
        self.left_tab = LeftTab::Paint;
        self.paint.open_new_dialog(self.texture.as_ref().map(|t| t.size));
    }

    pub fn tool_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Export stays pinned at the bottom; everything else scrolls above it.
        egui::Panel::bottom("export").show_separator_line(false).show(ui, |ui| {
            ui.add_space(4.0);
            let export = egui::Button::image_and_text(
                egui::Image::new((self.icon(Icon::Export), egui::vec2(16.0, 16.0))),
                RichText::new("Export FBX…").strong().color(Color32::from_rgb(255, 236, 210)),
            ).fill(Color32::from_rgb(140, 82, 22)).min_size(egui::vec2(ui.available_width(), 30.0));
            if ui.add(export).clicked() {
                self.pick_export();
            }
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(format!("{} objects · {} verts · {} faces", self.objects.len(),
                    self.objects.iter().map(|o| o.mesh.positions.len()).sum::<usize>(),
                    self.objects.iter().map(|o| o.mesh.faces.len()).sum::<usize>())).size(10.5).color(MUTED));
            });
            ui.add_space(2.0);
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);
            card(ui, "OBJECTS", |ui| self.objects_card(ui));
            card(ui, "SELECT", |ui| self.select_card(ui));
            card(ui, "ADD MESH", |ui| self.add_card(ui));
            card(ui, "TRANSFORM", |ui| self.transform_card(ui));
            card(ui, "MESH", |ui| self.mesh_card(ui));
            card(ui, "UV MAPPING", |ui| self.uv_card(ui));
            card(ui, "TEXTURE", |ui| self.texture_card(ui, &ctx));
        });
    }

    fn objects_card(&mut self, ui: &mut egui::Ui) {
        if self.objects.is_empty() {
            ui.label(RichText::new("Empty scene. Add a mesh below.").color(MUTED));
            return;
        }
        let object_mode = self.sel_mode == SelMode::Object;

        // Group by prefix: "Doodad_Barrel_3" belongs to "Doodad" when at least one other object does too.
        let mut members: std::collections::BTreeMap<&str, Vec<usize>> = std::collections::BTreeMap::new();
        for (i, obj) in self.objects.iter().enumerate() {
            if let Some(prefix) = group_prefix(&obj.name) {
                members.entry(prefix).or_default().push(i);
            }
        }
        members.retain(|_, m| m.len() >= 2);
        // Rows in scene order; a group appears where its first member is.
        let mut rows: Vec<ListRow> = Vec::new();
        let mut emitted: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (i, obj) in self.objects.iter().enumerate() {
            match group_prefix(&obj.name).filter(|p| members.contains_key(p)) {
                Some(prefix) => {
                    if emitted.insert(prefix) {
                        rows.push(ListRow::Group(prefix.to_string()));
                        if self.expanded_groups.contains(prefix) {
                            rows.extend(members[prefix].iter().map(|&m| ListRow::Object(m, true)));
                        }
                    }
                }
                None => rows.push(ListRow::Object(i, false)),
            }
        }

        enum Click {
            Object(usize),
            Group(String),
            ToggleGroup(String),
            Eye(Vec<usize>),
        }
        let mut click = None;
        let row_h = 21.0;
        // Only the rows in view are laid out, so thousands of objects cost nothing.
        ui.spacing_mut().item_spacing.y = 1.0;
        egui::ScrollArea::vertical().id_salt("object-list").max_height(232.0).auto_shrink([false, true]).show_rows(ui, row_h, rows.len(), |ui, range| {
            for row in &rows[range] {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::click());
                let x_in = resp.interact_pointer_pos().map(|p| p.x - rect.left());
                let painter = ui.painter().with_clip_rect(rect);
                let mid = rect.center().y;
                match row {
                    ListRow::Group(name) => {
                        let ids = &members[name.as_str()];
                        let open = self.expanded_groups.contains(name);
                        let selected = if object_mode { ids.iter().filter(|i| self.sel_objects.contains(i)).count() } else { 0 };
                        let all_selected = object_mode && selected == ids.len();
                        let fill = if all_selected { Color32::from_rgb(120, 72, 22) } else if resp.hovered() { TILE_HOVER } else { TILE };
                        painter.rect_filled(rect, 4.0, fill);
                        // Fold arrow.
                        let c = egui::pos2(rect.left() + 10.0, mid);
                        let tri = if open {
                            vec![c + egui::vec2(-4.0, -2.5), c + egui::vec2(4.0, -2.5), c + egui::vec2(0.0, 3.5)]
                        } else {
                            vec![c + egui::vec2(-2.5, -4.0), c + egui::vec2(3.5, 0.0), c + egui::vec2(-2.5, 4.0)]
                        };
                        painter.add(egui::Shape::convex_polygon(tri, Color32::from_gray(200), Stroke::NONE));
                        let visible = ids.iter().any(|&i| self.objects[i].visible);
                        let eye = if visible { Icon::EyeOpen } else { Icon::EyeClosed };
                        painter.image(self.icon(eye), Rect::from_min_size(egui::pos2(rect.left() + 21.0, mid - 8.0), egui::vec2(16.0, 16.0)), Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                        let note = if selected > 0 && !all_selected { format!("{selected} of {}", ids.len()) } else { format!("{}", ids.len()) };
                        painter.text(egui::pos2(rect.right() - 6.0, mid), egui::Align2::RIGHT_CENTER, note, egui::FontId::proportional(10.5), if selected > 0 { ACCENT } else { MUTED });
                        painter.with_clip_rect(rect.with_max_x(rect.right() - 48.0)).text(
                            egui::pos2(rect.left() + 42.0, mid), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), Color32::from_gray(235));
                        if resp.clicked() {
                            click = Some(match x_in {
                                Some(x) if x < 19.0 => Click::ToggleGroup(name.clone()),
                                Some(x) if x < 40.0 => Click::Eye(ids.clone()),
                                _ => Click::Group(name.clone()),
                            });
                        } else if resp.double_clicked() {
                            click = Some(Click::ToggleGroup(name.clone()));
                        }
                        resp.on_hover_text("Click the arrow to unfold  ·  click the name to select the whole group  ·  the eye shows or hides it");
                    }
                    ListRow::Object(i, indented) => {
                        let obj = &self.objects[*i];
                        let indent = if *indented { 14.0 } else { 0.0 };
                        // In edit modes only the object being edited is highlighted.
                        let selected = if object_mode { self.sel_objects.contains(i) } else { *i == self.active };
                        if selected {
                            painter.rect_filled(rect, 4.0, Color32::from_rgb(150, 88, 24));
                        } else if resp.hovered() {
                            painter.rect_filled(rect, 4.0, TILE_HOVER);
                        }
                        let eye = if obj.visible { Icon::EyeOpen } else { Icon::EyeClosed };
                        painter.image(self.icon(eye), Rect::from_min_size(egui::pos2(rect.left() + 3.0 + indent, mid - 8.0), egui::vec2(16.0, 16.0)), Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                        painter.text(egui::pos2(rect.right() - 6.0, mid), egui::Align2::RIGHT_CENTER, format!("{} faces", obj.mesh.faces.len()), egui::FontId::proportional(10.5), if selected { Color32::from_rgb(255, 226, 180) } else { MUTED });
                        let colour = if *i == self.active { Color32::WHITE } else { Color32::from_gray(210) };
                        // Long names are cut off at the face count instead of pushing the panel wider.
                        painter.with_clip_rect(rect.with_max_x(rect.right() - 58.0)).text(
                            egui::pos2(rect.left() + 24.0 + indent, mid), egui::Align2::LEFT_CENTER, &obj.name, egui::FontId::proportional(13.0), colour);
                        if resp.clicked() {
                            click = Some(match x_in {
                                Some(x) if x < 22.0 + indent => Click::Eye(vec![*i]),
                                _ => Click::Object(*i),
                            });
                        }
                        if resp.hovered() && obj.name.len() > 18 {
                            resp.on_hover_text(&obj.name);
                        }
                    }
                }
            }
        });
        ui.spacing_mut().item_spacing.y = 5.0;

        let extend = ui.input(|inp| inp.modifiers.shift || inp.modifiers.command);
        match click {
            Some(Click::Object(i)) => {
                self.set_mode(SelMode::Object);
                self.select_object(i, extend);
            }
            Some(Click::Group(name)) => {
                let ids: Vec<usize> = self.objects.iter().enumerate().filter(|(_, o)| group_prefix(&o.name) == Some(name.as_str())).map(|(i, _)| i).collect();
                self.set_mode(SelMode::Object);
                if !extend {
                    self.sel_objects.clear();
                }
                self.sel_objects.extend(&ids);
                if let Some(&first) = ids.first() {
                    self.set_active(first);
                }
            }
            Some(Click::ToggleGroup(name)) => {
                if !self.expanded_groups.remove(&name) {
                    self.expanded_groups.insert(name);
                }
            }
            Some(Click::Eye(ids)) => {
                // Hide if anything is showing, otherwise show.
                let show = !ids.iter().any(|&i| self.objects[i].visible);
                for i in ids {
                    self.objects[i].visible = show;
                }
            }
            None => {}
        }

        let active = self.active;
        if let Some(obj) = self.objects.get_mut(active) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Name").color(MUTED));
                ui.add(egui::TextEdit::singleline(&mut obj.name).desired_width(f32::INFINITY))
                    .on_hover_text("Objects whose names share the part before the first underscore are grouped: Doodad_Barrel and Doodad_Crate both go under \"Doodad\"");
            });
        }
        ui.horizontal(|ui| {
            if ui.small_button("Duplicate").clicked() {
                self.set_mode(SelMode::Object);
                if self.duplicate_selection() {
                    self.start_modal_no_undo(ModalKind::Grab, None);
                }
            }
            if ui.small_button("Delete").clicked() {
                self.set_mode(SelMode::Object);
                self.delete_selection();
            }
            let can_merge = object_mode && self.sel_objects.len() >= 2;
            if ui.add_enabled(can_merge, egui::Button::new("Merge").small())
                .on_hover_text("Join the selected objects into the active one  ·  Ctrl+J")
                .on_disabled_hover_text("Select two or more objects first (Shift+click)")
                .clicked()
            {
                self.merge_selected();
            }
        });
    }

    fn select_card(&mut self, ui: &mut egui::Ui) {
        let w = tile_width(ui, 4);
        let mut mode = None;
        row(ui, |ui| {
            for (m, icon, label, tip) in [
                (SelMode::Object, Icon::Object, "Object", "Whole objects  (O, or Tab to toggle)"),
                (SelMode::Vertex, Icon::Vertex, "Vertex", "Edit vertices  (V)"),
                (SelMode::Edge, Icon::Edge, "Edge", "Edit edges  (D)"),
                (SelMode::Face, Icon::Face, "Face", "Edit faces  (F)"),
            ] {
                if self.tile(ui, icon, label, self.sel_mode == m, w).on_hover_text(tip).clicked() {
                    mode = Some(m);
                }
            }
        });
        if let Some(m) = mode {
            self.set_mode(m);
        }
        let w = tile_width(ui, 3);
        row(ui, |ui| {
            for (t, icon, label, tip) in [
                (SelectTool::Click, Icon::Click, "Click", "Click to select  (Q)"),
                (SelectTool::Box, Icon::BoxSelect, "Box", "Drag a box  (B)"),
                (SelectTool::Circle, Icon::CircleSelect, "Circle", "Paint with a circle  (C)"),
            ] {
                if self.tile(ui, icon, label, self.select_tool == t, w).on_hover_text(tip).clicked() {
                    self.select_tool = t;
                }
            }
        });
        if self.select_tool == SelectTool::Circle {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Radius").color(MUTED));
                ui.add(egui::DragValue::new(&mut self.circle_radius).range(5.0..=300.0).suffix(" px")).on_hover_text("Also [ and ] in the viewport");
            });
        }
        if self.select_tool != SelectTool::Click {
            ui.checkbox(&mut self.only_visible, "Only visible")
                .on_hover_text("Box / circle skip what is hidden behind geometry. Untick to select straight through.");
        }
        ui.horizontal(|ui| {
            if ui.small_button("All").on_hover_text("A").clicked() {
                self.select_all();
            }
            if ui.small_button("None").on_hover_text("Alt+A").clicked() {
                self.deselect_all();
            }
            if ui.small_button("Linked").on_hover_text("Everything connected to the selection  (L)").clicked() {
                self.select_linked();
            }
            ui.label(RichText::new("Alt+click: loop").size(10.5).color(MUTED)).on_hover_text("Alt+click or double-click an edge to select its whole loop");
        });
    }

    fn add_card(&mut self, ui: &mut egui::Ui) {
        let w = tile_width(ui, 5);
        let seg = self.segments;
        let mut add: Option<(&str, Mesh)> = None;
        row(ui, |ui| {
            if self.tile(ui, Icon::Plane, "Plane", false, w).clicked() {
                add = Some(("Plane", Mesh::plane()));
            }
            if self.tile(ui, Icon::Cube, "Cube", false, w).clicked() {
                add = Some(("Cube", Mesh::cube()));
            }
            if self.tile(ui, Icon::Cylinder, "Cylinder", false, w).clicked() {
                add = Some(("Cylinder", Mesh::cylinder(seg, 0.5, 0.5, 1.0)));
            }
            if self.tile(ui, Icon::Cone, "Cone", false, w).clicked() {
                add = Some(("Cone", Mesh::cylinder(seg, 0.5, 0.0, 1.0)));
            }
            if self.tile(ui, Icon::Sphere, "Sphere", false, w).clicked() {
                add = Some(("Sphere", Mesh::sphere(seg, (seg / 2).max(3), 0.5)));
            }
        });
        if let Some((name, mesh)) = add {
            self.add_primitive(name, mesh);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Segments").color(MUTED));
            ui.add(egui::DragValue::new(&mut self.segments).range(3..=64)).on_hover_text("Sides of new cylinders, cones and spheres");
        });
    }

    fn transform_card(&mut self, ui: &mut egui::Ui) {
        let w = tile_width(ui, 5);
        let object_mode = self.sel_mode == SelMode::Object;
        let mut start = None;
        row(ui, |ui| {
            for (kind, icon, label, tip) in [
                (ModalKind::Grab, Icon::Move, "Move", "Move  (G)  ·  or drag an arrow in the view"),
                (ModalKind::Rotate, Icon::Rotate, "Rotate", "Rotate  (R)  ·  objects turn about their origin"),
                (ModalKind::Scale, Icon::Scale, "Scale", "Scale  (S)"),
            ] {
                if self.tile(ui, icon, label, false, w).on_hover_text(tip).clicked() {
                    start = Some(kind);
                }
            }
            if self.tile(ui, Icon::Snap, "Snap", self.snap_on, w)
                .on_hover_text("Move in steps of the distance below. Holding Ctrl flips this while you drag.")
                .clicked()
            {
                self.snap_on = !self.snap_on;
            }
            ui.add_enabled_ui(object_mode, |ui| {
                if self.tile(ui, Icon::Origin, "Origin", self.origin_only && object_mode, w)
                    .on_hover_text("Edit the origin: while this is on, moving (arrows, G, the Pos fields) shifts only the object's origin and leaves the geometry where it is.")
                    .on_disabled_hover_text("Switch to object mode to move origins. The button below works in edit modes.")
                    .clicked()
                {
                    self.origin_only = !self.origin_only;
                }
            });
        });
        if let Some(kind) = start {
            self.start_modal(kind, None);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Snap step").color(MUTED));
            ui.add(egui::DragValue::new(&mut self.grid_snap).speed(0.0125).range(0.001..=10.0).max_decimals(4));
        });
        self.position_fields(ui);
        if !self.objects.is_empty() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new("Origin to").color(MUTED));
                if object_mode {
                    if ui.small_button("Centre").on_hover_text("Middle of the object's bounding box").clicked() {
                        self.set_origin(OriginTo::Centre);
                    }
                    if ui.small_button("Base").on_hover_text("Bottom centre: handy for props that stand on the ground").clicked() {
                        self.set_origin(OriginTo::Base);
                    }
                    if ui.small_button("World 0").on_hover_text("The scene's zero point").clicked() {
                        self.set_origin(OriginTo::World);
                    }
                } else if ui.small_button("Selection").on_hover_text("Middle of the selected vertices, edges or faces").clicked() {
                    self.set_origin(OriginTo::Selection);
                }
            });
        }
    }

    fn mesh_card(&mut self, ui: &mut egui::Ui) {
        let w = tile_width(ui, 5);
        #[derive(Clone, Copy)]
        enum Act { Extrude, Inset, Bevel, LoopCut, Duplicate, Delete, Flip, MergeFaces, Weld }
        let mut act = None;
        let items = [
            (Act::Extrude, Icon::Extrude, "Extrude", "Pull the selected faces out  (E)"),
            (Act::Inset, Icon::Inset, "Inset", "Shrink the selected faces inwards, leaving a rim  (I)"),
            (Act::Bevel, Icon::Bevel, "Bevel", "Chamfer the selected edges  (Ctrl+B)"),
            (Act::LoopCut, Icon::LoopCut, "Loop cut", "Cut a ring of faces in two  (Ctrl+R)"),
            (Act::MergeFaces, Icon::MergeFaces, "Join", "Merge adjoining selected faces into one  (M)"),
            (Act::Duplicate, Icon::Duplicate, "Copy", "Duplicate the selection and move it  (Shift+D)"),
            (Act::Delete, Icon::Delete, "Delete", "Delete the selection  (X)"),
            (Act::Flip, Icon::Flip, "Flip", "Flip the selected faces' normals"),
            (Act::Weld, Icon::Weld, "Weld", "Merge vertices that sit on top of each other"),
        ];
        for chunk in items.chunks(5) {
            row(ui, |ui| {
                for &(a, icon, label, tip) in chunk {
                    if self.tile(ui, icon, label, false, w).on_hover_text(tip).clicked() {
                        act = Some(a);
                    }
                }
            });
        }
        match act {
            Some(Act::Extrude) => self.extrude(),
            Some(Act::Inset) => self.start_inset(),
            Some(Act::Bevel) => self.start_bevel(),
            Some(Act::LoopCut) => self.start_loop_cut(),
            Some(Act::Duplicate) => {
                if self.duplicate_selection() {
                    self.start_modal_no_undo(ModalKind::Grab, None);
                }
            }
            Some(Act::Delete) => self.delete_selection(),
            Some(Act::Flip) => {
                let faces = self.active_faces();
                if !faces.is_empty() {
                    self.push_undo();
                    self.mesh_mut().flip_faces(&faces);
                }
            }
            Some(Act::MergeFaces) => self.merge_selected_faces(),
            Some(Act::Weld) => {
                self.push_undo();
                let n = self.mesh_mut().merge_by_distance(1e-4);
                self.clear_selection();
                self.status = format!("Welded {n} vertices");
            }
            None => {}
        }
    }

    fn uv_card(&mut self, ui: &mut egui::Ui) {
        let w = tile_width(ui, 6);
        let tex = self.tex_size();
        #[derive(Clone, Copy)]
        enum Act { Fit, Box, View, Rotate, FlipU, FlipV }
        let mut act = None;
        let items = [
            (Act::Fit, Icon::UvFit, "Fit", "Stretch every selected face over the whole texture"),
            (Act::Box, Icon::UvBox, "Box", "Project along the nearest axis at the pixel density below"),
            (Act::View, Icon::UvView, "View", "Project the selected faces as seen in the 3D view"),
            (Act::Rotate, Icon::UvRotate, "Rotate", "Rotate the UVs by 90 degrees"),
            (Act::FlipU, Icon::FlipU, "Flip U", "Mirror the UVs left to right"),
            (Act::FlipV, Icon::FlipV, "Flip V", "Mirror the UVs top to bottom"),
        ];
        for chunk in items.chunks(6) {
            row(ui, |ui| {
                for &(a, icon, label, tip) in chunk {
                    if self.tile(ui, icon, label, false, w).on_hover_text(tip).clicked() {
                        act = Some(a);
                    }
                }
            });
        }
        match act {
            Some(Act::Fit) => self.uv_op(|m, f| m.uv_fit_each(f)),
            Some(Act::Box) => {
                let ppu = self.px_per_unit;
                self.uv_op(|m, f| m.uv_box_project(f, ppu, tex));
            }
            Some(Act::View) => {
                let view = self.camera.view();
                self.uv_op(|m, f| m.uv_project_view(f, view));
            }
            Some(Act::Rotate) => self.uv_op(|m, f| m.uv_transform(f, UvOp::Rotate90)),
            Some(Act::FlipU) => self.uv_op(|m, f| m.uv_transform(f, UvOp::FlipU)),
            Some(Act::FlipV) => self.uv_op(|m, f| m.uv_transform(f, UvOp::FlipV)),
            None => {}
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Pixels per unit").color(MUTED));
            ui.add(egui::DragValue::new(&mut self.px_per_unit).speed(0.25).range(1.0..=512.0)).on_hover_text("Texel density used by Box");
        });
        ui.checkbox(&mut self.uv.show_all, "Show unselected faces");
    }

    fn texture_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let w = tile_width(ui, 4);
        let has = self.texture.is_some();
        let (mut new, mut load, mut save, mut remove) = (false, false, false, false);
        row(ui, |ui| {
            new = self.tile(ui, Icon::New, "New", false, w).on_hover_text("Start a blank canvas in the Paint tab").clicked();
            load = self.tile(ui, Icon::Load, "Load", false, w).on_hover_text("Open an image (PNG, JPG, BMP, TGA)").clicked();
            ui.add_enabled_ui(has, |ui| {
                save = self.tile(ui, Icon::Save, "Save", false, w).on_hover_text("Save the image as PNG  (Ctrl+S)").clicked();
                remove = self.tile(ui, Icon::Delete, "Remove", false, w).on_hover_text("Detach the texture from the scene").clicked();
            });
        });
        if new {
            self.open_new_canvas();
        }
        if load {
            self.pick_texture(ctx);
        }
        if save {
            self.save_texture();
        }
        if remove {
            self.texture = None;
        }
        match &self.texture {
            Some(t) => {
                let name = t.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(format!("{name}{}  {}×{}", if t.dirty { " *" } else { "" }, t.size[0], t.size[1]));
                ui.label(RichText::new(if t.dirty { "Unsaved paint changes (Ctrl+S)." } else { "Reloads when the file changes." }).size(10.5).color(MUTED));
            }
            None => {
                ui.label(RichText::new("No texture (64×64 grid assumed).").size(10.5).color(MUTED));
            }
        }
    }
}
