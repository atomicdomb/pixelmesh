# PixelMesh

A deliberately small low-poly mesh editor: basic meshes, basic editing, a
pixel-snapping UV editor for pixel-art textures, and binary FBX export.

## Build and run

    cargo run --release
    cargo run --release -- model.pmesh sheet.png   # open a project / texture directly

Needs a recent stable Rust, a C compiler (for the bundled FBX parser) and
OpenGL 3.3. On Linux, file dialogs use the
XDG desktop portal.

## Building a Mac app

`scripts/macos-package.sh` builds `dist/PixelMesh.app` (Apple Silicon and
Intel in one binary, with the icon from `assets/app-icon.png`) and a zip of
it. It needs the Xcode command line tools.

**Just for this Mac** (ad-hoc signature):

    scripts/macos-package.sh

**For other Macs** (Developer ID signature, notarized by Apple, so it opens
with a normal double-click). One-time setup:

1. In Xcode > Settings > Accounts > Manage Certificates, add a
   "Developer ID Application" certificate. Find its exact name with
   `security find-identity -v -p codesigning`.
2. Create an app-specific password at account.apple.com, then store it:
   `xcrun notarytool store-credentials notary --apple-id YOU@EXAMPLE.COM --team-id TEAMID`

Then each time:

    SIGN_ID="Developer ID Application: Your Name (TEAMID)" NOTARY_PROFILE=notary scripts/macos-package.sh

Send the zip from `dist/`. The bundle identifier defaults to
`studio.butterflyknights.pixelmesh`; set `BUNDLE_ID` to use another.

### Opening files from Finder

The packaged app registers itself for its file types: double-clicking a
`.pmesh` project opens it in PixelMesh, and `.fbx` files (and PNG / JPEG /
BMP / TGA images) list PixelMesh under right-click > Open With. Dropping a
file on the Dock icon works too, also while PixelMesh is already running.
An FBX opened into the untouched start-up scene replaces the default cube;
otherwise it is imported into the current scene. To make PixelMesh the
default for FBX, use Finder's Get Info > Open with > Change All.

This only applies to the app bundle made by `scripts/macos-package.sh`; a
bare `cargo run` binary has no file types. After installing a new build,
macOS can take a moment to notice the file types; opening the app once
from its final location (for example /Applications) registers them.

## Layout

| Area   | What it does |
|--------|--------------|
| Left   | UV editor and Paint tabs (hide with the "UV / Paint panel" button in the menu bar, or U) |
| Centre | 3D viewport |
| Right  | Object list, modes, add meshes, transform, modifiers, mesh ops, UV mapping, texture, reference image, export |

## Objects and modes

The scene is a list of named objects, shown at the top of the right panel.
Click a name (or click the object in the viewport) to select it; the eye
next to it hides or shows it.

**Groups by name**: objects whose names share the part before the first
underscore are folded into a group named after it: `Doodad_Barrel_1` and
`Doodad_Crate` both sit under "Doodad". (A prefix used by only one object
does not make a group.) On a group's row, the arrow unfolds it, the eye
shows or hides every member, and clicking the name selects them all;
Shift+click adds the group to the selection. Rename an object to move it
to another group.

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

## Mirror

The Modifiers card adds a live mirror to the active object: turn on X, Y or Z
(or several) and a reflected copy appears on the other side of the
object's **origin**, along that axis of the object.

- The mirror follows every edit. In an edit mode the mirrored half is
  tinted blue and cannot be selected; you work on the real half.
- Vertices lying on the mirror plane are shared by both halves, so the seam
  is welded. Model one half with its open side on the plane.
- Put the plane where you need it by moving the origin: the Origin tile, or
  "Origin to Selection" with the seam's face or vertices selected.
- Exported FBX files contain the mirrored result. Merging objects bakes
  their mirrors first.
- **Apply** makes the mirror permanent: the mirrored half becomes ordinary
  geometry and the mirror is switched off.

The mirrored half uses the same UVs as the original, so both sides show
the same part of the texture.

## Array

The Modifiers card also has **Array**: set the count above 1 and the active
object is repeated in a row, each copy one **Step** further along the
object's own axes. Fences, stairs and runs of tiles are one object plus a
count. Like the mirror, the copies follow every edit, are tinted while you
edit, are written into exported FBX files, and become ordinary geometry
with **Apply**. Mirror and array can be combined (the mirror comes first).

## Hiding objects

