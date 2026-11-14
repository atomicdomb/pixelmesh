//! Icon tools.
//!
//! `cargo run --example icon_sheet -- preview.png`
//!     writes every icon enlarged, for a quick look.
//! `cargo run --example icon_sheet -- assets/icons.png --atlas`
//!     writes the built-in drawings as the 1:1 atlas the app loads.
//!     This OVERWRITES the file, including any icons you have edited.
use pixelmesh::icons;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().cloned().unwrap_or_else(|| "icons-preview.png".into());
    let atlas = args.iter().any(|a| a == "--atlas");
    let (scale, pad) = if atlas { (1u32, 0u32) } else { (6, 2) };
    let cols = icons::ATLAS_COLS;
    let cell = (icons::SIZE + pad) * scale;
    let rows = (icons::ALL.len() as u32).div_ceil(cols);
    let background = if atlas { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([38, 40, 46, 255]) };
    let mut img = image::RgbaImage::from_pixel(cols * cell, rows * cell, background);
    for (i, icon) in icons::ALL.iter().enumerate() {
        let data = icons::pixels(*icon);
        let (ox, oy) = ((i as u32 % cols) * cell + pad * scale / 2, (i as u32 / cols) * cell + pad * scale / 2);
        for y in 0..icons::SIZE {
            for x in 0..icons::SIZE {
                let k = ((y * icons::SIZE + x) * 4) as usize;
                if data[k + 3] > 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            img.put_pixel(ox + x * scale + dx, oy + y * scale + dy, image::Rgba([data[k], data[k + 1], data[k + 2], data[k + 3]]));
                        }
                    }
                }
            }
        }
    }
    img.save(&path).unwrap();
    println!("wrote {path} ({}x{})", img.width(), img.height());
}
