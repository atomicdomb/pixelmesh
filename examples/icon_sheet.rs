//! Writes every interface icon, enlarged, to one PNG for review:
//! `cargo run --example icon_sheet -- icons.png`
use pixelmesh::icons;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "icons.png".into());
    let (scale, cols, pad) = (6u32, 11u32, 2u32);
    let cell = (icons::SIZE + pad) * scale;
    let rows = (icons::ALL.len() as u32).div_ceil(cols);
    let mut img = image::RgbaImage::from_pixel(cols * cell, rows * cell, image::Rgba([38, 40, 46, 255]));
    for (i, icon) in icons::ALL.iter().enumerate() {
        let data = icons::pixels(*icon);
        let (ox, oy) = ((i as u32 % cols) * cell + scale, (i as u32 / cols) * cell + scale);
        for y in 0..icons::SIZE {
            for x in 0..icons::SIZE {
                let k = ((y * icons::SIZE + x) * 4) as usize;
                if data[k + 3] > 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            img.put_pixel(ox + x * scale + dx, oy + y * scale + dy, image::Rgba([data[k], data[k + 1], data[k + 2], 255]));
                        }
                    }
                }
            }
        }
    }
    img.save(&path).unwrap();
}
