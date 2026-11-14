//! Mesh data model, primitives and editing operations.
//!
//! The mesh is a plain polygon soup with shared positions and *per-face-corner*
//! UVs. Per-corner UVs mean every face can sit anywhere on the sprite sheet
//! independently, which is exactly what pixel-art texturing wants.

use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Face {
    /// Indices into `Mesh::positions`, counter-clockwise when seen from the front.
    pub verts: Vec<u32>,
    /// One UV per corner, same order as `verts`. (0,0) is the bottom-left of the texture.
    pub uvs: Vec<Vec2>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub faces: Vec<Face>,
}

fn quad_uvs() -> Vec<Vec2> {
    vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)]
}

impl Mesh {
    // ---------------------------------------------------------------- queries

    pub fn face_normal(&self, f: usize) -> Vec3 {
        // Newell's method: robust for quads / n-gons that are not perfectly planar.
        let v = &self.faces[f].verts;
        let mut n = Vec3::ZERO;
        for i in 0..v.len() {
            let a = self.positions[v[i] as usize];
            let b = self.positions[v[(i + 1) % v.len()] as usize];
            n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
        n.normalize_or_zero()
    }

    pub fn face_center(&self, f: usize) -> Vec3 {
        let v = &self.faces[f].verts;
        v.iter().map(|&i| self.positions[i as usize]).sum::<Vec3>() / v.len().max(1) as f32
    }

    /// Vertices used by a set of faces.
    pub fn verts_of_faces(&self, faces: &BTreeSet<usize>) -> BTreeSet<u32> {
        faces.iter().flat_map(|&f| self.faces[f].verts.iter().copied()).collect()
    }

    /// Unique undirected edges.
    pub fn edges(&self) -> Vec<(u32, u32)> {
        let mut set = BTreeSet::new();
        for f in &self.faces {
            for i in 0..f.verts.len() {
                let (a, b) = (f.verts[i], f.verts[(i + 1) % f.verts.len()]);
                set.insert((a.min(b), a.max(b)));
            }
        }
        set.into_iter().collect()
    }

    /// Ray cast against all faces. Returns (face index, distance along ray).
    pub fn raycast(&self, origin: Vec3, dir: Vec3) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (fi, f) in self.faces.iter().enumerate() {
            let p0 = self.positions[f.verts[0] as usize];
            for i in 1..f.verts.len().saturating_sub(1) {
                let p1 = self.positions[f.verts[i] as usize];
                let p2 = self.positions[f.verts[i + 1] as usize];
                if let Some(t) = ray_triangle(origin, dir, p0, p1, p2) {
                    if best.map_or(true, |(_, bt)| t < bt) {
                        best = Some((fi, t));
                    }
                }
            }
        }
        best
    }

