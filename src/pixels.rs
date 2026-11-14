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

/// Replace the 4-connected area of identical colour around (x, y).
pub fn flood_fill(data: &mut [u8], w: u32, h: u32, x: i32, y: i32, c: Rgba) {
    let Some(target) = get(data, w, h, x, y) else { return };
    if target == c {
        return;
    }
    let mut stack = vec![(x, y)];
    while let Some((px, py)) = stack.pop() {
        if get(data, w, h, px, py) != Some(target) {
            continue;
        }
        // Fill a whole horizontal run at once, then queue the rows above and below.
        let (mut l, mut r) = (px, px);
        while get(data, w, h, l - 1, py) == Some(target) {
            l -= 1;
        }
        while get(data, w, h, r + 1, py) == Some(target) {
            r += 1;
        }
        for fx in l..=r {
            set(data, w, h, fx, py, c);
        }
        for ny in [py - 1, py + 1] {
            let mut fx = l;
            while fx <= r {
                if get(data, w, h, fx, ny) == Some(target) {
                    stack.push((fx, ny));
                    // Skip the rest of this run; one seed per run is enough.
                    while fx <= r && get(data, w, h, fx, ny) == Some(target) {
                        fx += 1;
                    }
                } else {
                    fx += 1;
                }
            }
        }
    }
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
        flood_fill(&mut d, 8, 8, 3, 3, G);
        assert_eq!(count(&d, G), 9);
        assert_eq!(count(&d, R), 16);
        // Outside area: everything that is still clear, including around the corner.
        flood_fill(&mut d, 8, 8, 7, 7, G);
        assert_eq!(count(&d, CLEAR), 0);
        // Filling with the colour already there is a no-op (and must terminate).
        flood_fill(&mut d, 8, 8, 0, 0, G);
        assert_eq!(count(&d, G), 64 - 16);
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
