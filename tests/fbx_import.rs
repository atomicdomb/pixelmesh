//! Import tests: our own exports must come back unchanged, and files written
//! by Blender (Z-up, centimetre units, object transforms) must arrive
//! Y-up, in metres, with outward faces.

use glam::{Vec2, Vec3};
use pixelmesh::fbx::{export_bytes, FbxObject, FbxOptions, FbxTexture};
use pixelmesh::fbx_import::{import_bytes, import_file};
use pixelmesh::mesh::Mesh;
use std::path::Path;

fn bounds(m: &Mesh) -> (Vec3, Vec3) {
    let lo = m.positions.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
    let hi = m.positions.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
    (lo, hi)
}

#[test]
fn own_export_round_trips_exactly() {
    let mut cube = Mesh::cube();
    cube.bevel_edges(&cube.edges().into_iter().collect(), 0.1); // triangles, quads and octagon-free n-gons
    let mut cone = Mesh::cylinder(6, 0.5, 0.0, 1.0).translated(Vec3::new(2.0, 0.0, 0.0));
    let all = (0..cone.faces.len()).collect();
    cone.uv_box_project(&all, 16.0, Vec2::splat(64.0));
    let objects = [FbxObject::baked("Crate", &cube), FbxObject::baked("Spike", &cone)];
    let png = b"\x89PNG not really";
    let opt = FbxOptions { material_name: "Mat", scale: 1.0, texture: Some(FbxTexture { file_name: "sheet.png", embed: Some(png) }) };

    let back = import_bytes(&export_bytes(&objects, &opt), Path::new("/some/dir")).unwrap();
    assert_eq!(back.objects.len(), 2);
    assert_eq!(back.faces_without_uvs, 0);
    for (got, (name, want)) in back.objects.iter().zip([("Crate", &cube), ("Spike", &cone)]) {
        assert_eq!(got.name, name);
        assert_eq!(got.mesh.positions.len(), want.positions.len());
        assert_eq!(got.mesh.faces.len(), want.faces.len());
        for (gf, wf) in got.mesh.faces.iter().zip(&want.faces) {
            assert_eq!(gf.verts.len(), wf.verts.len());
            for k in 0..gf.verts.len() {
                assert!(got.mesh.positions[gf.verts[k] as usize].distance(want.positions[wf.verts[k] as usize]) < 1e-6);
                assert!(gf.uvs[k].distance(wf.uvs[k]) < 1e-6);
            }
        }
    }
    let tex = back.texture.expect("texture reference lost");
    assert_eq!(tex.path, Path::new("/some/dir/sheet.png"));
    assert_eq!(tex.embedded.as_deref(), Some(&png[..]));
}

fn check_blender_scene(file: &str, expect_embedded: bool) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(file);
    let scene = import_file(&path).unwrap();
    assert_eq!(scene.objects.len(), 2, "{file}");

    // Blender: 2 m cube centred at (3, 1, 1), Z up.  Here: Y up, so (x, y, z) -> (x, z, -y).
    let crate_ = scene.objects.iter().find(|o| o.name == "Crate").expect("Crate missing");
    let (lo, hi) = bounds(&crate_.mesh);
    assert!(lo.distance(Vec3::new(2.0, 0.0, -2.0)) < 1e-4, "{file}: {lo}");
    assert!(hi.distance(Vec3::new(4.0, 2.0, 0.0)) < 1e-4, "{file}: {hi}");
    assert_eq!((crate_.mesh.positions.len(), crate_.mesh.faces.len()), (8, 6));
    let centre = (lo + hi) * 0.5;
    for f in 0..6 {
        assert!(crate_.mesh.face_normal(f).dot(crate_.mesh.face_center(f) - centre) > 0.0, "{file}: face {f} points inwards");
        assert!(crate_.mesh.faces[f].uvs.iter().all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)));
    }
    // The default cube unwrap gives every face a quarter-by-quarter square of the sheet.
    let (ulo, uhi) = crate_.mesh.uv_bounds(&[0].into()).unwrap();
    assert!(((uhi - ulo) - Vec2::new(0.25, 0.25)).length() < 1e-4);

    // A plane scaled by -1 on X in Blender still faces up.
    let plane = scene.objects.iter().find(|o| o.name == "MirroredPlane").expect("plane missing");
    assert!(plane.mesh.face_normal(0).distance(Vec3::Y) < 1e-4, "{file}: {}", plane.mesh.face_normal(0));
    assert!((bounds(&plane.mesh).0.y - 0.5).abs() < 1e-4);

    let tex = scene.texture.expect("texture not found");
    assert_eq!(tex.path.file_name().unwrap(), "crate_tex.png");
    assert_eq!(tex.embedded.is_some(), expect_embedded);
    if let Some(bytes) = tex.embedded {
        assert!(bytes.starts_with(b"\x89PNG"));
    }
}

#[test]
fn blender_export_default_settings() {
    check_blender_scene("blender_scene.fbx", true);
}

#[test]
fn blender_export_other_axes_and_units() {
    check_blender_scene("blender_scene_cm.fbx", false);
}

#[test]
fn rubbish_is_rejected() {
    assert!(import_bytes(b"this is not an fbx file", Path::new(".")).is_err());
}

#[test]
fn origin_and_rotation_survive_a_round_trip() {
    use pixelmesh::xform::euler_quat;
    // A cube turned and moved: world-space vertices plus the pivot / orientation that go with them.
    let (origin, rotation) = (Vec3::new(2.0, 0.5, -1.0), Vec3::new(20.0, 45.0, -30.0));
    let q = euler_quat(rotation);
    let mut cube = Mesh::cube();
    for p in &mut cube.positions {
        *p = origin + q * *p;
    }
    let objects = [FbxObject { name: "Turned", mesh: &cube, origin, rotation }];
    let bytes = export_bytes(&objects, &FbxOptions { material_name: "Mat", scale: 1.0, texture: None });
    let back = import_bytes(&bytes, Path::new(".")).unwrap();
    let got = &back.objects[0];
    assert!(got.origin.distance(origin) < 1e-5, "{}", got.origin);
    assert!(euler_quat(got.rotation).angle_between(q) < 1e-3, "{}", got.rotation);
    // The geometry is back where it was in the world.
    for (a, b) in got.mesh.positions.iter().zip(&cube.positions) {
        assert!(a.distance(*b) < 1e-5, "{a} vs {b}");
    }
    // And in the file it is stored unrotated: an axis-aligned unit cube around the pivot.
    let scene = ufbx::load_memory(&bytes, ufbx::LoadOpts::default()).unwrap();
    let local = &scene.meshes[0].vertices;
    assert!(local.iter().all(|v| [v.x, v.y, v.z].iter().all(|c| (c.abs() - 0.5).abs() < 1e-5 || c.abs() < 1e-5 || (c - 1.0).abs() < 1e-5)));
}
