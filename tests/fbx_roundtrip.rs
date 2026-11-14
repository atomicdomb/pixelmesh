//! Export with our writer, then parse the result with ufbx (an independent,
//! widely used FBX loader) and compare what comes back.

use glam::{Vec2, Vec3};
use pixelmesh::fbx::{export_bytes, FbxObject, FbxOptions, FbxTexture};
use pixelmesh::mesh::Mesh;

fn sample_mesh() -> Mesh {
    let mut m = Mesh::cube();
    m.append(&Mesh::cylinder(6, 0.5, 0.0, 1.0).translated(Vec3::new(2.0, 0.0, 0.0)));
    let all = (0..m.faces.len()).collect();
    m.uv_box_project(&all, 16.0, Vec2::new(64.0, 64.0));
    m
}

fn load(bytes: &[u8]) -> ufbx::SceneRoot {
    // strict: reject anything ufbx would otherwise silently repair.
    let opts = ufbx::LoadOpts { strict: true, ..Default::default() };
    ufbx::load_memory(bytes, opts).expect("ufbx failed to parse the exported file")
}

#[test]
fn geometry_and_uvs_survive() {
    let mesh = sample_mesh();
    let bytes = export_bytes(&[FbxObject::baked("Thing", &mesh)], &FbxOptions { material_name: "Mat", texture: None, scale: 1.0 });
    let scene = load(&bytes);

    assert_eq!(scene.metadata.version, 7400);
    assert!(!scene.metadata.ascii);
    assert_eq!(scene.settings.axes.up, ufbx::CoordinateAxis::PositiveY);
    assert_eq!(scene.settings.unit_meters, 1.0);

    assert_eq!(scene.meshes.len(), 1);
    let fm = &scene.meshes[0];
    assert_eq!(fm.num_faces, mesh.faces.len());
    assert_eq!(fm.num_vertices, mesh.positions.len());
    assert!(fm.vertex_uv.exists && fm.vertex_normal.exists);

    for (fi, face) in mesh.faces.iter().enumerate() {
        let ff = &fm.faces[fi];
        assert_eq!(ff.num_indices as usize, face.verts.len());
        let n = mesh.face_normal(fi);
        for k in 0..face.verts.len() {
            let ix = ff.index_begin as usize + k;
            let p = fm.vertex_position.values[fm.vertex_position.indices[ix] as usize];
            let uv = fm.vertex_uv.values[fm.vertex_uv.indices[ix] as usize];
            let fnrm = fm.vertex_normal.values[fm.vertex_normal.indices[ix] as usize];
            let want = mesh.positions[face.verts[k] as usize];
            assert!((Vec3::new(p.x as f32, p.y as f32, p.z as f32) - want).length() < 1e-6);
            assert!((Vec2::new(uv.x as f32, uv.y as f32) - face.uvs[k]).length() < 1e-6);
            assert!((Vec3::new(fnrm.x as f32, fnrm.y as f32, fnrm.z as f32) - n).length() < 1e-5);
        }
    }

    assert_eq!(scene.nodes.iter().filter(|n| n.mesh.is_some()).count(), 1);
    let node = scene.nodes.iter().find(|n| n.mesh.is_some()).unwrap();
    assert_eq!(&*node.element.name, "Thing");
    assert_eq!(node.materials.len(), 1);
}

#[test]
fn texture_is_linked_and_embedded() {
    let mesh = sample_mesh();
    let png = b"\x89PNG fake bytes for the test";
    let opt = FbxOptions { material_name: "Mat", scale: 1.0, texture: Some(FbxTexture { file_name: "sheet.png", embed: Some(png) }) };
    let scene = load(&export_bytes(&[FbxObject::baked("Thing", &mesh)], &opt));

    assert_eq!(scene.materials.len(), 1);
    let mat = &scene.materials[0];
    let tex = mat.fbx.diffuse_color.texture.as_ref().expect("diffuse texture not connected");
    assert_eq!(&*tex.relative_filename, "sheet.png");
    assert_eq!(tex.content.as_ref(), png);

    let opt = FbxOptions { material_name: "Mat", scale: 1.0, texture: Some(FbxTexture { file_name: "sheet.png", embed: None }) };
    let scene = load(&export_bytes(&[FbxObject::baked("Thing", &mesh)], &opt));
    assert!(scene.materials[0].fbx.diffuse_color.texture.as_ref().unwrap().content.is_empty());
}

#[test]
fn every_object_becomes_its_own_model() {
    let cube = Mesh::cube();
    let cone = Mesh::cylinder(6, 0.5, 0.0, 1.0).translated(Vec3::new(2.0, 0.0, 0.0));
    let objects = [FbxObject::baked("Crate", &cube), FbxObject::baked("Spike", &cone)];
    let opt = FbxOptions { material_name: "Mat", scale: 1.0, texture: Some(FbxTexture { file_name: "sheet.png", embed: None }) };
    let scene = load(&export_bytes(&objects, &opt));

    assert_eq!(scene.meshes.len(), 2);
    assert_eq!(scene.materials.len(), 1);
    let nodes: Vec<_> = scene.nodes.iter().filter(|n| n.mesh.is_some()).collect();
    assert_eq!(nodes.len(), 2);
    for (node, (name, mesh)) in nodes.iter().zip([("Crate", &cube), ("Spike", &cone)]) {
        assert_eq!(&*node.element.name, name);
        let fm = node.mesh.as_ref().unwrap();
        assert_eq!((fm.num_vertices, fm.num_faces), (mesh.positions.len(), mesh.faces.len()));
        assert_eq!(node.materials.len(), 1);
        assert!(node.materials[0].fbx.diffuse_color.texture.is_some());
    }
}
