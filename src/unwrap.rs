//! Automatic UV unwrap for low-poly, pixel-art models.
//!
//! 1. Faces are grouped into islands: neighbours stay together while the edge
//!    between them is flatter than the seam angle.
//! 2. Each island is unfolded rigidly, face by face, around the shared edges,
//!    like a paper model. Nothing is stretched, so every face keeps its true
//!    proportions and the same pixel density. A face that would land on top of
//!    another is left out and starts an island of its own.
//! 3. The islands are packed in rows onto the texture, on whole pixels.

use crate::mesh::Mesh;
use glam::{Vec2, Vec3};
use std::collections::{BTreeSet, HashMap, VecDeque};

#[derive(Clone, Copy, Debug)]
pub struct UnwrapOptions {
    /// Texture pixels per world unit.
    pub px_per_unit: f32,
    /// Texture size in pixels.
    pub texture: (u32, u32),
    /// Empty pixels left between islands.
    pub padding: u32,
    /// Edges sharper than this (degrees between the faces' normals) become seams.
    pub seam_angle: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnwrapResult {
    pub islands: usize,
    /// The density actually used: lower than asked for if the islands did not fit.
    pub px_per_unit: f32,
    /// Texture size that the requested density would have needed.
    pub needed: (u32, u32),
}

/// One unfolded island: faces with their flat coordinates in world units.
struct Island {
    mesh: usize,
    faces: Vec<(usize, Vec<Vec2>)>,
    min: Vec2,
    max: Vec2,
}

/// Unwrap the given faces of each mesh into one shared layout.
pub fn unwrap(meshes: &mut [(&mut Mesh, BTreeSet<usize>)], opt: &UnwrapOptions) -> UnwrapResult {
    let mut islands: Vec<Island> = Vec::new();
    for (i, (mesh, faces)) in meshes.iter().enumerate() {
        islands.extend(build_islands(mesh, faces, opt.seam_angle, i));
    }
    let (tw, th) = (opt.texture.0.max(1), opt.texture.1.max(1));
    if islands.is_empty() {
        return UnwrapResult { islands: 0, px_per_unit: opt.px_per_unit, needed: (0, 0) };
    }
    // Tallest first packs rows tightly.
    islands.sort_by(|a, b| (b.max.y - b.min.y).total_cmp(&(a.max.y - a.min.y)));

    let fits = |density: f32| {
        let (_, w, h) = pack(&islands, density, tw, opt.padding);
        w <= tw && h <= th
    };
    let (_, need_w, need_h) = pack(&islands, opt.px_per_unit, tw, opt.padding);
    let mut density = opt.px_per_unit;
    if !fits(density) {
        // Prefer a whole number of pixels per unit so the art stays on the pixel grid...
        let whole = (1..density.floor().max(1.0) as u32).rev().map(|d| d as f32).find(|&d| fits(d));
        density = whole.unwrap_or_else(|| {
            // ...and only go below one pixel per unit if nothing else fits.
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..30 {
                let mid = (lo + hi) * 0.5;
                if fits(mid) { lo = mid } else { hi = mid }
            }
            lo.max(1e-4)
        });
    }

    let (places, _, _) = pack(&islands, density, tw, opt.padding);
    for (island, (px, py)) in islands.iter().zip(places) {
        let mesh = &mut *meshes[island.mesh].0;
        for (face, flat) in &island.faces {
            for (k, p) in flat.iter().enumerate() {
                // Pixel position: x to the right, y down from the top of the image.
                let x = px as f32 + (p.x - island.min.x) * density;
                let y = py as f32 + (island.max.y - p.y) * density;
                mesh.faces[*face].uvs[k] = Vec2::new(x / tw as f32, 1.0 - y / th as f32);
            }
        }
    }
    UnwrapResult { islands: islands.len(), px_per_unit: density, needed: (need_w.max(1), need_h.max(1)) }
}

/// Row ("shelf") packing. Returns each island's top-left pixel and the width / height used.
fn pack(islands: &[Island], density: f32, tex_w: u32, padding: u32) -> (Vec<(u32, u32)>, u32, u32) {
    let size = |i: &Island| {
        let s = (i.max - i.min) * density;
        // A hair of tolerance so 16.000001 does not round up to 17.
        (((s.x - 1e-3).ceil().max(1.0)) as u32, ((s.y - 1e-3).ceil().max(1.0)) as u32)
    };
    let (mut x, mut y, mut row_h, mut used_w) = (0u32, 0u32, 0u32, 0u32);
    let mut places = Vec::with_capacity(islands.len());
    for island in islands {
        let (w, h) = size(island);
        if x > 0 && x + w > tex_w {
            y += row_h + padding;
            x = 0;
            row_h = 0;
        }
        places.push((x, y));
        used_w = used_w.max(x + w);
        x += w + padding;
        row_h = row_h.max(h);
    }
    (places, used_w, y + row_h)
}

fn build_islands(mesh: &Mesh, faces: &BTreeSet<usize>, seam_angle: f32, mesh_index: usize) -> Vec<Island> {
    let key = |a: u32, b: u32| (a.min(b), a.max(b));
    let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for &f in faces {
        let v = &mesh.faces[f].verts;
        for i in 0..v.len() {
            edge_faces.entry(key(v[i], v[(i + 1) % v.len()])).or_default().push(f);
        }
    }
    let normals: HashMap<usize, Vec3> = faces.iter().map(|&f| (f, mesh.face_normal(f))).collect();
    let area = |f: usize| {
        let v = &mesh.faces[f].verts;
        let p0 = mesh.positions[v[0] as usize];
        (1..v.len() - 1).map(|i| (mesh.positions[v[i] as usize] - p0).cross(mesh.positions[v[i + 1] as usize] - p0).length()).sum::<f32>()
    };
    // Start islands from the biggest faces: they make the most natural centres.
    let mut order: Vec<usize> = faces.iter().copied().collect();
    order.sort_by(|&a, &b| area(b).total_cmp(&area(a)).then(a.cmp(&b)));
    let min_cos = seam_angle.clamp(0.0, 180.0).to_radians().cos() - 1e-4;

    let mut done: BTreeSet<usize> = BTreeSet::new();
    let mut islands = Vec::new();
    for seed in order {
        if done.contains(&seed) || normals[&seed] == Vec3::ZERO {
            continue;
        }
        // The seed is laid out upright: walls keep "up" pointing up on the texture.
        let (t, b) = crate::mesh::face_basis(normals[&seed]);
        let flat_seed: Vec<Vec2> = mesh.faces[seed].verts.iter().map(|&v| {
            let p = mesh.positions[v as usize];
            Vec2::new(p.dot(t), p.dot(b))
        }).collect();
        let mut placed: Vec<(usize, Vec<Vec2>)> = vec![(seed, flat_seed)];
        done.insert(seed);
        let mut queue: VecDeque<usize> = VecDeque::from([0]);
        while let Some(at) = queue.pop_front() {
            let f = placed[at].0;
            let verts = mesh.faces[f].verts.clone();
            for i in 0..verts.len() {
                let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
                let (a2, b2) = (placed[at].1[i], placed[at].1[(i + 1) % verts.len()]);
                for &g in &edge_faces[&key(a, b)] {
                    if done.contains(&g) || normals[&g] == Vec3::ZERO || normals[&f].dot(normals[&g]) < min_cos {
                        continue;
                    }
                    let Some(flat) = unfold(mesh, g, normals[&g], a, b, a2, b2) else { continue };
                    // A face that would land on top of the island so far stays out (and seeds its own later).
                    if placed.iter().any(|(_, other)| overlap(&flat, other)) {
                        continue;
                    }
                    done.insert(g);
                    placed.push((g, flat));
                    queue.push_back(placed.len() - 1);
                }
            }
        }
        let all = placed.iter().flat_map(|(_, flat)| flat.iter().copied());
        let min = all.clone().fold(Vec2::splat(f32::MAX), Vec2::min);
        let max = all.fold(Vec2::splat(f32::MIN), Vec2::max);
        islands.push(Island { mesh: mesh_index, faces: placed, min, max });
    }
    islands
}

/// Lay face `g` flat next to a neighbour: its edge between vertices `a` and `b` goes where
/// the neighbour already put it (`a2`, `b2`), and the rest follows rigidly.
fn unfold(mesh: &Mesh, g: usize, normal: Vec3, a: u32, b: u32, a2: Vec2, b2: Vec2) -> Option<Vec<Vec2>> {
    let verts = &mesh.faces[g].verts;
    // The face in its own plane, counter-clockwise.
    let t = normal.any_orthonormal_vector();
    let bt = normal.cross(t);
    let local: Vec<Vec2> = verts.iter().map(|&v| {
        let p = mesh.positions[v as usize];
        Vec2::new(p.dot(t), p.dot(bt))
    }).collect();
    let (ia, ib) = (verts.iter().position(|&v| v == a)?, verts.iter().position(|&v| v == b)?);
    let (from, to) = (local[ib] - local[ia], b2 - a2);
    if from.length_squared() < 1e-12 || to.length_squared() < 1e-12 {
        return None;
    }
    // Rotation that turns the edge as it lies in the face into the edge as already placed.
    let angle = to.y.atan2(to.x) - from.y.atan2(from.x);
    let (sin, cos) = angle.sin_cos();
    Some(local.iter().map(|&p| {
        let d = p - local[ia];
        a2 + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
    }).collect())
}

/// Do two flat polygons cover some common area? Touching along an edge or at a corner does not count.
fn overlap(p: &[Vec2], q: &[Vec2]) -> bool {
    let bounds = |poly: &[Vec2]| (poly.iter().copied().fold(Vec2::splat(f32::MAX), Vec2::min), poly.iter().copied().fold(Vec2::splat(f32::MIN), Vec2::max));
    let ((pl, ph), (ql, qh)) = (bounds(p), bounds(q));
    let eps = 1e-4;
    if pl.x >= qh.x - eps || ql.x >= ph.x - eps || pl.y >= qh.y - eps || ql.y >= ph.y - eps {
        return false;
    }
    // Edges that properly cross.
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    for i in 0..p.len() {
        let (a, b) = (p[i], p[(i + 1) % p.len()]);
        for j in 0..q.len() {
            let (c, d) = (q[j], q[(j + 1) % q.len()]);
            let (d1, d2, d3, d4) = (cross(a, b, c), cross(a, b, d), cross(c, d, a), cross(c, d, b));
            if d1 * d2 < -eps * eps && d3 * d4 < -eps * eps {
                return true;
            }
        }
    }
    // One inside the other: test a point pulled slightly towards the middle of each.
    let inside = |pt: Vec2, poly: &[Vec2]| {
        let mut hit = false;
        let mut j = poly.len() - 1;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[j]);
            if (a.y > pt.y) != (b.y > pt.y) && pt.x < (b.x - a.x) * (pt.y - a.y) / (b.y - a.y) + a.x {
                hit = !hit;
            }
            j = i;
        }
        hit
    };
    let centre = |poly: &[Vec2]| poly.iter().copied().sum::<Vec2>() / poly.len() as f32;
    inside(centre(p), q) || inside(centre(q), p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(px: f32, tex: (u32, u32)) -> UnwrapOptions {
        UnwrapOptions { px_per_unit: px, texture: tex, padding: 1, seam_angle: 50.0 }
    }

    fn all_faces(m: &Mesh) -> BTreeSet<usize> {
        (0..m.faces.len()).collect()
    }

    /// Every face keeps its true shape: edge lengths on the texture match the model.
    fn undistorted(m: &Mesh, tex: (u32, u32), density: f32) -> bool {
        let scale = Vec2::new(tex.0 as f32, tex.1 as f32) / density;
        m.faces.iter().all(|f| (0..f.verts.len()).all(|i| {
            let j = (i + 1) % f.verts.len();
            let world = m.positions[f.verts[i] as usize].distance(m.positions[f.verts[j] as usize]);
            let flat = ((f.uvs[i] - f.uvs[j]) * scale).length();
            (world - flat).abs() < 1e-3
        }))
    }

    /// No two faces share texture area.
    fn no_overlaps(m: &Mesh) -> bool {
        (0..m.faces.len()).all(|i| (i + 1..m.faces.len()).all(|j| !overlap(&m.faces[i].uvs, &m.faces[j].uvs)))
    }

    fn in_texture(m: &Mesh) -> bool {
        m.faces.iter().flat_map(|f| &f.uvs).all(|uv| uv.is_finite() && uv.x >= -1e-5 && uv.x <= 1.0 + 1e-5 && uv.y >= -1e-5 && uv.y <= 1.0 + 1e-5)
    }

    #[test]
    fn cube_becomes_six_pixel_aligned_squares() {
        let mut m = Mesh::cube();
        let faces = all_faces(&m);
        let r = unwrap(&mut [(&mut m, faces)], &opts(16.0, (64, 64)));
        assert_eq!((r.islands, r.px_per_unit), (6, 16.0));
        assert!(undistorted(&m, (64, 64), 16.0) && no_overlaps(&m) && in_texture(&m));
        for f in &m.faces {
            for uv in &f.uvs {
                let px = *uv * 64.0;
                assert!((px - px.round()).abs().max_element() < 1e-3, "{px}");
            }
        }
        // Sides stay upright: higher on the model is higher on the texture.
        for f in m.faces.iter().filter(|f| f.verts.iter().any(|&v| m.positions[v as usize].y > 0.5) && f.verts.iter().any(|&v| m.positions[v as usize].y < 0.5)) {
            for i in 0..4 {
                for j in 0..4 {
                    if m.positions[f.verts[i] as usize].y > m.positions[f.verts[j] as usize].y {
                        assert!(f.uvs[i].y > f.uvs[j].y);
                    }
                }
            }
        }
    }

    #[test]
    fn cylinder_side_unrolls_into_one_strip() {
        let mut m = Mesh::cylinder(8, 0.5, 0.5, 1.0);
        let faces = all_faces(&m);
        let r = unwrap(&mut [(&mut m, faces)], &opts(16.0, (64, 64)));
        assert_eq!(r.islands, 3); // side strip + two caps
        assert_eq!(r.px_per_unit, 16.0);
        assert!(undistorted(&m, (64, 64), 16.0) && no_overlaps(&m) && in_texture(&m));
        // The strip is as long as the cylinder's rim: 8 x 0.3827 units = 49 px at 16 px per unit.
        let side: BTreeSet<usize> = (0..8).collect();
        let (lo, hi) = m.uv_bounds(&side).unwrap();
        assert!(((hi.x - lo.x) * 64.0 - 48.98).abs() < 0.05, "{}", (hi.x - lo.x) * 64.0);
        assert!(((hi.y - lo.y) * 64.0 - 16.0).abs() < 1e-3);
    }

    #[test]
    fn density_drops_when_the_texture_is_too_small() {
        let mut m = Mesh::cube();
        let faces = all_faces(&m);
        let r = unwrap(&mut [(&mut m, faces)], &opts(16.0, (16, 16)));
        assert!(r.px_per_unit < 16.0 && r.px_per_unit >= 1.0 && r.px_per_unit.fract() == 0.0, "{}", r.px_per_unit);
        assert!(r.needed.1 > 16);
        assert!(undistorted(&m, (16, 16), r.px_per_unit) && no_overlaps(&m) && in_texture(&m));
    }

    #[test]
    fn curved_and_bevelled_shapes_stay_valid() {
        let mut sphere = Mesh::sphere(8, 4, 0.5);
        let faces = all_faces(&sphere);
        let r = unwrap(&mut [(&mut sphere, faces)], &opts(16.0, (128, 128)));
        assert!(r.islands >= 1);
        assert!(undistorted(&sphere, (128, 128), r.px_per_unit) && no_overlaps(&sphere) && in_texture(&sphere));

        // Two objects share one layout without colliding.
        let (mut a, mut b) = (Mesh::cube(), Mesh::cylinder(6, 0.5, 0.0, 1.0));
        let (fa, fb) = (all_faces(&a), all_faces(&b));
        let r = unwrap(&mut [(&mut a, fa), (&mut b, fb)], &opts(8.0, (64, 64)));
        assert_eq!(r.px_per_unit, 8.0);
        let mut both = a.clone();
        both.append(&b);
        assert!(no_overlaps(&both) && in_texture(&both));

        // Only the chosen faces are touched.
        let mut m = Mesh::cube();
        let before = m.faces[5].uvs.clone();
        unwrap(&mut [(&mut m, [0, 1].into())], &opts(16.0, (64, 64)));
        assert_eq!(m.faces[5].uvs, before);
    }
}