- **H** hides the selected objects, **Shift+H** shows only them (in an edit
  mode: only the object being edited), **Alt+H** shows everything again.
- The same three are the Hide / Solo / Show all buttons under the object
  list, in the View menu and in the right-click menu. The eye in the object
  list toggles one object or one group.
- Hidden objects cannot be picked or moved, and the top-left of the 3D view
  says how many are hidden. They are still exported.

## Reference images

The Reference image card loads a picture to model against, one each for the
**Front**, **Side** and **Top** views. A picture appears only when you look
straight along its axis in orthographic view (1, 3, 7 and their Ctrl
opposites), so it never gets in the way while orbiting.

Height sets its size in scene units (front and side pictures stand on the
ground), Offset slides it, Opacity fades it, and **In front** draws it over
the model instead of behind it. Reference images are saved in the project
file by path.

## Moving things

- **Handles**: the Move, Rotate and Scale tiles in the Transform card choose
  which handles sit on the selection. They work for objects, vertices,
  edges and faces, and each drag is one undo step.
  - *Move*: drag the red / green / blue arrow to move along X / Y / Z, or
    the dot in the middle to move freely.
  - *Rotate*: drag a ring to turn about that axis. Only the near half of a
    ring can be grabbed; the far half is drawn faintly.
  - *Scale*: drag a box to scale along that axis, or the middle box to
    scale evenly.
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

## Views

The buttons at the top right of the 3D view (and the View menu, and the
keys below) look straight at the model from the front, back, right, left,
top or bottom. These views are orthographic, so parallel edges stay
parallel and things line up; the current one is named at the top left.
Orbiting away returns to perspective. "Ortho" (or 5) switches between the
two by hand, and then it stays as you set it.

"Front" shows the side of the model that faces +Z, the blue axis.

In the straight-on front, back, right and left views the grid stands
upright facing you (one square = one unit, with the red X or blue Z axis
along the ground and the green Y axis up the middle) instead of lying on
the ground where it would only be a line. It is drawn behind the model and
follows the view as you pan. Orbiting away brings the ground grid back.

## Inset, bevel, loop cut

All three work on the object being edited (Tab into vertex / face mode first).

- **Inset (I)**: shrinks the selected faces inwards and leaves a rim of
  new faces. Several selected faces inset together as one region.
- **Bevel (Ctrl+B)**: chamfers or rounds edges. In face mode it takes every
  edge of the selected faces; in vertex mode every edge whose two ends are
  selected. While sizing it, press **1-9** for the number of segments: 1 is
  a flat chamfer, more follow a quarter circle. The number is remembered
  for the next bevel. Corners where three or more rounded edges meet are
  closed with a fan of triangles.
- **Loop cut (Ctrl+R)**: hover an edge and a yellow preview shows the cut
  running round the ring of quads. Press 1-9 for the number of cuts, click
  to place. A single cut can then be slid along the edge; click again to
  confirm. The new loop is left selected in vertex mode.

There are two ways to start an inset or bevel:

- **From the Mesh card**: clicking the Inset or Bevel tile only picks the
  tool (the tile stays lit and nothing changes yet). Then press in the 3D
  view, drag to the right for more, and release to confirm. Esc or a
  right-click puts the tool away.
- **From the keyboard (I, Ctrl+B) or the right-click menu**: it starts at
  once under the mouse; move right for more, click or Enter confirms, Esc
  or right-click cancels. Snapping ("Snap every" / Ctrl) applies
to the inset and bevel width and to the loop-cut slide distance. Existing
faces keep their texture exactly in place through all three.

## Subdivide, bridge, fill

- **Subdivide**: splits the selected faces evenly. A quad becomes four
  quads, a triangle four triangles. Neighbouring faces receive the new
  midpoints so the mesh stays closed, and UVs are interpolated.
- **Bridge**: joins two edge loops with a band of quads. Select the two
  loops in edge mode (Alt+click each, Shift to add the second). In face
  mode, select two faces with the same number of corners: both are removed
  and their outlines joined, which makes a tunnel or a connecting piece.
- **Fill**: closes a hole. Select the edges (or vertices) around it and a
  face is made, facing the same way as its neighbours.

All three are tiles in the Mesh card and entries in the right-click menu.

## Size, and local axes

Under Pos and Rot the Transform card shows **Size**: the active object's
extent along its own X, Y and Z. Typing a number scales the object about
its origin to exactly that size. In an edit mode it shows the size of the
selection and scales it about its middle.