    /// Grow a face set to everything connected to it through shared vertices.
    pub fn linked_faces(&self, seed: &BTreeSet<usize>) -> BTreeSet<usize> {
        let mut vert_faces: HashMap<u32, Vec<usize>> = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for &v in &f.verts {
                vert_faces.entry(v).or_default().push(fi);
            }
        }
        let mut out = seed.clone();
        let mut stack: Vec<usize> = seed.iter().copied().collect();
        while let Some(f) = stack.pop() {
            for &v in &self.faces[f].verts {
                for &nf in &vert_faces[&v] {
                    if out.insert(nf) {
                        stack.push(nf);
                    }
                }
            }
        }
        out
    }

    // ------------------------------------------------------------- primitives

    /// Append another mesh; returns the indices of the newly added faces.
    pub fn append(&mut self, other: &Mesh) -> BTreeSet<usize> {
        let base = self.positions.len() as u32;
        let first = self.faces.len();
        self.positions.extend_from_slice(&other.positions);
        for f in &other.faces {
            self.faces.push(Face { verts: f.verts.iter().map(|v| v + base).collect(), uvs: f.uvs.clone() });
        }
        (first..self.faces.len()).collect()
    }

    pub fn translated(mut self, offset: Vec3) -> Mesh {
        for p in &mut self.positions {
            *p += offset;
        }
        self
    }

    /// 1x1 plane on the ground (XZ), facing up.
    pub fn plane() -> Mesh {
        let h = 0.5;
        Mesh {
            positions: vec![Vec3::new(-h, 0.0, h), Vec3::new(h, 0.0, h), Vec3::new(h, 0.0, -h), Vec3::new(-h, 0.0, -h)],
            faces: vec![Face { verts: vec![0, 1, 2, 3], uvs: quad_uvs() }],
        }
    }

    /// 1x1x1 cube sitting on the ground. Every face gets the whole texture.
    pub fn cube() -> Mesh {
        let p = |x: f32, y: f32, z: f32| Vec3::new(x - 0.5, y, z - 0.5);
        let positions = vec![
            p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(0., 1., 1.), // front (+Z)
            p(0., 0., 0.), p(1., 0., 0.), p(1., 1., 0.), p(0., 1., 0.), // back (-Z)
        ];
        let quads: [[u32; 4]; 6] = [
            [0, 1, 2, 3], // +Z
            [5, 4, 7, 6], // -Z
            [1, 5, 6, 2], // +X
            [4, 0, 3, 7], // -X
            [3, 2, 6, 7], // +Y
            [4, 5, 1, 0], // -Y
        ];
        Mesh { positions, faces: quads.iter().map(|q| Face { verts: q.to_vec(), uvs: quad_uvs() }).collect() }
    }

    /// Cylinder (or cone when `top_radius` is 0) standing on the ground.
    pub fn cylinder(segments: u32, radius: f32, top_radius: f32, height: f32) -> Mesh {
        let n = segments.max(3);
        let mut m = Mesh::default();
        let ring = |i: u32| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            // Start at +Z and go counter-clockwise seen from above.
            Vec2::new(a.sin(), a.cos())
        };
        let disc_uv = |i: u32| {
            let d = ring(i);
            Vec2::new(0.5 + d.x * 0.5, 0.5 + d.y * 0.5)
        };
        for i in 0..n {
            let d = ring(i);
            m.positions.push(Vec3::new(d.x * radius, 0.0, d.y * radius));
        }
        let cone = top_radius <= 1e-6;
        if cone {
            m.positions.push(Vec3::new(0.0, height, 0.0));
        } else {
            for i in 0..n {
                let d = ring(i);
                m.positions.push(Vec3::new(d.x * top_radius, height, d.y * top_radius));
            }
        }
        for i in 0..n {
            let j = (i + 1) % n;
            let (u0, u1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
            if cone {
                m.faces.push(Face {
                    verts: vec![i, j, n],
                    uvs: vec![Vec2::new(u0, 0.0), Vec2::new(u1, 0.0), Vec2::new((u0 + u1) * 0.5, 1.0)],
                });
            } else {
                m.faces.push(Face {
                    verts: vec![i, j, n + j, n + i],
                    uvs: vec![Vec2::new(u0, 0.0), Vec2::new(u1, 0.0), Vec2::new(u1, 1.0), Vec2::new(u0, 1.0)],
                });
            }
        }
        // Bottom cap faces down, so reverse the ring. V is mirrored to keep the image upright from below.
        m.faces.push(Face {
            verts: (0..n).rev().collect(),
            uvs: (0..n).rev().map(|i| { let u = disc_uv(i); Vec2::new(u.x, 1.0 - u.y) }).collect(),
        });
        if !cone {
            m.faces.push(Face {
                verts: (n..2 * n).collect(),
                uvs: (0..n).map(|i| { let u = disc_uv(i); Vec2::new(u.x, 1.0 - u.y) }).collect(),
            });
        }
        m
    }

    /// Lat/long sphere sitting on the ground.
    pub fn sphere(segments: u32, rings: u32, radius: f32) -> Mesh {
        let (n, r) = (segments.max(3), rings.max(2));
        let mut m = Mesh::default();
        m.positions.push(Vec3::new(0.0, 0.0, 0.0)); // south pole
        for k in 1..r {
            let phi = k as f32 / r as f32 * std::f32::consts::PI; // 0 = south pole
            let (y, rr) = (radius - radius * phi.cos(), radius * phi.sin());
            for i in 0..n {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                m.positions.push(Vec3::new(a.sin() * rr, y, a.cos() * rr));
            }
        }
        m.positions.push(Vec3::new(0.0, 2.0 * radius, 0.0)); // north pole
        let north = m.positions.len() as u32 - 1;
        let idx = |k: u32, i: u32| 1 + (k - 1) * n + (i % n);
        for i in 0..n {
            let (u0, u1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
            let v = |k: u32| k as f32 / r as f32;
            m.faces.push(Face {
                verts: vec![0, idx(1, i + 1), idx(1, i)],
                uvs: vec![Vec2::new((u0 + u1) * 0.5, 0.0), Vec2::new(u1, v(1)), Vec2::new(u0, v(1))],
            });
            for k in 1..r - 1 {
                m.faces.push(Face {
                    verts: vec![idx(k, i), idx(k, i + 1), idx(k + 1, i + 1), idx(k + 1, i)],
                    uvs: vec![Vec2::new(u0, v(k)), Vec2::new(u1, v(k)), Vec2::new(u1, v(k + 1)), Vec2::new(u0, v(k + 1))],
                });
            }
            m.faces.push(Face {
                verts: vec![idx(r - 1, i), idx(r - 1, i + 1), north],
                uvs: vec![Vec2::new(u0, v(r - 1)), Vec2::new(u1, v(r - 1)), Vec2::new((u0 + u1) * 0.5, 1.0)],
            });
        }
        m
    }

    // ---------------------------------------------------------------- editing

    /// Delete faces, then drop vertices nothing uses any more.
    pub fn delete_faces(&mut self, faces: &BTreeSet<usize>) {
        let mut i = 0;
        self.faces.retain(|_| {
            i += 1;
            !faces.contains(&(i - 1))
        });
        self.remove_unused_verts();
    }

    /// Delete vertices together with every face touching them.
    pub fn delete_verts(&mut self, verts: &BTreeSet<u32>) {
        self.faces.retain(|f| !f.verts.iter().any(|v| verts.contains(v)));
        self.remove_unused_verts();
    }

    pub fn remove_unused_verts(&mut self) {
        let mut used = vec![false; self.positions.len()];
        for f in &self.faces {
            for &v in &f.verts {
                used[v as usize] = true;
            }
        }
        let mut remap = vec![u32::MAX; self.positions.len()];
        let mut out = Vec::new();
        for (i, p) in self.positions.iter().enumerate() {
            if used[i] {
                remap[i] = out.len() as u32;
                out.push(*p);
            }
        }
        self.positions = out;
        for f in &mut self.faces {
            for v in &mut f.verts {
                *v = remap[*v as usize];
            }
        }
    }

    /// Region extrude. The selected faces are lifted onto new vertices and the
    /// open border is closed with side quads. Returns the direction to pull in
    /// (average normal); the selected face indices stay valid.
    pub fn extrude_faces(&mut self, faces: &BTreeSet<usize>) -> Vec3 {
        if faces.is_empty() {
            return Vec3::Y;
        }
        let normal = faces.iter().map(|&f| self.face_normal(f)).sum::<Vec3>().normalize_or(Vec3::Y);

        // Directed edges of the selection; an edge whose reverse is absent lies on the border.
        let mut directed: BTreeSet<(u32, u32)> = BTreeSet::new();
        for &f in faces {
            let v = &self.faces[f].verts;
            for i in 0..v.len() {
                directed.insert((v[i], v[(i + 1) % v.len()]));
            }
        }
        let mut new_index: HashMap<u32, u32> = HashMap::new();
        for v in self.verts_of_faces(faces) {
            new_index.insert(v, self.positions.len() as u32);
            self.positions.push(self.positions[v as usize]);
        }
        for &(a, b) in &directed {
            if !directed.contains(&(b, a)) {
                self.faces.push(Face { verts: vec![a, b, new_index[&b], new_index[&a]], uvs: quad_uvs() });
            }
        }
        for &f in faces {
            for v in &mut self.faces[f].verts {
                *v = new_index[v];
            }
        }
        // Vertices that were only used by the lifted faces are now orphaned
        // (e.g. extruding a lone plane); that is fine, the walls still use them.
        normal
    }

    pub fn flip_faces(&mut self, faces: &BTreeSet<usize>) {
        for &f in faces {
            self.faces[f].verts.reverse();
            self.faces[f].uvs.reverse();
        }
    }

    /// Copy faces onto fresh vertices. Returns the new face indices.
    pub fn duplicate_faces(&mut self, faces: &BTreeSet<usize>) -> BTreeSet<usize> {
        let mut new_index: HashMap<u32, u32> = HashMap::new();
        for v in self.verts_of_faces(faces) {
            new_index.insert(v, self.positions.len() as u32);
            self.positions.push(self.positions[v as usize]);
        }
        let first = self.faces.len();
        for &f in faces {
            let src = self.faces[f].clone();
            self.faces.push(Face { verts: src.verts.iter().map(|v| new_index[v]).collect(), uvs: src.uvs });
        }
        (first..self.faces.len()).collect()
    }

    /// Weld vertices closer than `eps`. Faces that collapse below a triangle are removed.
    /// Returns how many vertices were removed.
    pub fn merge_by_distance(&mut self, eps: f32) -> usize {
        let before = self.positions.len();
        let mut remap: Vec<u32> = (0..before as u32).collect();
        for i in 0..before {
            if remap[i] != i as u32 {
                continue;
            }
            for j in i + 1..before {
                if remap[j] == j as u32 && self.positions[i].distance(self.positions[j]) <= eps {
                    remap[j] = i as u32;
                }
            }
        }
        for f in &mut self.faces {
            let mut verts = Vec::new();
            let mut uvs = Vec::new();
            for (k, &v) in f.verts.iter().enumerate() {
                let nv = remap[v as usize];
                if verts.last() != Some(&nv) {
                    verts.push(nv);
                    uvs.push(f.uvs[k]);
                }
            }
            if verts.len() > 1 && verts.first() == verts.last() {
                verts.pop();
                uvs.pop();
            }
            f.verts = verts;
            f.uvs = uvs;
        }
        self.faces.retain(|f| f.verts.len() >= 3);
        self.remove_unused_verts();
        before - self.positions.len()
    }

    // --------------------------------------------------------------- UV tools

    /// Stretch each face over the whole texture (quads map corner to corner).
    pub fn uv_fit_each(&mut self, faces: &BTreeSet<usize>) {
        for &f in faces {
            if self.faces[f].verts.len() == 4 {
                self.faces[f].uvs = quad_uvs();
                continue;
            }
            let n = self.face_normal(f);
            let (t, b) = face_basis(n);
            let pts: Vec<Vec2> = self.faces[f].verts.iter().map(|&v| {
                let p = self.positions[v as usize];
                Vec2::new(p.dot(t), p.dot(b))
            }).collect();
            self.faces[f].uvs = normalize_points(&pts, false);
        }
    }

    /// Box projection at a fixed texel density: `px_per_unit` texture pixels per
    /// world unit. Geometry that sits on the grid lands on pixel boundaries.
    pub fn uv_box_project(&mut self, faces: &BTreeSet<usize>, px_per_unit: f32, tex_size: Vec2) {
        for &f in faces {
            let n = self.face_normal(f);
            let (t, b) = face_basis(snap_axis(n));
            let pts: Vec<Vec2> = self.faces[f].verts.iter().map(|&v| {
                let p = self.positions[v as usize];
                Vec2::new(p.dot(t), p.dot(b)) * px_per_unit / tex_size
            }).collect();
            // Move the face so its lower-left corner sits at the texture origin (whole pixels only).
            let min = pts.iter().fold(Vec2::splat(f32::MAX), |a, p| a.min(*p));
            let shift = (min * tex_size).floor() / tex_size;
            self.faces[f].uvs = pts.iter().map(|p| *p - shift).collect();
        }
    }

    /// Project the faces through the current view and fit the result into the texture.
    pub fn uv_project_view(&mut self, faces: &BTreeSet<usize>, view: Mat4) {
        let mut pts = Vec::new();
        for &f in faces {
            for &v in &self.faces[f].verts {
                let p = view.transform_point3(self.positions[v as usize]);
                pts.push(Vec2::new(p.x, p.y));
            }
        }
        let uvs = normalize_points(&pts, true);
        let mut k = 0;
        for &f in faces {
            for uv in &mut self.faces[f].uvs {
                *uv = uvs[k];
                k += 1;
            }
        }
    }

    pub fn uv_bounds(&self, faces: &BTreeSet<usize>) -> Option<(Vec2, Vec2)> {
        let mut it = faces.iter().flat_map(|&f| self.faces[f].uvs.iter().copied());
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    /// Apply a 2D transform to the UVs of the given faces around their common centre.
    pub fn uv_transform(&mut self, faces: &BTreeSet<usize>, op: UvOp) {
        let Some((lo, hi)) = self.uv_bounds(faces) else { return };
        let c = (lo + hi) * 0.5;
        for &f in faces {
            for uv in &mut self.faces[f].uvs {
                let d = *uv - c;
                *uv = match op {
                    UvOp::FlipU => c + Vec2::new(-d.x, d.y),
                    UvOp::FlipV => c + Vec2::new(d.x, -d.y),
                    // Rotating about the centre keeps a square (or pixel-aligned) island pixel-aligned.
                    UvOp::Rotate90 => c + Vec2::new(-d.y, d.x),
                };
            }
        }
        if op == UvOp::Rotate90 {
            // Re-anchor at the old lower-left corner so non-square islands do not drift off-grid.
            if let Some((nlo, _)) = self.uv_bounds(faces) {
                let shift = lo - nlo;
                for &f in faces {
                    for uv in &mut self.faces[f].uvs {
                        *uv += shift;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UvOp {
    FlipU,
    FlipV,
    Rotate90,
}

/// Tangent / bitangent for projecting a face with the given normal, chosen so
/// that textures appear upright on walls and unmirrored from outside.
fn face_basis(n: Vec3) -> (Vec3, Vec3) {
    if n.y.abs() > 0.999 {
        // Floor / ceiling: X to the right, "up" on the texture is away from the viewer standing at +Z.
        let t = Vec3::X;
        (t, n.cross(t))
    } else {
        let t = Vec3::Y.cross(n).normalize();
        (t, n.cross(t))
    }
}

/// Snap a normal to its dominant world axis.
fn snap_axis(n: Vec3) -> Vec3 {
    let a = n.abs();
    if a.x >= a.y && a.x >= a.z {
        Vec3::X * n.x.signum()
    } else if a.y >= a.z {
        Vec3::Y * n.y.signum()
    } else {
        Vec3::Z * n.z.signum()
    }
}

/// Fit points into the unit square. With `keep_aspect` the longer side fills it.
fn normalize_points(pts: &[Vec2], keep_aspect: bool) -> Vec<Vec2> {
    let lo = pts.iter().fold(Vec2::splat(f32::MAX), |a, p| a.min(*p));
    let hi = pts.iter().fold(Vec2::splat(f32::MIN), |a, p| a.max(*p));
    let mut size = (hi - lo).max(Vec2::splat(1e-6));
    if keep_aspect {
        size = Vec2::splat(size.max_element());
    }
    pts.iter().map(|p| (*p - lo) / size).collect()
}

fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-5).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closed and consistently wound: every directed edge has exactly one opposite.
    fn is_closed(m: &Mesh) -> bool {
        let mut set = BTreeSet::new();
        for f in &m.faces {
            for i in 0..f.verts.len() {
                if !set.insert((f.verts[i], f.verts[(i + 1) % f.verts.len()])) {
                    return false;
                }
            }
        }
        set.iter().all(|&(a, b)| set.contains(&(b, a)))
    }

    fn normals_point_out(m: &Mesh) -> bool {
        let c = m.positions.iter().copied().sum::<Vec3>() / m.positions.len() as f32;
        (0..m.faces.len()).all(|f| m.face_normal(f).dot(m.face_center(f) - c) > 0.0)
    }

    #[test]
    fn primitives_are_closed_and_outward() {
        for (name, m) in [
            ("cube", Mesh::cube()),
            ("cylinder", Mesh::cylinder(8, 0.5, 0.5, 1.0)),
            ("cone", Mesh::cylinder(8, 0.5, 0.0, 1.0)),
            ("sphere", Mesh::sphere(8, 6, 0.5)),
        ] {
            assert!(is_closed(&m), "{name} not closed");
            assert!(normals_point_out(&m), "{name} has inward faces");
            assert!(m.faces.iter().all(|f| f.verts.len() == f.uvs.len()), "{name} uv count");
        }
        assert!(Mesh::plane().face_normal(0).abs_diff_eq(Vec3::Y, 1e-6));
    }

    #[test]
    fn extrude_top_of_cube_stays_closed() {
        let mut m = Mesh::cube();
        let top: BTreeSet<usize> = [4].into();
        let n = m.extrude_faces(&top);
        assert!(n.abs_diff_eq(Vec3::Y, 1e-6));
        for v in m.verts_of_faces(&top) {
            m.positions[v as usize] += n;
        }
        assert_eq!(m.faces.len(), 10);
        assert_eq!(m.positions.len(), 12);
        assert!(is_closed(&m));
        assert!(normals_point_out(&m));
    }

    #[test]
    fn delete_and_merge() {
        let mut m = Mesh::cube();
        m.delete_faces(&[0].into());
        assert_eq!((m.faces.len(), m.positions.len()), (5, 8));

        let mut two = Mesh::cube();
        two.append(&Mesh::cube().translated(Vec3::X));
        assert_eq!(two.merge_by_distance(1e-4), 4);
        assert_eq!(two.positions.len(), 12);
    }

    #[test]
    fn box_project_is_pixel_aligned_and_upright() {
        let mut m = Mesh::cube();
        let all: BTreeSet<usize> = (0..6).collect();
        m.uv_box_project(&all, 16.0, Vec2::new(64.0, 32.0));
        for f in &m.faces {
            for uv in &f.uvs {
                let px = *uv * Vec2::new(64.0, 32.0);
                assert!((px - px.round()).abs().max_element() < 1e-3, "not on a pixel: {px}");
            }
        }
        // 16 px per unit on a 1-unit face of a 64x32 sheet.
        let (lo, hi) = m.uv_bounds(&[0].into()).unwrap();
        assert!(lo.abs_diff_eq(Vec2::ZERO, 1e-5) && hi.abs_diff_eq(Vec2::new(0.25, 0.5), 1e-5));
        // Side faces: higher on the model means higher on the texture.
        for f in 0..4 {
            let face = &m.faces[f];
            for i in 0..4 {
                for j in 0..4 {
                    let (pi, pj) = (m.positions[face.verts[i] as usize], m.positions[face.verts[j] as usize]);
                    if pi.y > pj.y {
                        assert!(face.uvs[i].y > face.uvs[j].y);
                    }
                }
            }
        }
    }

    #[test]
    fn raycast_hits_nearest_face() {
        let m = Mesh::cube();
        let (f, t) = m.raycast(Vec3::new(0.0, 0.5, 5.0), Vec3::NEG_Z).unwrap();
        assert_eq!(f, 0);
        assert!((t - 4.5).abs() < 1e-5);
    }
}
