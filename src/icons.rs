//! 16x16 pixel-art icons for the interface, drawn in code so there are no
//! image files to ship. `pixels(icon)` returns RGBA, row 0 at the top.

use crate::pixels::{self, Rgba};

pub const SIZE: u32 = 16;

const W: Rgba = [228, 229, 234, 255]; // main strokes
const G: Rgba = [128, 134, 146, 255]; // secondary strokes
const D: Rgba = [78, 83, 94, 255]; // fills / shadow
const O: Rgba = [255, 158, 44, 255]; // accent (matches the selection colour)
const B: Rgba = [92, 164, 255, 255]; // second accent
const R: Rgba = [240, 92, 92, 255]; // destructive

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Icon {
    Object, Vertex, Edge, Face,
    Click, BoxSelect, CircleSelect,
    Plane, Cube, Cylinder, Cone, Sphere,
    Move, Rotate, Scale, Snap, Origin,
    Extrude, Inset, Bevel, LoopCut, Duplicate, Delete, Flip, MergeFaces, Weld,
    UvFit, UvBox, UvView, UvRotate, FlipU, FlipV,
    New, Load, Save, Export,
    EyeOpen, EyeClosed,
    Pencil, Eraser, Fill, Picker, Line, Rect,
    // New icons go at the end so the positions in assets/icons.png never shift.
    Wand,
}

pub const ALL: [Icon; 45] = [
    Icon::Object, Icon::Vertex, Icon::Edge, Icon::Face,
    Icon::Click, Icon::BoxSelect, Icon::CircleSelect,
    Icon::Plane, Icon::Cube, Icon::Cylinder, Icon::Cone, Icon::Sphere,
    Icon::Move, Icon::Rotate, Icon::Scale, Icon::Snap, Icon::Origin,
    Icon::Extrude, Icon::Inset, Icon::Bevel, Icon::LoopCut, Icon::Duplicate, Icon::Delete, Icon::Flip, Icon::MergeFaces, Icon::Weld,
    Icon::UvFit, Icon::UvBox, Icon::UvView, Icon::UvRotate, Icon::FlipU, Icon::FlipV,
    Icon::New, Icon::Load, Icon::Save, Icon::Export,
    Icon::EyeOpen, Icon::EyeClosed,
    Icon::Pencil, Icon::Eraser, Icon::Fill, Icon::Picker, Icon::Line, Icon::Rect,
    Icon::Wand,
];

/// A 16x16 scratch canvas with a few drawing helpers.
struct C(Vec<u8>);

impl C {
    fn new() -> Self {
        C(vec![0; (SIZE * SIZE * 4) as usize])
    }
    fn p(&mut self, x: i32, y: i32, c: Rgba) {
        pixels::set(&mut self.0, SIZE, SIZE, x, y, c);
    }
    fn l(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba) {
        pixels::line(&mut self.0, SIZE, SIZE, (x0, y0), (x1, y1), 1, c);
    }
    /// Rectangle outline.
    fn r(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba) {
        pixels::rect(&mut self.0, SIZE, SIZE, (x0, y0), (x1, y1), false, 1, c);
    }
    /// Filled rectangle.
    fn f(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba) {
        pixels::rect(&mut self.0, SIZE, SIZE, (x0, y0), (x1, y1), true, 1, c);
    }
    /// Every other pixel of a horizontal / vertical line.
    fn dash(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba) {
        let n = (x1 - x0).abs().max((y1 - y0).abs());
        for i in (0..=n).step_by(2) {
            self.p(x0 + (x1 - x0).signum() * i, y0 + (y1 - y0).signum() * i, c);
        }
    }
    /// Ellipse outline between two angles (degrees, 0 = right, counter-clockwise on screen).
    fn arc(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32, c: Rgba) {
        let steps = 96;
        for i in 0..=steps {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            self.p((cx + rx * a.cos()).round() as i32, (cy - ry * a.sin()).round() as i32, c);
        }
    }
    fn pts(&mut self, pts: &[(i32, i32)], c: Rgba) {
        for &(x, y) in pts {
            self.p(x, y, c);
        }
    }
    /// The little isometric cube shared by several icons.
    fn cube(&mut self, c: Rgba) {
        self.r(2, 5, 10, 13, c);
        self.l(2, 5, 5, 2, c);
        self.l(10, 5, 13, 2, c);
        self.l(5, 2, 13, 2, c);
        self.l(13, 2, 13, 10, c);
        self.l(10, 13, 13, 10, c);
    }
}

