# PixelMesh

A deliberately small low-poly mesh editor: basic meshes, basic editing, a
pixel-snapping UV editor for pixel-art textures, and binary FBX export.

## Build and run

    cargo run --release
    cargo run --release -- model.pmesh sheet.png   # open a project / texture directly

Needs a recent stable Rust, a C compiler (for the bundled FBX parser) and
OpenGL 3.3. On Linux, file dialogs use the
XDG desktop portal.

## Layout

| Area   | What it does |
|--------|--------------|
| Left   | UV editor and Paint tabs (hide with the "UV / Paint panel" button in the menu bar, or U) |
| Centre | 3D viewport |
| Right  | Object list, modes, add meshes, transform, mesh ops, UV mapping, texture, export |

## Objects and modes

The scene is a list of named objects, shown at the top of the right panel.
Click a name (or click the object in the viewport) to select it; the box
next to it hides or shows it.

- **Object mode**: transforms move the whole selected object. Adding a mesh
  creates a new object.
- **Vertex / Edge / Face mode**: edit the selected object. Adding a mesh
  adds it to that object.

Tab switches between object mode and the last edit mode.

In object mode several objects can be selected at once (Shift+click in the
list or viewport, or box / circle select); they then move, duplicate and
delete together. **Merge** (button in the object list, or Ctrl+J) joins all
selected objects into the active one, shown in bold.

## Selection tools

Pick the tool in the Select card; all three work on objects, vertices,
edges and faces.

**Loops**: Alt+click or double-click selects a whole loop: the edge loop
through the edge under the cursor (edge and vertex mode), or the band of
faces across it (face mode). Shift adds to the selection.

**Right-click** in the 3D view opens a menu for the current selection:
delete, merge faces, extrude, inset, bevel, loop cut, duplicate, flip.
Right-clicking a face or object that is not selected selects it first.
(Right-*drag* still orbits.)

| Tool | How it selects |
|------|----------------|
| Click (Q) | Click one item; Shift+click adds or removes |
| Box (B) | Drag a rectangle |
| Circle (C) | Paint with a circle around the cursor; `[` and `]` change its radius |

For box and circle: a plain drag starts a new selection, Shift adds to the
current one, Ctrl removes from it. "Only visible" skips vertices and faces
hidden behind geometry; untick it to select straight through the mesh.

## Origin and rotation

Every object has an **origin** (its pivot, drawn as an orange ring) and a
recorded **rotation**. In object mode the Transform card shows both:

- **Pos**: position of the origin. Editing it moves the object.
- **Rot**: rotation in degrees about the origin (applied X, then Y, then
  Z). Type a value or drag the field; set it back to 0 to undo a rotation.
  Rotating with R updates these numbers too.
- **Origin tile**: while it is on, moving (arrows, G, or the fields, now
  labelled "Origin") shifts only the origin and leaves the geometry alone.
- **Origin to**: Centre (middle of the bounding box), Base (bottom centre),
  World 0, or, in an edit mode, Selection (middle of what is selected).

A single object rotates and scales about its origin; several selected
objects turn about the middle of the group. Editing vertices, edges or
faces never moves the origin. New primitives start with the origin at the
centre of their base.

Origin and rotation are written to FBX as each model's position and
rotation, and read back on import.

## Moving things

- **Arrows**: drag the red / green / blue arrow to move the selection along
  X / Y / Z. Works for objects, faces and vertices.
- **Snapping**: tick "Snap every" and set the distance; moves then jump in
  steps of that distance. Holding Ctrl flips the setting while dragging.
  (Rotation snaps to 15 degrees, scale to 0.1.)
- **Keyboard**: G / R / S as below.

## Merge faces

Select two or more adjoining faces and choose "Join" in the Mesh card,
"Merge faces" in the right-click menu, or press M. Each connected group
becomes one polygon and keeps its texture mapping. Vertices on the outline
stay if a neighbouring face still uses them. A group is skipped when its
outline is not a single loop (for example a ring of faces round a hole).

## Inset, bevel, loop cut

All three work on the object being edited (Tab into vertex / face mode first).

- **Inset (I)**: shrinks the selected faces inwards and leaves a rim of
  new faces. Several selected faces inset together as one region.
