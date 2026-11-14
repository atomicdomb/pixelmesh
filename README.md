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
| Left   | UV editor (toggle with the "UV editor" button in the menu bar, or U) |
| Centre | 3D viewport |
| Right  | Object list, modes, add meshes, transform, mesh ops, UV mapping, texture, export |

## Objects and modes

The scene is a list of named objects, shown at the top of the right panel.
Click a name (or click the object in the viewport) to select it; the box
next to it hides or shows it.

- **Object mode**: transforms move the whole selected object. Adding a mesh
  creates a new object.
- **Vertex / Face mode**: edit the selected object. Adding a mesh adds it
  to that object.

Tab switches between object mode and the last edit mode.

In object mode several objects can be selected at once (Shift+click in the
list or viewport, or box / circle select); they then move, duplicate and
delete together. **Merge** (button in the object list, or Ctrl+J) joins all
selected objects into the active one, shown in bold.

## Selection tools

Pick the tool under "Mode"; all three work on objects, vertices and faces.

| Tool | How it selects |
|------|----------------|
| Click (Q) | Click one item; Shift+click adds or removes |
| Box (B) | Drag a rectangle |
| Circle (C) | Paint with a circle around the cursor; `[` and `]` change its radius |

For box and circle: a plain drag starts a new selection, Shift adds to the
current one, Ctrl removes from it. "Only visible" skips vertices and faces
hidden behind geometry; untick it to select straight through the mesh.

## Moving things

- **Arrows**: drag the red / green / blue arrow to move the selection along
  X / Y / Z. Works for objects, faces and vertices.
- **Snapping**: tick "Snap every" and set the distance; moves then jump in
  steps of that distance. Holding Ctrl flips the setting while dragging.
  (Rotation snaps to 15 degrees, scale to 0.1.)
- **Keyboard**: G / R / S as below.

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
| Tab, O / V / F | Toggle object-edit, object / vertex / face mode |
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

Select an object (all its faces) or individual faces in face mode, then:

- drag a corner to move it (corners sharing the spot move together),
- drag inside a face to move all selected faces as one island,
- scroll to zoom, right- or middle-drag to pan.

With "Snap UVs to pixels" on, everything lands on texel boundaries.
"Box project" unwraps at a fixed texel density ("Pixels per unit"), so a
1-unit face covers exactly e.g. 16x16 pixels. The texture reloads
automatically when the PNG changes on disk.

## FBX import

File > Import FBX adds every mesh in the file to the scene as its own
object (or pass the file on the command line: `pixelmesh model.fbx`).
Binary and ASCII FBX are both read.

- Object transforms, the file's up axis and its unit are baked in, so
  models arrive Y-up and in metres whatever program wrote them.
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
share one material with the texture on the diffuse slot. Object positions
are baked into the vertices, so every model's pivot is the scene origin. The
texture is referenced by file name (a copy is placed next to the .fbx if
missing) or embedded if "Embed texture in FBX" is ticked.

`cargo test` round-trips exports through the ufbx parser in strict mode and
imports FBX files written by Blender (`tests/fixtures`).

## Code map

- `src/mesh.rs` - mesh data, primitives, edit and UV operations (unit tested)
- `src/fbx.rs` - FBX writer
- `src/fbx_import.rs` - FBX reader (via ufbx)
- `src/app.rs` - state, menus, tool panel, files
- `src/viewport.rs` - 3D view, picking, modal transforms
- `src/uv_editor.rs` - UV editor
- `src/render.rs`, `src/camera.rs` - OpenGL drawing, orbit camera