/// Icons per row in the atlas image (`assets/icons.png`), in the order of [`ALL`].
pub const ATLAS_COLS: u32 = 11;

pub fn atlas_size() -> (u32, u32) {
    (ATLAS_COLS * SIZE, (ALL.len() as u32).div_ceil(ATLAS_COLS) * SIZE)
}

/// Cut one icon out of an atlas image. Returns None if the image is too small to contain it.
pub fn from_atlas(rgba: &[u8], width: u32, height: u32, icon: Icon) -> Option<Vec<u8>> {
    let i = ALL.iter().position(|&x| x == icon)? as u32;
    let (cx, cy) = ((i % ATLAS_COLS) * SIZE, (i / ATLAS_COLS) * SIZE);
    if cx + SIZE > width || cy + SIZE > height || rgba.len() < (width * height * 4) as usize {
        return None;
    }
    let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in cy..cy + SIZE {
        let row = ((y * width + cx) * 4) as usize;
        out.extend_from_slice(&rgba[row..row + (SIZE * 4) as usize]);
    }
    Some(out)
}

/// The built-in drawing of an icon: used to generate the default atlas, and as a
/// fallback for icons the atlas image does not contain.
pub fn pixels(icon: Icon) -> Vec<u8> {
    let mut c = C::new();
    match icon {
        Icon::Object => {
            c.f(3, 6, 9, 12, D);
            c.cube(O);
        }
        Icon::Vertex => {
            c.cube(G);
            c.f(9, 4, 11, 6, O);
        }
        Icon::Edge => {
            c.cube(G);
            c.f(10, 5, 11, 13, O);
        }
        Icon::Face => {
            c.cube(G);
            c.f(3, 6, 9, 12, O);
        }
        Icon::Click => {
            // Arrow pointer.
            for (y, w) in [(2, 1), (3, 2), (4, 3), (5, 4), (6, 5), (7, 6), (8, 7), (9, 8), (10, 4), (11, 2)] {
                c.l(4, y, 4 + w - 1, y, W);
            }
            c.f(8, 10, 9, 13, W);
            c.p(7, 10, W);
            c.p(10, 13, W);
        }
        Icon::BoxSelect => {
            c.dash(2, 3, 13, 3, W);
            c.dash(2, 12, 13, 12, W);
            c.dash(2, 3, 2, 12, W);
            c.dash(13, 4, 13, 12, W);
            c.f(6, 6, 9, 9, O);
        }
        Icon::CircleSelect => {
            c.arc(7.5, 7.5, 5.5, 5.5, 0.0, 360.0, W);
            c.f(7, 7, 8, 8, O);
        }
        Icon::Plane => {
            for i in 0..5 {
                c.l(1 + i, 10 - i, 9 + i, 10 - i, D);
            }
            c.l(1, 10, 6, 5, W);
            c.l(6, 5, 14, 5, W);
            c.l(14, 5, 9, 10, W);
            c.l(1, 10, 9, 10, W);
        }
        Icon::Cube => {
            c.f(3, 6, 9, 12, D);
            c.cube(W);
        }
        Icon::Cylinder => {
            c.f(3, 4, 12, 11, D);
            c.arc(7.5, 4.0, 5.0, 2.0, 0.0, 360.0, W);
            c.l(2, 4, 2, 11, W);
            c.l(13, 4, 13, 11, W);
            c.arc(7.5, 11.0, 5.0, 2.5, 180.0, 360.0, W);
        }
        Icon::Cone => {
            for y in 3..=11 {
                let half = (y - 1) / 2;
                c.l(7 - half + 1, y, 8 + half - 1, y, D);
            }
            c.l(7, 1, 2, 11, W);
            c.l(8, 1, 13, 11, W);
            c.arc(7.5, 11.0, 5.5, 2.5, 180.0, 360.0, W);
        }
        Icon::Sphere => {
            c.arc(7.5, 7.5, 6.0, 6.0, 0.0, 360.0, W);
            c.arc(7.5, 7.5, 6.0, 2.5, 180.0, 360.0, G);
            c.pts(&[(5, 4), (6, 3)], W);
        }
        Icon::Move => {
            c.l(7, 2, 7, 13, W);
            c.l(8, 2, 8, 13, W);
            c.l(2, 7, 13, 7, W);
            c.l(2, 8, 13, 8, W);
            c.pts(&[(6, 3), (9, 3), (5, 4), (10, 4), (6, 12), (9, 12), (5, 11), (10, 11)], W);
            c.pts(&[(3, 6), (3, 9), (4, 5), (4, 10), (12, 6), (12, 9), (11, 5), (11, 10)], W);
            c.pts(&[(7, 1), (8, 1), (7, 14), (8, 14), (1, 7), (1, 8), (14, 7), (14, 8)], W);
        }
        Icon::Rotate => {
            c.arc(7.5, 8.0, 5.0, 5.0, 60.0, 340.0, W);
            // Arrow head at the open end.
            c.pts(&[(10, 1), (10, 2), (10, 3), (10, 4), (11, 4), (12, 4), (13, 4), (11, 3), (11, 2), (12, 3)], O);
        }
        Icon::Scale => {
            c.r(2, 8, 7, 13, G);
            c.r(2, 2, 13, 13, D);
            c.l(7, 8, 13, 2, O);
            c.pts(&[(10, 2), (11, 2), (12, 2), (13, 3), (13, 4), (13, 5), (12, 3)], O);
        }
        Icon::Snap => {
            // Horseshoe magnet.
            c.f(3, 2, 5, 9, R);
            c.f(10, 2, 12, 9, R);
            c.arc(7.5, 9.0, 4.5, 4.5, 180.0, 360.0, R);
            c.arc(7.5, 9.0, 3.5, 3.5, 180.0, 360.0, R);
            c.arc(7.5, 9.0, 2.5, 2.5, 180.0, 360.0, R);
            c.f(3, 2, 5, 4, W);
            c.f(10, 2, 12, 4, W);
        }
        Icon::Origin => {
            // Target: ring, cross hairs and a centre dot.
            c.arc(7.5, 7.5, 5.0, 5.0, 0.0, 360.0, O);
            c.l(7, 0, 7, 3, W);
            c.l(8, 0, 8, 3, W);
            c.l(7, 12, 7, 15, W);
            c.l(8, 12, 8, 15, W);
            c.l(0, 7, 3, 7, W);
            c.l(0, 8, 3, 8, W);
            c.l(12, 7, 15, 7, W);
            c.l(12, 8, 15, 8, W);
            c.f(7, 7, 8, 8, W);
        }
        Icon::Extrude => {
            c.f(3, 11, 12, 13, D);
            c.r(3, 11, 12, 13, G);
            c.f(7, 3, 8, 9, O);
            c.pts(&[(7, 1), (8, 1), (6, 2), (7, 2), (8, 2), (9, 2), (5, 3), (6, 3), (9, 3), (10, 3), (4, 4), (5, 4), (10, 4), (11, 4)], O);
        }
        Icon::Inset => {
            c.r(2, 2, 13, 13, W);
            c.f(5, 5, 10, 10, D);
            c.r(5, 5, 10, 10, O);
        }
        Icon::Bevel => {
            c.l(2, 13, 2, 7, W);
            c.l(7, 2, 13, 2, W);
            c.l(2, 7, 7, 2, O);
            c.l(3, 7, 7, 3, O);
            c.dash(2, 2, 6, 2, D);
            c.dash(2, 2, 2, 6, D);
        }
        Icon::LoopCut => {
            c.r(2, 3, 13, 12, W);
            c.l(7, 1, 7, 14, O);
            c.l(8, 1, 8, 14, O);
        }
        Icon::Duplicate => {
            c.r(2, 2, 9, 9, G);
            c.f(6, 6, 13, 13, D);
            c.r(6, 6, 13, 13, W);
        }
        Icon::Delete => {
            for i in 0..10 {
                c.p(3 + i, 3 + i, R);
                c.p(4 + i, 3 + i, R);
                c.p(12 - i, 3 + i, R);
                c.p(13 - i, 3 + i, R);
            }
        }
        Icon::Flip => {
            c.f(4, 4, 5, 13, W);
            c.pts(&[(4, 2), (5, 2), (3, 3), (4, 3), (5, 3), (6, 3), (2, 4), (3, 4), (6, 4), (7, 4)], W);
            c.f(10, 2, 11, 11, O);
            c.pts(&[(10, 13), (11, 13), (9, 12), (10, 12), (11, 12), (12, 12), (8, 11), (9, 11), (12, 11), (13, 11)], O);
        }
        Icon::MergeFaces => {
            c.r(1, 4, 14, 11, W);
            c.dash(7, 5, 7, 10, G);
            c.pts(&[(3, 7), (4, 7), (5, 7), (4, 6), (4, 8), (5, 8), (5, 6)], O);
            c.pts(&[(12, 7), (11, 7), (10, 7), (11, 6), (11, 8), (10, 8), (10, 6)], O);
        }
        Icon::Weld => {
            c.f(1, 6, 3, 8, G);
            c.f(12, 6, 14, 8, G);
            c.pts(&[(5, 7), (4, 7), (10, 7), (11, 7)], W);
            c.f(6, 5, 9, 9, O);
        }
        Icon::UvFit => {
            for (x, y, dx, dy) in [(2, 2, 1, 1), (13, 2, -1, 1), (2, 13, 1, -1), (13, 13, -1, -1)] {
                c.l(x, y, x + 3 * dx, y, W);
                c.l(x, y, x, y + 3 * dy, W);
            }
            c.f(5, 5, 10, 10, O);
        }
        Icon::UvBox => {
            // Unfolded box.
            c.r(1, 6, 5, 10, W);
            c.r(5, 6, 9, 10, W);
            c.r(9, 6, 13, 10, W);
            c.r(5, 2, 9, 6, W);
            c.r(5, 10, 9, 14, W);
            c.f(6, 7, 8, 9, O);
        }
        Icon::UvView => {
            // Eye looking at a square.
            c.arc(7.5, 8.0, 6.0, 4.0, 0.0, 360.0, W);
            c.f(6, 6, 9, 9, O);
            c.f(7, 7, 8, 8, D);
        }
        Icon::UvRotate => {
            c.r(2, 6, 9, 13, G);
            c.arc(5.0, 10.0, 8.0, 8.0, 20.0, 80.0, O);
            c.pts(&[(12, 8), (13, 8), (14, 8), (13, 9), (13, 7), (12, 7), (14, 7)], O);
        }
        Icon::FlipU => {
            c.dash(7, 1, 7, 14, G);
            for i in 0..5 {
                c.l(5 - i, 3 + i, 5 - i, 12 - i, W);
                c.l(10 + i, 3 + i, 10 + i, 12 - i, O);
            }
        }
        Icon::FlipV => {
            c.dash(1, 7, 14, 7, G);
            for i in 0..5 {
                c.l(3 + i, 5 - i, 12 - i, 5 - i, W);
                c.l(3 + i, 10 + i, 12 - i, 10 + i, O);
            }
        }
        Icon::New => {
            c.f(3, 1, 12, 14, D);
            c.r(3, 1, 12, 14, W);
            c.f(7, 5, 8, 10, O);
            c.f(5, 7, 10, 8, O);
        }
        Icon::Load => {
            c.f(1, 5, 13, 13, D);
            c.l(1, 3, 5, 3, W);
            c.l(5, 3, 7, 5, W);
            c.l(7, 5, 13, 5, W);
            c.l(1, 3, 1, 13, W);
            c.l(1, 13, 13, 13, W);
            c.l(13, 5, 13, 13, W);
            c.l(3, 8, 14, 8, O);
            c.l(3, 8, 1, 13, O);
            c.l(14, 8, 13, 13, O);
        }
        Icon::Save => {
            c.f(2, 2, 13, 13, D);
            c.r(2, 2, 13, 13, W);
            c.f(5, 2, 10, 5, W);
            c.f(8, 3, 9, 4, D);
            c.f(4, 8, 11, 12, O);
        }
        Icon::Export => {
            c.l(2, 3, 2, 13, W);
            c.l(2, 13, 11, 13, W);
            c.l(11, 13, 11, 10, W);
            c.l(2, 3, 7, 3, W);
            c.f(6, 6, 12, 7, O);
            c.pts(&[(11, 4), (12, 5), (13, 6), (13, 7), (12, 8), (11, 9), (11, 5), (11, 8), (12, 6), (12, 7), (14, 6), (14, 7)], O);
        }
        Icon::EyeOpen => {
            c.arc(7.5, 8.0, 6.5, 4.0, 0.0, 360.0, W);
            c.f(6, 6, 9, 10, W);
            c.f(7, 7, 8, 9, D);
        }
        Icon::EyeClosed => {
            c.arc(7.5, 6.0, 6.5, 4.0, 180.0, 360.0, G);
            c.pts(&[(3, 11), (4, 12), (7, 12), (7, 13), (8, 12), (8, 13), (11, 12), (12, 11)], G);
        }
        Icon::Pencil => {
            for i in 0..8 {
                c.l(4 + i, 11 - i, 5 + i, 12 - i, O);
                c.p(5 + i, 11 - i, O);
            }
            c.pts(&[(12, 2), (13, 3), (12, 3), (13, 2), (11, 3), (12, 4)], R);
            c.pts(&[(2, 13), (3, 13), (2, 12), (3, 12), (4, 12), (3, 11)], W);
        }
        Icon::Eraser => {
            c.f(2, 5, 13, 11, R);
            c.f(2, 5, 6, 11, W);
            c.l(3, 13, 12, 13, G);
        }
        Icon::Fill => {
            // Tilted bucket with a drop.
            c.l(3, 7, 8, 2, W);
            c.l(8, 2, 13, 7, W);
            c.l(13, 7, 8, 12, W);
            c.l(8, 12, 3, 7, W);
            for i in 0..4 {
                c.l(5 + i, 8 + i.min(3 - i).min(0), 11 - i, 8 + i, B);
            }
            c.l(4, 7, 12, 7, B);
            c.pts(&[(13, 10), (13, 11), (14, 11), (12, 11), (13, 12), (14, 12), (12, 12), (13, 13)], B);
        }
        Icon::Picker => {
            c.f(10, 2, 13, 5, W);
            c.pts(&[(9, 5), (10, 6), (8, 4), (11, 7)], W);
            for i in 0..6 {
                c.p(8 - i, 7 + i, G);
                c.p(9 - i, 7 + i, G);
            }
            c.pts(&[(2, 13), (2, 12), (3, 13)], B);
        }
        Icon::Line => {
            c.l(3, 12, 12, 3, W);
            c.f(2, 11, 4, 13, O);
            c.f(11, 2, 13, 4, O);
        }
        Icon::Wand => {
            for i in 0..8 {
                c.p(2 + i, 13 - i, W);
                c.p(3 + i, 13 - i, G);
            }
            // Sparkle at the tip.
            c.l(12, 0, 12, 6, O);
            c.l(9, 3, 15, 3, O);
            c.pts(&[(10, 1), (14, 1), (10, 5), (14, 5)], O);
            c.p(12, 3, W);
        }
        Icon::Rect => {
            c.r(2, 4, 13, 11, W);
            c.f(1, 3, 3, 5, O);
            c.f(12, 10, 14, 12, O);
        }
    }
    c.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_round_trip() {
        let (w, h) = atlas_size();
        let mut atlas = vec![0u8; (w * h * 4) as usize];
        for (i, &icon) in ALL.iter().enumerate() {
            let (cx, cy) = ((i as u32 % ATLAS_COLS) * SIZE, (i as u32 / ATLAS_COLS) * SIZE);
            let px = pixels(icon);
            for y in 0..SIZE {
                let dst = (((cy + y) * w + cx) * 4) as usize;
                let src = (y * SIZE * 4) as usize;
                atlas[dst..dst + (SIZE * 4) as usize].copy_from_slice(&px[src..src + (SIZE * 4) as usize]);
            }
        }
        for &icon in &ALL {
            assert_eq!(from_atlas(&atlas, w, h, icon).unwrap(), pixels(icon), "{icon:?}");
        }
        // An older, shorter atlas simply lacks the newest icons.
        assert!(from_atlas(&atlas, w, SIZE, Icon::Wand).is_none());
        assert!(from_atlas(&atlas, w, SIZE, Icon::Object).is_some());
    }
}
