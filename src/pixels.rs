//! Pixel operations for the paint editor. Images are RGBA8, row 0 at the top.

pub type Rgba = [u8; 4];

fn index(w: u32, h: u32, x: i32, y: i32) -> Option<usize> {
    (x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h).then(|| (y as usize * w as usize + x as usize) * 4)
}

pub fn get(data: &[u8], w: u32, h: u32, x: i32, y: i32) -> Option<Rgba> {
    index(w, h, x, y).map(|i| [data[i], data[i + 1], data[i + 2], data[i + 3]])
}

pub fn set(data: &mut [u8], w: u32, h: u32, x: i32, y: i32, c: Rgba) {
    if let Some(i) = index(w, h, x, y) {
        data[i..i + 4].copy_from_slice(&c);
    }
}

/// Square brush of `size` pixels, centred on (x, y) (even sizes extend down-right).
pub fn stamp(data: &mut [u8], w: u32, h: u32, x: i32, y: i32, size: u32, c: Rgba) {
    let size = size.max(1) as i32;
    let off = (size - 1) / 2;
    for dy in 0..size {
        for dx in 0..size {
            set(data, w, h, x - off + dx, y - off + dy, c);
        }
    }
}

/// Bresenham line, stamped with the brush.
pub fn line(data: &mut [u8], w: u32, h: u32, from: (i32, i32), to: (i32, i32), size: u32, c: Rgba) {
    let (mut x, mut y) = from;
    let (dx, dy) = ((to.0 - x).abs(), -(to.1 - y).abs());
    let (sx, sy) = (if x < to.0 { 1 } else { -1 }, if y < to.1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        stamp(data, w, h, x, y, size, c);
        if (x, y) == to {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Axis-aligned rectangle between two corner pixels (inclusive).
pub fn rect(data: &mut [u8], w: u32, h: u32, a: (i32, i32), b: (i32, i32), filled: bool, size: u32, c: Rgba) {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    if filled {
        for y in y0.max(0)..=y1.min(h as i32 - 1) {
            for x in x0.max(0)..=x1.min(w as i32 - 1) {
                set(data, w, h, x, y, c);
            }
        }
    } else {
        line(data, w, h, (x0, y0), (x1, y0), size, c);
        line(data, w, h, (x0, y1), (x1, y1), size, c);
        line(data, w, h, (x0, y0), (x0, y1), size, c);
        line(data, w, h, (x1, y0), (x1, y1), size, c);
    }
}

/// The 4-connected area of identical colour around (x, y), as one flag per pixel.
/// With `allowed`, pixels outside that mask act as walls (and a start outside it gives nothing).
pub fn flood_region(data: &[u8], w: u32, h: u32, x: i32, y: i32, allowed: Option<&[bool]>) -> Vec<bool> {
    let mut region = vec![false; (w * h) as usize];
    let Some(target) = get(data, w, h, x, y) else { return region };
    let open = |px: i32, py: i32, region: &[bool]| {
        index(w, h, px, py).is_some_and(|i| {
            let k = i / 4;
            !region[k] && allowed.is_none_or(|m| m[k]) && data[i..i + 4] == target
        })
    };
    let mut stack = vec![(x, y)];
    while let Some((px, py)) = stack.pop() {
        if !open(px, py, &region) {
            continue;
        }
        region[py as usize * w as usize + px as usize] = true;
        stack.extend([(px + 1, py), (px - 1, py), (px, py + 1), (px, py - 1)]);
    }
    region
}

/// Replace the 4-connected area of identical colour around (x, y), staying inside `allowed` if given.
pub fn flood_fill(data: &mut [u8], w: u32, h: u32, x: i32, y: i32, c: Rgba, allowed: Option<&[bool]>) {
    if get(data, w, h, x, y) == Some(c) {
        return;
    }
    let region = flood_region(data, w, h, x, y, allowed);
    for (k, _) in region.iter().enumerate().filter(|(_, inside)| **inside) {
        data[k * 4..k * 4 + 4].copy_from_slice(&c);
    }
}

/// Lay colour `src` over colour `dst` ("normal" blending, straight alpha).
pub fn blend_over(dst: Rgba, src: Rgba) -> Rgba {
    let (sa, da) = (src[3] as f32 / 255.0, dst[3] as f32 / 255.0);
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return [0, 0, 0, 0];
    }
    let ch = |i: usize| ((src[i] as f32 * sa + dst[i] as f32 * da * (1.0 - sa)) / out_a).round() as u8;
    [ch(0), ch(1), ch(2), (out_a * 255.0).round() as u8]
}

/// Combine a stroke with the image it is drawn on. `layer` holds the stroke's own
/// pixels (alpha 0 = untouched); `base` is the image before the stroke. Each
/// touched pixel is blended over the base exactly once, however often the brush
/// passed over it. With `blend` off the stroke's pixels simply replace the base.
pub fn composite(data: &mut [u8], base: &[u8], layer: &[u8], blend: bool) {
    for ((out, b), l) in data.chunks_exact_mut(4).zip(base.chunks_exact(4)).zip(layer.chunks_exact(4)) {
        let under = [b[0], b[1], b[2], b[3]];
        let px = if l[3] == 0 {
            under
        } else if blend {
            blend_over(under, [l[0], l[1], l[2], l[3]])
        } else {
            [l[0], l[1], l[2], l[3]]
        };
        out.copy_from_slice(&px);
    }
}

/// Put back every pixel outside `mask` from `original` (used to confine a stroke to a selection).
pub fn restore_outside(data: &mut [u8], original: &[u8], mask: &[bool]) {
    for (k, _) in mask.iter().enumerate().filter(|(_, selected)| !**selected) {
        data[k * 4..k * 4 + 4].copy_from_slice(&original[k * 4..k * 4 + 4]);
    }
}

/// Outline of a selection as horizontal and vertical segments on the pixel grid
/// (corner coordinates), with straight runs merged.
pub fn mask_outline(mask: &[bool], w: u32, h: u32) -> Vec<[(i32, i32); 2]> {
    let (w, h) = (w as i32, h as i32);
    let at = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && mask[(y * w + x) as usize];
    let mut segments = Vec::new();
    // Horizontal edges: between row y-1 and row y.
    for y in 0..=h {
        let mut start = None;
        for x in 0..=w {
            let edge = x < w && at(x, y - 1) != at(x, y);
            match (edge, start) {
                (true, None) => start = Some(x),
                (false, Some(s)) => {
                    segments.push([(s, y), (x, y)]);
                    start = None;
                }
                _ => {}
            }
        }
    }
    for x in 0..=w {
        let mut start = None;
        for y in 0..=h {
            let edge = y < h && at(x - 1, y) != at(x, y);
            match (edge, start) {
                (true, None) => start = Some(y),
                (false, Some(s)) => {
                    segments.push([(x, s), (x, y)]);
                    start = None;
                }
                _ => {}
            }
        }
    }
    segments
}

/// Distinct visible colours in the image, most used first, at most `max`.
pub fn used_colors(data: &[u8], max: usize) -> Vec<Rgba> {
    let mut counts: std::collections::HashMap<Rgba, usize> = std::collections::HashMap::new();
    for px in data.chunks_exact(4) {
        if px[3] > 0 {
            *counts.entry([px[0], px[1], px[2], px[3]]).or_default() += 1;
        }
    }
    let mut list: Vec<(Rgba, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    list.into_iter().take(max).map(|(c, _)| c).collect()
}

/// Read the colours out of a palette file's text. Understands GIMP `.gpl`, JASC `.pal`,
/// `.hex` (one RRGGBB per line) and paint.net `.txt` (AARRGGBB). Duplicates are dropped
/// and at most 256 colours are returned.
pub fn parse_palette(text: &str) -> Vec<Rgba> {
    let mut out: Vec<Rgba> = Vec::new();
    // A JASC file starts with three header lines: the name, a version and the colour count.
    let mut skip = 0;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if line.eq_ignore_ascii_case("JASC-PAL") {
            skip = 2;
            continue;
        }
        let hex = line.strip_prefix('#').unwrap_or(line);
        let is_hex = (hex.len() == 6 || hex.len() == 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) && !hex.contains(' ');
        let color = if is_hex {
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
            // Eight digits: paint.net's AARRGGBB.
            Some(if hex.len() == 8 { [byte(2), byte(4), byte(6), byte(0)] } else { [byte(0), byte(2), byte(4), 255] })
        } else if line.starts_with('#') || line.starts_with(';') {
            None
        } else {
            // "R G B" (optionally followed by a name).
            let nums: Vec<u8> = line.split_whitespace().take(3).map_while(|t| t.parse::<u8>().ok()).collect();
            (nums.len() == 3).then(|| [nums[0], nums[1], nums[2], 255])
        };
        if let Some(c) = color {
            if !out.contains(&c) && out.len() < 256 {
                out.push(c);
            }
        }
    }
    out
}

/// The distinct colours of an image in the order they first appear (row by row), at most `max`.
pub fn image_palette(data: &[u8], max: usize) -> Vec<Rgba> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for px in data.chunks_exact(4) {
        let c = [px[0], px[1], px[2], px[3]];
        if c[3] != 0 && seen.insert(c) {
            out.push(c);
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

/// A piece of an image that was cut or copied: a rectangle of pixels with a mask saying which of them count.
#[derive(Clone)]
pub struct Clip {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub mask: Vec<bool>,
    /// Where its top-left corner was in the image it came from.
    pub origin: (i32, i32),
}

/// Copy the masked pixels (all of them without a mask) into a clip cropped to their bounding box.
pub fn lift(data: &[u8], w: u32, h: u32, mask: Option<&[bool]>) -> Option<Clip> {
    let selected = |x: u32, y: u32| mask.is_none_or(|m| m[(y * w + x) as usize]);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if selected(x, y) {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    if x0 == u32::MAX {
        return None;
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut clip = Clip { w: cw, h: ch, rgba: vec![0; (cw * ch * 4) as usize], mask: vec![false; (cw * ch) as usize], origin: (x0 as i32, y0 as i32) };
    for y in 0..ch {
        for x in 0..cw {
            if selected(x0 + x, y0 + y) {
                let (src, dst) = ((((y0 + y) * w + x0 + x) * 4) as usize, ((y * cw + x) * 4) as usize);
                clip.rgba[dst..dst + 4].copy_from_slice(&data[src..src + 4]);
                clip.mask[(y * cw + x) as usize] = true;
            }
        }
    }
    Some(clip)
}

/// Put a clip into an image with its top-left corner at `at`. Transparent pixels of the clip
/// leave the image alone. Returns the mask of the image pixels the clip covers.
pub fn drop_clip(data: &mut [u8], w: u32, h: u32, clip: &Clip, at: (i32, i32)) -> Vec<bool> {
    let mut covered = vec![false; (w * h) as usize];
    for y in 0..clip.h as i32 {
        for x in 0..clip.w as i32 {
            let k = (y as u32 * clip.w + x as u32) as usize;
            if !clip.mask[k] {
                continue;
            }
            let Some(i) = index(w, h, at.0 + x, at.1 + y) else { continue };
            covered[i / 4] = true;
            if clip.rgba[k * 4 + 3] != 0 {
                data[i..i + 4].copy_from_slice(&clip.rgba[k * 4..k * 4 + 4]);
            }
        }
    }
    covered
}

/// Make the masked pixels transparent.
pub fn clear_masked(data: &mut [u8], mask: &[bool]) {
    for (k, _) in mask.iter().enumerate().filter(|(_, m)| **m) {
        data[k * 4..k * 4 + 4].copy_from_slice(&[0, 0, 0, 0]);
    }
}

/// Mirror image of a brush position across the line `axis` (a pixel boundary), for a brush `size` pixels wide.
pub fn mirror_coord(x: i32, axis: i32, size: u32) -> i32 {
    let (n, off) = (size as i32, (size as i32 - 1) / 2);
    2 * axis - x + 2 * off - n
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rgba = [255, 0, 0, 255];
    const G: Rgba = [0, 255, 0, 255];
    const CLEAR: Rgba = [0, 0, 0, 0];

    fn count(data: &[u8], c: Rgba) -> usize {
        data.chunks_exact(4).filter(|p| **p == c).count()
    }

    #[test]
    fn set_get_and_out_of_bounds() {
        let mut d = vec![0u8; 4 * 4 * 3];
        set(&mut d, 4, 3, 3, 2, R);
        set(&mut d, 4, 3, 4, 0, R); // off the right edge: must not wrap to the next row
        set(&mut d, 4, 3, -1, 1, R);
        assert_eq!(get(&d, 4, 3, 3, 2), Some(R));
        assert_eq!(count(&d, R), 1);
        assert_eq!(get(&d, 4, 3, 0, 3), None);
    }

    #[test]
    fn lines_have_no_gaps_or_doubles() {
        let mut d = vec![0u8; 16 * 16 * 4];
        line(&mut d, 16, 16, (1, 1), (12, 5), 1, R);
        // A shallow line touches each column exactly once.
        assert_eq!(count(&d, R), 12);
        assert_eq!(get(&d, 16, 16, 1, 1), Some(R));
        assert_eq!(get(&d, 16, 16, 12, 5), Some(R));

        let mut d = vec![0u8; 16 * 16 * 4];
        line(&mut d, 16, 16, (5, 5), (5, 5), 3, R);
        assert_eq!(count(&d, R), 9);
        assert_eq!(get(&d, 16, 16, 4, 4), Some(R));
    }

    #[test]
    fn rect_outline_and_filled() {
        let mut d = vec![0u8; 8 * 8 * 4];
        rect(&mut d, 8, 8, (6, 5), (1, 2), false, 1, R);
        assert_eq!(count(&d, R), 2 * 6 + 2 * 2);
        rect(&mut d, 8, 8, (1, 2), (6, 5), true, 1, G);
        assert_eq!((count(&d, G), count(&d, R)), (24, 0));
    }

    #[test]
    fn fill_stops_at_borders() {
        let mut d = vec![0u8; 8 * 8 * 4];
        rect(&mut d, 8, 8, (1, 1), (5, 5), false, 1, R);
        flood_fill(&mut d, 8, 8, 3, 3, G, None);
        assert_eq!(count(&d, G), 9);
        assert_eq!(count(&d, R), 16);
        // Outside area: everything that is still clear, including around the corner.
        flood_fill(&mut d, 8, 8, 7, 7, G, None);
        assert_eq!(count(&d, CLEAR), 0);
        // Filling with the colour already there is a no-op (and must terminate).
        flood_fill(&mut d, 8, 8, 0, 0, G, None);
        assert_eq!(count(&d, G), 64 - 16);
    }

    #[test]
    fn selection_confines_painting() {
        // Select the left half of an 8x8 image.
        let mask: Vec<bool> = (0..64).map(|i| i % 8 < 4).collect();
        let original = vec![0u8; 8 * 8 * 4];

        // A fill started inside the selection stops at its edge...
        let mut d = original.clone();
        flood_fill(&mut d, 8, 8, 1, 1, R, Some(&mask));
        assert_eq!(count(&d, R), 32);
        assert_eq!(get(&d, 8, 8, 4, 0), Some(CLEAR));
        // ...and one started outside it does nothing.
        flood_fill(&mut d, 8, 8, 6, 6, G, Some(&mask));
        assert_eq!(count(&d, G), 0);

        // A stroke across the whole image only leaves its selected part behind.
        let mut d = original.clone();
        line(&mut d, 8, 8, (0, 3), (7, 3), 1, G);
        restore_outside(&mut d, &original, &mask);
        assert_eq!(count(&d, G), 4);
        assert_eq!(get(&d, 8, 8, 3, 3), Some(G));
        assert_eq!(get(&d, 8, 8, 4, 3), Some(CLEAR));

        // Magic wand: the connected area of one colour.
        let mut d = original.clone();
        rect(&mut d, 8, 8, (1, 1), (5, 5), false, 1, R);
        let inside = flood_region(&d, 8, 8, 3, 3, None);
        assert_eq!(inside.iter().filter(|b| **b).count(), 9);
        let ring = flood_region(&d, 8, 8, 1, 1, None);
        assert_eq!(ring.iter().filter(|b| **b).count(), 16);
    }

    #[test]
    fn outline_of_a_rectangle_is_four_segments() {
        let mask: Vec<bool> = (0..64).map(|i| (2..6).contains(&(i % 8)) && (1..4).contains(&(i / 8))).collect();
        let mut o = mask_outline(&mask, 8, 8);
        o.sort();
        assert_eq!(o, vec![[(2, 1), (2, 4)], [(2, 1), (6, 1)], [(2, 4), (6, 4)], [(6, 1), (6, 4)]]);
        // Selecting everything outlines the image border.
        assert_eq!(mask_outline(&vec![true; 64], 8, 8).len(), 4);
    }

    #[test]
    fn blending_shades_instead_of_replacing() {
        // 10% black over mid grey darkens it a little and stays opaque.
        assert_eq!(blend_over([200, 100, 50, 255], [0, 0, 0, 26]), [180, 90, 45, 255]);
        // Opaque paint replaces; fully transparent paint does nothing.
        assert_eq!(blend_over([200, 100, 50, 255], R), R);
        assert_eq!(blend_over([200, 100, 50, 255], [9, 9, 9, 0]), [200, 100, 50, 255]);
        // Onto a transparent pixel the colour keeps its own alpha.
        assert_eq!(blend_over(CLEAR, [10, 20, 30, 128]), [10, 20, 30, 128]);
        // Half red over half green: both contribute, alpha adds up.
        let c = blend_over([0, 255, 0, 128], [255, 0, 0, 128]);
        assert_eq!(c[3], 192);
        assert!(c[0] > c[1] && c[1] > 60);

        // A stroke that crosses itself still darkens each pixel only once.
        let base = vec![200u8; 4 * 4 * 4].chunks(4).flat_map(|_| [200, 200, 200, 255]).collect::<Vec<u8>>();
        let mut layer = vec![0u8; 4 * 4 * 4];
        let shade = [0, 0, 0, 51];
        line(&mut layer, 4, 4, (0, 1), (3, 1), 1, shade);
        line(&mut layer, 4, 4, (3, 1), (0, 1), 1, shade);
        let mut out = base.clone();
        composite(&mut out, &base, &layer, true);
        assert_eq!(get(&out, 4, 4, 2, 1), Some([160, 160, 160, 255]));
        assert_eq!(get(&out, 4, 4, 2, 2), Some([200, 200, 200, 255]));
        // With blending off the stroke's colour and alpha are written as they are.
        composite(&mut out, &base, &layer, false);
        assert_eq!(get(&out, 4, 4, 2, 1), Some(shade));
    }

    #[test]
    fn used_colors_sorted_by_use() {
        let mut d = vec![0u8; 4 * 4 * 4];
        rect(&mut d, 4, 4, (0, 0), (3, 1), true, 1, R);
        set(&mut d, 4, 4, 0, 3, G);
        assert_eq!(used_colors(&d, 8), vec![R, G]);
        assert_eq!(used_colors(&d, 1), vec![R]);
    }

    #[test]
    fn palettes() {
        let gpl = "GIMP Palette\nName: Test\nColumns: 4\n#\n255   0   0\tRed\n  0 255   0 Green\n255 0 0 again\n";
        assert_eq!(parse_palette(gpl), vec![R, G]);
        let jasc = "JASC-PAL\n0100\n2\n255 0 0\n0 255 0\n";
        assert_eq!(parse_palette(jasc), vec![R, G]);
        assert_eq!(parse_palette("ff0000\n#00FF00\n"), vec![R, G]);
        assert_eq!(parse_palette("; paint.net\n80FF0000\n"), vec![[255, 0, 0, 128]]);
        assert_eq!(image_palette(&[255, 0, 0, 255, 0, 0, 0, 0, 0, 255, 0, 255, 255, 0, 0, 255], 8), vec![R, G]);
    }

    #[test]
    fn lift_and_drop_move_pixels() {
        // 4x2 image, red at (1,0) and green at (2,0).
        let mut data = vec![0u8; 4 * 2 * 4];
        set(&mut data, 4, 2, 1, 0, R);
        set(&mut data, 4, 2, 2, 0, G);
        let mask = [false, true, true, false, false, false, false, false];
        let clip = lift(&data, 4, 2, Some(&mask)).unwrap();
        assert_eq!((clip.w, clip.h, clip.origin), (2, 1, (1, 0)));
        clear_masked(&mut data, &mask);
        assert_eq!(count(&data, CLEAR), 8);
        // Dropped one row down and one to the right; the part hanging over the edge is cut off.
        let covered = drop_clip(&mut data, 4, 2, &clip, (3, 1));
        assert_eq!(get(&data, 4, 2, 3, 1), Some(R));
        assert_eq!(covered.iter().filter(|c| **c).count(), 1);
        assert!(lift(&data, 4, 2, Some(&[false; 8])).is_none());
    }

    #[test]
    fn mirrored_brush_covers_the_mirrored_pixels() {
        for size in 1..=4u32 {
            for x in 0..8 {
                let mut a = vec![0u8; 8 * 4];
                let mut b = vec![0u8; 8 * 4];
                stamp(&mut a, 8, 1, x, 0, size, R);
                stamp(&mut b, 8, 1, mirror_coord(x, 4, size), 0, size, R);
                let flipped: Vec<u8> = a.chunks_exact(4).rev().flatten().copied().collect();
                assert_eq!(flipped, b, "size {size} at {x}");
            }
        }
    }
}