**Local axes** (next to Snap step) points the move arrows, rotation rings
and scale handles along the active object's own axes instead of the
world's, and makes X / Y / Z during G, R, S lock to those axes. It matters
once an object has been rotated.

## Knife, and making a face

- **Knife (K, or the Knife tile)**: click a point on an edge, then a point
  on another edge of the same face, and the face is cut between them. Keep
  clicking to carry the cut on from the last point. Close to a vertex the
  cut snaps to it; Ctrl snaps to the middle of an edge. Enter or a
  right-click finishes, Esc throws the cuts away. Points have to be on
  edges or vertices; to get a point inside a face, cut to it in two steps
  or subdivide first.
- **Make face (F, or the Face tile)**: in vertex or edge mode, select three
  or more vertices and a face is made from them. It follows existing edges
  between them when there are any, faces the same way as its neighbours,
  and otherwise faces you. F only does this when the selected vertices
  could become a face; with fewer than three selected (or when they already
  are a face) F switches to face mode as before.

## X-ray

The X-ray button at the top right of the 3D view (Alt+Z, or View > X-ray)
makes faces see-through. Edges and vertices behind them show, and can be
clicked, box-selected and circle-selected through the mesh. Faces are still
picked front-most.

## 3D viewport

| Input | Action |
|-------|--------|
| Click / Shift+click | Select / add to selection |
| Drag a handle | Move, rotate or scale along / about that axis |
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
| Shift+X / Y / Z during G, R, S | Add or drop an axis. One axis: along it. Two (Shift+X then Shift+Y): move or scale in both and leave the third alone; rotate turns within that plane. None or all three: free |
| K | Knife |
| F | Make a face from the selected vertices (otherwise: face mode) |
| Alt+Z | X-ray on / off |
| E | Extrude selected faces along their normal |
| I / Ctrl+B / Ctrl+R | Inset / bevel / loop cut |
| Shift+D | Duplicate (object or faces) |
| X or Delete | Delete (object or faces / vertices) |
| H / Shift+H / Alt+H | Hide selected / show only selected / show everything |
| Ctrl+C / Ctrl+V | Copy / paste the UVs of the selected faces |
| U | Show / hide the UV editor |
| 1 / 3 / 7 | Front / right / top view |
| Ctrl+1 / Ctrl+3 / Ctrl+7 (or Cmd) | Back / left / bottom view |
| 9 | The opposite side of the current view |
| 5 / Home | Toggle orthographic / frame all |
| . (full stop) | Centre the view on the selection. Double-clicking an object in the list does the same |
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
| Drag the U or V arrow in the middle | Scale along that one axis only, about the middle of the selection |
| G | Move with the mouse; X / Y lock an axis; click confirms, Esc cancels |
| S | Scale about the centre; X / Y lock an axis |
| R | Rotate about the middle of the selection with the mouse. Ctrl: 15° steps, Shift: 90° steps |
| Drag the round knob above the selection frame | Rotate, same modifiers |
| Rotate: Left 90° / Right 90° / 180° buttons | Exact quarter turns; with "Snap to pixels" on the result stays on the pixel grid |
| L / Shift+L | Select the whole island under the mouse / add it. The Linked button grows the selection to whole islands |
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

**Unwrap** (UV card in the right panel) lays out a whole model for you:

1. The model is cut into islands along every edge sharper than the **Seam
   angle** (50 degrees by default: a cube becomes six squares, the side of
   a cylinder stays one strip).
2. Each island is unfolded flat around its edges like a paper model, so no
   face is stretched and everything has the same pixel density.
3. The islands are packed in rows from the top of the texture, on whole
   pixels, **Gap** pixels apart, at **Pixels per unit**.

In object mode it unwraps all selected objects into one shared layout; in
an edit mode, the selected faces (or the whole object if none are
selected). Walls come out upright. If the islands do not fit the texture at
the chosen density, the highest whole number of pixels per unit that fits
is used instead and the status bar says what size the original density
would have needed. Unwrap replaces the UVs of those faces, so run it before
painting, not after.