- **Bevel (Ctrl+B)**: chamfers edges. In face mode it takes every edge of
  the selected faces; in vertex mode every edge whose two ends are selected.
- **Loop cut (Ctrl+R)**: hover an edge and a yellow preview shows the cut
  running round the ring of quads. Press 1-9 for the number of cuts, click
  to place. A single cut can then be slid along the edge; click again to
  confirm. The new loop is left selected in vertex mode.

Inset and bevel are sized by moving the mouse to the right; click or Enter
confirms, Esc or right-click cancels. Snapping ("Snap every" / Ctrl) applies
to the inset and bevel width and to the loop-cut slide distance. Existing
faces keep their texture exactly in place through all three.

## 3D viewport

| Input | Action |
|-------|--------|
| Click / Shift+click | Select / add to selection |
| Drag an arrow | Move along that axis |
| Middle- or right-drag (or Alt+left-drag) | Orbit; with Shift: pan |
| Scroll | Zoom |
| Tab, O / V / D / F | Toggle object-edit, object / vertex / edge / face mode |
| Alt+click or double-click | Select a loop |
| Right-click | Context menu |
| M | Merge selected faces |
| Q / B / C | Click / box / circle select tool |
| Ctrl+J | Merge selected objects |
| A / Alt+A / L | Select all / none / linked (edit modes) |
| G / R / S | Move / rotate / scale. Then X, Y, Z to lock an axis, click or Enter to confirm, Esc or right-click to cancel |
| E | Extrude selected faces along their normal |
| I / Ctrl+B / Ctrl+R | Inset / bevel / loop cut |
| Shift+D | Duplicate (object or faces) |
| X or Delete | Delete (object or faces / vertices) |
| U | Show / hide the UV editor |
| 1 / 3 / 7 / 5 / Home | Front / right / top / toggle orthographic / frame all |
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo |

## UV editor

The UV editor shows the faces chosen in the 3D view: the whole selected
object in object mode, or the selected faces in face mode. Inside it you
have a separate UV selection.

| Input | Action |
|-------|--------|
| V / F (or the buttons) | Select UV vertices / whole faces |
| Q / B / C (or the buttons) | Click / box / circle select; Shift adds, Ctrl removes |
| A / Alt+A | Select all / none |
| Drag inside the selection frame, or drag any selected vertex / face | Move the whole selection as a group |
| Drag a handle on the selection frame | Scale; the opposite side stays put. Shift keeps proportions |
| G | Move with the mouse; X / Y lock an axis; click confirms, Esc cancels |
| S | Scale about the centre; X / Y lock an axis |
| Scroll, right- or middle-drag | Zoom, pan |

With two or more vertices selected a frame is drawn around them, slightly
outside so its handles never cover a vertex. A plain drag inside the frame
moves the group even with the box or circle tool active; hold Shift or Ctrl
to box / circle select inside it instead.

UV vertices that share a position are picked together, so stitched faces
stay stitched. In click mode, dragging something unselected selects and
moves it in one go.

**Snap to pixels** (on by default): moves land on pixel corners, handles
snap to the pixel grid, and S snaps the selection's size to whole pixels.
The readout at the bottom shows the move or size in pixels. "Snap
selection now" rounds the selected UVs to the nearest pixel corners.

