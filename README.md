# PixelMesh

A deliberately small low-poly mesh editor: basic meshes, basic editing, a
pixel-snapping UV editor for pixel-art textures, and binary FBX export.

## Build and run

    cargo run --release
    cargo run --release -- model.pmesh sheet.png   # open a project / texture directly

Needs a recent stable Rust and OpenGL 3.3. On Linux, file dialogs use the
XDG desktop portal.

## Layout

| Area   | What it does |
|--------|--------------|
| Left   | UV editor: the texture with the UVs of the selected faces |
| Centre | 3D viewport |
| Right  | Tools: add meshes, transform, mesh ops, UV mapping, texture, export |

## 3D viewport

| Input | Action |
|-------|--------|
| Click / Shift+click | Select / add to selection |
| Middle- or right-drag (or Alt+left-drag) | Orbit; with Shift: pan |
| Scroll | Zoom |
| V / F | Vertex mode / face mode |
| A / Alt+A / L | Select all / none / linked |
| G / R / S | Move / rotate / scale. Then X, Y, Z to lock an axis, Ctrl to snap, click or Enter to confirm, Esc or right-click to cancel |
| E | Extrude selected faces along their normal |
| Shift+D | Duplicate |
| X or Delete | Delete |
| 1 / 3 / 7 / 5 / Home | Front / right / top / toggle orthographic / frame all |
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo |

## UV editor

Select faces in the 3D view, then:

- drag a corner to move it (corners sharing the spot move together),
- drag inside a face to move all selected faces as one island,
- scroll to zoom, right- or middle-drag to pan.

With "Snap UVs to pixels" on, everything lands on texel boundaries.
"Box project" unwraps at a fixed texel density ("Pixels per unit"), so a
1-unit face covers exactly e.g. 16x16 pixels. The texture reloads
automatically when the PNG changes on disk.

## FBX export

File > Export FBX writes binary FBX 7.4: Y-up, 1 unit = 1 metre, flat
normals, one UV set, one material with the texture on the diffuse slot. The
texture is referenced by file name (a copy is placed next to the .fbx if
missing) or embedded if "Embed texture in FBX" is ticked.

`cargo test` round-trips exports through the ufbx parser in strict mode.

## Code map

- `src/mesh.rs` - mesh data, primitives, edit and UV operations (unit tested)
- `src/fbx.rs` - FBX writer
- `src/app.rs` - state, menus, tool panel, files
- `src/viewport.rs` - 3D view, picking, modal transforms
- `src/uv_editor.rs` - UV editor
- `src/render.rs`, `src/camera.rs` - OpenGL drawing, orbit camera