"Box" (same card) projects at the same fixed texel density ("Pixels per
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
| Move | V | Drag the selected pixels somewhere else (the whole image if nothing is selected). Ctrl at the start of the drag leaves a copy behind. Arrow keys nudge by one pixel |

**Paint on model**: tick it and, while the Paint tab is open, the left
button paints straight onto the model in the 3D view with the pencil,
eraser, fill or picker. The stroke lands on whatever texture pixel is under
the cursor, so faces that share pixels are painted together. Orbit with the
right or middle button; the top-left of the view shows the pixel under the
cursor. Selections still confine the paint. Untick it to select and move
things in the 3D view again.

**Mirror**: X mirrors every stroke left to right, Y top to bottom, both
give four-way symmetry. The number beside each is where the mirror line
sits (drawn as a dashed line); right-click the number to put it back in the
middle. Applies to Pencil, Eraser, Line and Rect.

**Moving, copying and pasting pixels**: Ctrl+C copies the selected pixels
(the whole image without a selection), Ctrl+X cuts them, Ctrl+V pastes
under the mouse and switches to the Move tool. Pixels that were just moved
or pasted keep "floating": you can move or nudge them again without
disturbing what is underneath, until you paint or select something else.
Transparent pixels in the moved piece leave the image under them alone.

**Palettes**: "Palette…" replaces the starter swatches with a palette file:
GIMP `.gpl`, JASC `.pal`, `.hex`, paint.net `.txt`, or any image (its
colours are used, up to 256). Aseprite exports all of these. The choice is
remembered between sessions; "Default" goes back to the built-in 16.

**Opacity and blending**: the Opacity value (with 10 / 25 / 50 / 100
shortcuts) sets how strongly Pencil, Line, Rect and Fill paint; it
multiplies the alpha of the chosen colour. With **Blend** on (the default)
see-through paint is laid over the pixel underneath, so black at 10%
darkens and white at 10% lightens: shading and highlights. Within one
stroke each pixel is tinted once, however often the brush crosses it; a
new stroke over the same pixels tints them again. With Blend off, the
colour and its alpha replace the pixel exactly, which is how to write real
transparency into the image.

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

Export variants, in the File menu:

- **Export selected objects only**: the Export button writes just the
  selection (in an edit mode, the object being edited).
- **Export FBX per group…**: choose a folder and every name group gets its
  own file: all `Doodad_` objects in `Doodad.fbx`, all `Wall_` objects in
  `Wall.fbx`, and objects without a group in `Other.fbx`.
- From the command line, without using the window:
  `pixelmesh scene.pmesh --export out.fbx` or
  `pixelmesh tiles.fbx --export-groups folder`.

`cargo test` round-trips exports through the ufbx parser in strict mode and
imports FBX files written by Blender (`tests/fixtures`).

## UV extras

- **Copy UVs / Paste UVs** (UV Mapping card, right-click menu, Ctrl+C /
  Ctrl+V over the 3D view): copy the UVs of the selected faces and paste
  them onto other faces, so identical faces share the same pixels. A face
  takes its UVs from a copied face with the same number of corners; several
  copied faces are handed out in order. If a pasted face comes out turned,
  use Rotate.
- **Texel density check** (UV Mapping card or View menu): tints faces in
  the 3D view whose pixels per unit differ from the "Pixels per unit"
  setting. Blue has too few pixels (the texture looks stretched), red has
  too many, untinted faces match within about 5%.

## Autosave, recovery and recent files

- Once a minute, unsaved work (the scene and any unsaved paint) is copied
  to a recovery file in the background. If PixelMesh does not close
  properly, the next launch offers to **Recover** it. A normal exit, or
  saving everything, removes the copy.
- Closing the window with unsaved changes asks first: Save and quit, Quit
  without saving, or Cancel. A full FBX export counts as saving the scene.
- **File > Open recent** lists the last ten projects and imported FBX
  files.
- Settings and the recovery file live in
  `~/Library/Application Support/PixelMesh` on macOS (`~/.config/pixelmesh`
  on Linux, `%APPDATA%\PixelMesh` on Windows).

## Large scenes

Scenes with hundreds of objects are drawn from cached data: each distinct
mesh is prepared once, unselected and selected objects are kept in two
buffers on the graphics card, and those are only rebuilt when something in
them changes. Only the object being edited is rebuilt every frame. The
object list lays out just the rows in view. Undo keeps up to 200 steps or
about 256 MB, whichever comes first.

Running with the environment variable `PIXELMESH_PROFILE=1` prints the
time spent building each frame to the terminal.

## Code map

- `src/mesh.rs` - mesh data, primitives, edit and UV operations (unit tested)
- `src/mesh_ops.rs` - subdivide, bridge, fill, ray-to-UV and density look-ups (unit tested)
- `src/scene.rs` - settings, autosave, recent files, hiding, export variants, reference images
- `src/unwrap.rs` - automatic UV unwrap (unit tested)
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