"Box project" (right panel) unwraps at a fixed texel density ("Pixels per
unit"), so a 1-unit face covers exactly e.g. 16x16 pixels. The texture
reloads automatically when the image changes on disk.

## Paint tab (pixel editor)

The left panel has two tabs, "UV editor" and "Paint". Paint edits the
texture itself; every stroke shows on the model immediately. Both tabs
share the same zoom and pan.

- **New canvas**: "New…" (or File > New image) asks for a size, with
  presets such as 32x32 and 256x128, and a fill (white, transparent, or the
  background colour). Non-square canvases are shown with square pixels.
- **Save**: "Save" or Ctrl+S writes a PNG ("Save as…" for a new canvas or
  a new name). A `*` marks unsaved changes. Exporting an FBX always uses
  the pixels as painted, saved or not.
- **UV overlay**: toggles the UV outlines of every visible object over the
  image, with the faces shown in the UV tab highlighted, so you can paint
  right up to an edge.

| Tool | Key | What it does |
|------|-----|--------------|
| Pencil | B | Left button paints the foreground colour, right button the background colour |
| Eraser | E | Makes pixels transparent |
| Fill | G | Fills the connected area of one colour |
| Picker | I | Picks a colour (left: foreground, right: background). Alt with any tool does the same |
| Line | L | Drag for a straight line |
| Rect | U | Drag for a rectangle; "Filled rect" fills it |
| Select | M | Drag a rectangular selection. Shift adds, Ctrl removes, a single click deselects |
| Wand | W | Selects the connected area of one colour. Shift adds, Ctrl removes |

**Selections**: while pixels are selected (dashed outline), every paint
tool only changes those pixels: strokes are clipped to the selection and
Fill stops at its edge. The "Selection" row has All (Ctrl+A), None (Ctrl+D
or Esc), Invert (Ctrl+Shift+I) and **From UV faces**, which selects the
pixels covered by the faces currently shown in the UV tab, so you can paint
one face of the model without spilling onto its neighbours. Delete makes
the selected pixels transparent.

Other controls: `[` / `]` brush size, X swaps the two colours, scroll
zooms, middle-drag or Space+drag pans, Ctrl+Z / Ctrl+Shift+Z undo and redo
strokes (one shared history with mesh edits). The palette row holds 16
starter colours followed by the colours already used in the image; left-
click sets the foreground, right-click the background.

Transparent pixels are cut out on the model and in the exported texture.

## FBX import

File > Import FBX adds every mesh in the file to the scene as its own
object (or pass the file on the command line: `pixelmesh model.fbx`).
Binary and ASCII FBX are both read.

- The file's up axis and unit are converted, so models arrive Y-up and in
  metres whatever program wrote them. Each object keeps its pivot as its
  origin and its orientation as its rotation; scale is applied to the
  vertices. (Files from Z-up programs such as Blender usually carry a 90
  degree X rotation on every object; it shows up in the Rot fields.)
- Polygons (triangles, quads, n-gons) and the first UV set are kept.
  Faces without UVs get a default "fit to texture" mapping.
- If no texture is loaded yet, the file's diffuse / base-colour texture
  is used: embedded images directly, otherwise the referenced file if it
  can be found next to the FBX.
- Not imported: normals (the editor is flat-shaded), extra UV sets, vertex
  colours, per-face material assignments, rigs and animation.

## FBX export

File > Export FBX writes binary FBX 7.4: Y-up, 1 unit = 1 metre. Every
object becomes its own named model with flat normals and one UV set; all
share one material with the texture on the diffuse slot. Each model's
pivot and rotation are the object's origin and rotation. The
texture is referenced by file name (a copy is placed next to the .fbx if
missing) or embedded if "Embed texture in FBX" is ticked.

`cargo test` round-trips exports through the ufbx parser in strict mode and
imports FBX files written by Blender (`tests/fixtures`).

## Code map

- `src/mesh.rs` - mesh data, primitives, edit and UV operations (unit tested)
- `src/fbx.rs` - FBX writer
- `src/fbx_import.rs` - FBX reader (via ufbx)
- `src/app.rs` - state, menus, files
- `src/sidebar.rs`, `src/icons.rs` - tool panel, theme and icons

## Editing the icons

The interface icons live in `assets/icons.png`: a grid of 16x16 cells, 11
per row, compiled into the program. Edit that file in any pixel editor
(including this one's Paint tab), keep each icon inside its cell, and run
`cargo run` again to see the result. The order of the cells is the `ALL`
list in `src/icons.rs`; new icons are always added at the end, so existing
cells never move.

`cargo run --example icon_sheet -- preview.png` writes an enlarged preview
of the built-in drawings. Adding `--atlas` and pointing it at
`assets/icons.png` resets the file to those built-in drawings, overwriting
any edits.
- `src/viewport.rs` - 3D view, picking, modal transforms
- `src/uv_editor.rs` - UV editor
- `src/paint.rs`, `src/pixels.rs` - paint tab and its pixel operations (unit tested)
- `src/render.rs`, `src/camera.rs` - OpenGL drawing, orbit camera
