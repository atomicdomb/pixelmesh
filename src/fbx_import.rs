//! FBX import, built on the ufbx parser (binary and ASCII FBX, old and new versions).
//!
//! Every mesh node becomes one object. Node transforms, the file's up axis
//! and its unit are all baked into the vertices, so whatever the source
//! application used, the result is Y-up in metres like the rest of the editor.

use crate::mesh::{Face, Mesh};
use glam::{Vec2, Vec3};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub struct ImportedObject {
    pub name: String,
    pub mesh: Mesh,
}

pub struct ImportedTexture {
    /// Where the texture file should be (it may not exist when the image was embedded).
    pub path: PathBuf,
    /// The embedded image file, if the FBX carried one.
    pub embedded: Option<Vec<u8>>,
}

pub struct Imported {
    pub objects: Vec<ImportedObject>,
    /// First diffuse / base-colour texture found on the imported meshes.
    pub texture: Option<ImportedTexture>,
    /// How many faces had no UVs and were given a default mapping.
    pub faces_without_uvs: usize,
}

pub fn import_file(path: &Path) -> Result<Imported, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    import_bytes(&bytes, path.parent().unwrap_or(Path::new(".")))
}

/// `base_dir` is the folder relative texture paths are resolved against.
pub fn import_bytes(bytes: &[u8], base_dir: &Path) -> Result<Imported, String> {
    let opts = ufbx::LoadOpts {
        // Convert to the editor's convention while loading.
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        ignore_animation: true,
        ..Default::default()
    };
    let scene = ufbx::load_memory(bytes, opts).map_err(|e| format!("not a readable FBX file ({})", e.description))?;

    let mut out = Imported { objects: Vec::new(), texture: None, faces_without_uvs: 0 };
    for node in &scene.nodes {
        let Some(fm) = &node.mesh else { continue };
        let to_world = &node.geometry_to_world;
        let mut mesh = Mesh::default();
        for v in fm.vertices.iter() {
            let p = ufbx::transform_position(to_world, *v);
            mesh.positions.push(Vec3::new(p.x as f32, p.y as f32, p.z as f32));
        }
        // A mirrored node (negative scale) turns every face inside out; undo that.
        let mirrored = ufbx::matrix_determinant(to_world) < 0.0;
        let has_uv = fm.vertex_uv.exists;
        let mut missing_uv: BTreeSet<usize> = BTreeSet::new();
        for face in fm.faces.iter() {
            let mut verts: Vec<u32> = Vec::new();
            let mut uvs: Vec<Vec2> = Vec::new();
            for k in 0..face.num_indices as usize {
                let ix = face.index_begin as usize + k;
                let v = fm.vertex_indices[ix];
                // Drop repeated corners (zero-length edges).
                if verts.last() == Some(&v) {
                    continue;
                }
                verts.push(v);
                uvs.push(if has_uv {
                    let uv = fm.vertex_uv.values[fm.vertex_uv.indices[ix] as usize];
                    Vec2::new(uv.x as f32, uv.y as f32)
                } else {
                    Vec2::ZERO
                });
            }
            if verts.len() > 1 && verts.first() == verts.last() {
                verts.pop();
                uvs.pop();
            }
            if verts.len() < 3 {
                continue; // points and lines
            }
            if mirrored {
                verts.reverse();
                uvs.reverse();
            }
            if !has_uv {
                missing_uv.insert(mesh.faces.len());
            }
            mesh.faces.push(Face { verts, uvs });
        }
        if mesh.faces.is_empty() {
            continue;
        }
        mesh.remove_unused_verts();
        out.faces_without_uvs += missing_uv.len();
        mesh.uv_fit_each(&missing_uv);

        if out.texture.is_none() {
            out.texture = node.materials.iter().find_map(|m| texture_of(m, base_dir));
        }
        let name = if node.element.name.is_empty() { "Object".to_string() } else { node.element.name.to_string() };
        out.objects.push(ImportedObject { name, mesh });
    }
    if out.objects.is_empty() {
        return Err("the file contains no meshes".into());
    }
    Ok(out)
}

fn texture_of(material: &ufbx::Material, base_dir: &Path) -> Option<ImportedTexture> {
    let tex = material.fbx.diffuse_color.texture.as_ref().or(material.pbr.base_color.texture.as_ref())?;
    // Prefer a path that actually exists: as written, relative to the FBX, or just the file name next to it.
    let names = [tex.filename.to_string(), tex.relative_filename.to_string(), tex.absolute_filename.to_string()];
    let mut candidates: Vec<PathBuf> = Vec::new();
    for n in names.iter().filter(|n| !n.is_empty()) {
        // FBX files written on Windows use backslashes.
        let p = PathBuf::from(n.replace('\\', "/"));
        candidates.push(if p.is_absolute() { p.clone() } else { base_dir.join(&p) });
        if let Some(file) = p.file_name() {
            candidates.push(base_dir.join(file));
        }
    }
    let embedded = (!tex.content.is_empty()).then(|| tex.content.to_vec());
    let path = candidates.iter().find(|p| p.is_file()).or(candidates.last()).cloned()?;
    Some(ImportedTexture { path, embedded })
}
