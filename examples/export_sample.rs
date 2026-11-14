//! Writes a small textured sample: `cargo run --example export_sample -- out_dir`
use glam::{Vec2, Vec3};
use pixelmesh::fbx::{export_file, FbxObject, FbxOptions, FbxTexture};
use pixelmesh::mesh::Mesh;

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let mut cube = Mesh::cube();
    let mut cone = Mesh::cylinder(6, 0.5, 0.0, 1.0).translated(Vec3::new(2.0, 0.0, 0.0));
    for m in [&mut cube, &mut cone] {
        let all = (0..m.faces.len()).collect();
        m.uv_box_project(&all, 16.0, Vec2::new(16.0, 16.0));
    }
    let objects = [FbxObject { name: "Crate", mesh: &cube }, FbxObject { name: "Spike", mesh: &cone }];

    // 16x16 checker so UV orientation is visible after import.
    let img = image::RgbaImage::from_fn(16, 16, |x, y| {
        let v = if (x / 4 + y / 4) % 2 == 0 { 230 } else { 90 };
        image::Rgba([v, (x * 16) as u8, (y * 16) as u8, 255])
    });
    img.save(dir.join("sheet.png")).unwrap();
    let png = std::fs::read(dir.join("sheet.png")).unwrap();

    let tex = |embed| Some(FbxTexture { file_name: "sheet.png", embed });
    export_file(&dir.join("sample.fbx"), &objects, &FbxOptions { material_name: "Sample", scale: 1.0, texture: tex(None) }).unwrap();
    export_file(&dir.join("sample_embedded.fbx"), &objects, &FbxOptions { material_name: "Sample", scale: 1.0, texture: tex(Some(&png)) }).unwrap();
    println!("{} objects", objects.len());
}
