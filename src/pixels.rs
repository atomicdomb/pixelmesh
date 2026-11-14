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
    fn used_colors_sorted_by_use() {
        let mut d = vec![0u8; 4 * 4 * 4];
        rect(&mut d, 4, 4, (0, 0), (3, 1), true, 1, R);
        set(&mut d, 4, 4, 0, 3, G);
        assert_eq!(used_colors(&d, 8), vec![R, G]);
        assert_eq!(used_colors(&d, 1), vec![R]);
    }
}
