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

impl EdgeRing {
    /// The quads the ring passes through (a "face loop").
    pub fn faces(&self) -> BTreeSet<usize> {
        self.quads.iter().map(|q| q.0).collect()
    }
}

/// Cheap order-sensitive hash step (the FxHash mixer).
pub fn mix(h: u64, word: u64) -> u64 {
    (h.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95)
}

impl Mesh {
    /// A hash of the whole mesh: changes whenever any position, face or UV changes.
    /// Fast enough to compute for every object every frame; used to reuse cached draw data.
    pub fn fingerprint(&self) -> u64 {
        let mut h = mix(self.positions.len() as u64, self.faces.len() as u64);
        for p in &self.positions {
            h = mix(h, (p.x.to_bits() as u64) << 32 | p.y.to_bits() as u64);
            h = mix(h, p.z.to_bits() as u64);
        }
        for f in &self.faces {
            h = mix(h, f.verts.len() as u64);
            for (v, uv) in f.verts.iter().zip(&f.uvs) {
                h = mix(h, *v as u64);
                h = mix(h, (uv.x.to_bits() as u64) << 32 | uv.y.to_bits() as u64);
            }
        }
        h
    }

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

    /// Split a face into triangles (as corner indices). Uses ear clipping, so
    /// concave polygons, which merging faces readily produces, come out right.
    pub fn triangulate(&self, f: usize) -> Vec<[usize; 3]> {
        let verts = &self.faces[f].verts;
        let n = verts.len();
        if n < 3 {
            return Vec::new();
        }
        if n == 3 {
            return vec![[0, 1, 2]];
        }
        let fan = |idx: &[usize]| (1..idx.len() - 1).map(|i| [idx[0], idx[i], idx[i + 1]]).collect::<Vec<_>>();
        let normal = self.face_normal(f);
        if normal == Vec3::ZERO {
            return fan(&(0..n).collect::<Vec<_>>());
        }
        // 2D coordinates in the face plane, counter-clockwise.
        let t = normal.any_orthonormal_vector();
        let b = normal.cross(t);
        let pts: Vec<Vec2> = verts.iter().map(|&v| {
            let p = self.positions[v as usize];
            Vec2::new(p.dot(t), p.dot(b))
        }).collect();
        let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
        let mut idx: Vec<usize> = (0..n).collect();
        let mut tris = Vec::with_capacity(n - 2);
        while idx.len() > 3 {
            let m = idx.len();
            let ear = (0..m).find(|&i| {
                let (p, c, nx) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
                if cross(pts[p], pts[c], pts[nx]) <= 1e-12 {
                    return false; // reflex or flat corner
                }
                // No other corner may lie inside the candidate triangle, or on its border.
                let eps = -1e-9;
                idx.iter().all(|&o| {
                    o == p || o == c || o == nx
                        || !(cross(pts[p], pts[c], pts[o]) >= eps && cross(pts[c], pts[nx], pts[o]) >= eps && cross(pts[nx], pts[p], pts[o]) >= eps)
                })
            });
            match ear {
                Some(i) => {
                    tris.push([idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]]);
                    idx.remove(i);
                }
                None => {
                    // Degenerate polygon: fall back to a fan for what is left.
                    tris.extend(fan(&idx));
                    return tris;
                }
            }
        }
        tris.push([idx[0], idx[1], idx[2]]);
        tris
    }

    /// The edge loop through edge (a, b): it continues at each end through the
    /// one edge that does not share a triangle or quad with the current edge
    /// (straight on through a grid; along the rim of a cap).
    pub fn edge_loop(&self, a: u32, b: u32) -> Vec<(u32, u32)> {
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        let mut edge_faces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        let mut vert_edges: HashMap<u32, BTreeSet<(u32, u32)>> = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for i in 0..f.verts.len() {
                let e = key(f.verts[i], f.verts[(i + 1) % f.verts.len()]);
                edge_faces.entry(e).or_default().push(fi);
                vert_edges.entry(e.0).or_default().insert(e);
                vert_edges.entry(e.1).or_default().insert(e);
            }
        }
        let start = key(a, b);
        if !edge_faces.contains_key(&start) {
            return Vec::new();
        }
        let share_small_face = |e1: (u32, u32), e2: (u32, u32)| {
            edge_faces[&e1].iter().any(|f| self.faces[*f].verts.len() <= 4 && edge_faces[&e2].contains(f))
        };
        let mut seen: BTreeSet<(u32, u32)> = [start].into();
        let mut result = vec![start];
        for mut v in [b, a] {
            let mut cur = start;
            loop {
                let next: Vec<(u32, u32)> = vert_edges[&v].iter().copied().filter(|&e| e != cur && !share_small_face(cur, e)).collect();
                let [e] = next[..] else { break };
                if !seen.insert(e) {
                    break;
                }
                result.push(e);
                v = if e.0 == v { e.1 } else { e.0 };
                cur = e;
            }
        }
        result
    }

    /// Join each connected group of the given faces into a single polygon.
    /// Groups whose outline is not one simple loop (a hole, or two parts
    /// touching at a point) are left alone. Returns the new faces and how
    /// many groups were skipped.
    pub fn merge_faces(&mut self, faces: &BTreeSet<usize>) -> (BTreeSet<usize>, usize) {
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        // Group the faces by shared edges.
        let mut edge_owner: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for &f in faces {
            let v = &self.faces[f].verts;
            for i in 0..v.len() {
                edge_owner.entry(key(v[i], v[(i + 1) % v.len()])).or_default().push(f);
            }
        }
        let mut group_of: HashMap<usize, usize> = HashMap::new();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for &seed in faces {
            if group_of.contains_key(&seed) {
                continue;
            }
            let id = groups.len();
            let mut members = vec![seed];
            group_of.insert(seed, id);
            let mut i = 0;
            while i < members.len() {
                let v = self.faces[members[i]].verts.clone();
                for k in 0..v.len() {
                    for &other in &edge_owner[&key(v[k], v[(k + 1) % v.len()])] {
                        if !group_of.contains_key(&other) {
                            group_of.insert(other, id);
                            members.push(other);
                        }
                    }
                }
                i += 1;
            }
            groups.push(members);
        }

        let mut merged: Vec<Face> = Vec::new();
        let mut remove: BTreeSet<usize> = BTreeSet::new();
        let mut skipped = 0;
        for members in groups.iter().filter(|g| g.len() >= 2) {
            let inside: BTreeSet<usize> = members.iter().copied().collect();
            let mut directed: HashMap<(u32, u32), Vec2> = HashMap::new(); // edge -> uv at its start corner
            for &f in members {
                let face = &self.faces[f];
                for i in 0..face.verts.len() {
                    directed.insert((face.verts[i], face.verts[(i + 1) % face.verts.len()]), face.uvs[i]);
                }
            }
            // Outline: edges whose opposite direction is not in the group.
            let mut next: HashMap<u32, (u32, Vec2)> = HashMap::new();
            let mut simple = true;
            let mut outline_len = 0;
            for (&(a, b), &uv) in &directed {
                if !directed.contains_key(&(b, a)) {
                    outline_len += 1;
                    simple &= next.insert(a, (b, uv)).is_none();
                }
            }
            let Some(&first) = next.keys().min() else {
                skipped += 1;
                continue;
            };
            let mut verts = Vec::new();
            let mut uvs = Vec::new();
            let mut cur = first;
            while simple && verts.len() <= outline_len {
                let Some(&(to, uv)) = next.get(&cur) else {
                    simple = false;
                    break;
                };
                verts.push(cur);
                uvs.push(uv);
                cur = to;
                if cur == first {
                    break;
                }
            }
            if !simple || verts.len() != outline_len || verts.len() < 3 {
                skipped += 1;
                continue;
            }
            // Drop corners that are now just a point on a straight edge and that nothing else uses.
            let used_outside: BTreeSet<u32> = self.faces.iter().enumerate()
                .filter(|(i, _)| !inside.contains(i))
                .flat_map(|(_, f)| f.verts.iter().copied())
                .collect();
            let mut i = 0;
            while verts.len() > 3 && i < verts.len() {
                let n = verts.len();
                let (p, c, nx) = (self.positions[verts[(i + n - 1) % n] as usize], self.positions[verts[i] as usize], self.positions[verts[(i + 1) % n] as usize]);
                let straight = (c - p).cross(nx - c).length() <= 1e-5 * (c - p).length().max(1e-6) * (nx - c).length().max(1e-6);
                if straight && !used_outside.contains(&verts[i]) {
                    verts.remove(i);
                    uvs.remove(i);
                } else {
                    i += 1;
                }
            }
            merged.push(Face { verts, uvs });
            remove.extend(members);
        }
        // Add first, then delete, so vertices kept alive only by the new faces survive the clean-up.
        let count = merged.len();
        self.faces.extend(merged);
        self.delete_faces(&remove);
        ((self.faces.len() - count..self.faces.len()).collect(), skipped)
    }

    /// Ray cast against all faces. Returns (face index, distance along ray).
    pub fn raycast(&self, origin: Vec3, dir: Vec3) -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (fi, f) in self.faces.iter().enumerate() {
            for [a, b, c] in self.triangulate(fi) {
                let (p0, p1, p2) = (self.positions[f.verts[a] as usize], self.positions[f.verts[b] as usize], self.positions[f.verts[c] as usize]);
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
        // Only vertices on the border are duplicated; vertices inside the region simply move with it.
        let border: Vec<(u32, u32)> = directed.iter().copied().filter(|&(a, b)| !directed.contains(&(b, a))).collect();
        let mut new_index: HashMap<u32, u32> = HashMap::new();
        for &(a, b) in &border {
            for v in [a, b] {
                if !new_index.contains_key(&v) {
                    new_index.insert(v, self.positions.len() as u32);
                    self.positions.push(self.positions[v as usize]);
                }
            }
        }
        for &(a, b) in &border {
            self.faces.push(Face { verts: vec![a, b, new_index[&b], new_index[&a]], uvs: quad_uvs() });
        }
        for &f in faces {
            for v in &mut self.faces[f].verts {
                if let Some(&nv) = new_index.get(v) {
                    *v = nv;
                }
            }
        }
        normal
    }

    /// Map from a directed edge (a -> b) to the face that contains it in that order.
    fn directed_edge_faces(&self) -> HashMap<(u32, u32), usize> {
        let mut map = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for i in 0..f.verts.len() {
                map.insert((f.verts[i], f.verts[(i + 1) % f.verts.len()]), fi);
            }
        }
        map
    }

    /// UV for a point `p` lying near corner `k` of face `f`, extrapolated from that
    /// corner and its two neighbours so the texture stays exactly where it was.
    fn corner_uv(&self, f: usize, k: usize, p: Vec3) -> Vec2 {
        let face = &self.faces[f];
        let n = face.verts.len();
        let (kp, kn) = ((k + n - 1) % n, (k + 1) % n);
        let o = self.positions[face.verts[k] as usize];
        let e1 = self.positions[face.verts[kp] as usize] - o;
        let e2 = self.positions[face.verts[kn] as usize] - o;
        let d = p - o;
        let (a11, a12, a22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
        let det = a11 * a22 - a12 * a12;
        if det.abs() < 1e-12 {
            return face.uvs[k];
        }
        let (b1, b2) = (d.dot(e1), d.dot(e2));
        let a = (b1 * a22 - b2 * a12) / det;
        let b = (a11 * b2 - a12 * b1) / det;
        face.uvs[k] + a * (face.uvs[kp] - face.uvs[k]) + b * (face.uvs[kn] - face.uvs[k])
    }

    /// Inset the selected faces as one region: the region's border moves inwards
    /// by `t` and a rim of quads fills the gap. Face indices stay valid (the
    /// selected faces become the inner faces). UVs are kept in place.
    pub fn inset_faces(&mut self, faces: &BTreeSet<usize>, t: f32) {
        if t <= 0.0 || faces.is_empty() {
            return;
        }
        let orig = self.clone();
        let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
        for &f in faces {
            let v = &orig.faces[f].verts;
            for i in 0..v.len() {
                directed.insert((v[i], v[(i + 1) % v.len()]), f);
            }
        }
        let mut border: Vec<((u32, u32), usize)> =
            directed.iter().filter(|(&(a, b), _)| !directed.contains_key(&(b, a))).map(|(&e, &f)| (e, f)).collect();
        border.sort();
        if border.is_empty() {
            return;
        }
        // Border edge arriving at / leaving each border vertex.
        let mut incoming: HashMap<u32, (u32, usize)> = HashMap::new();
        let mut outgoing: HashMap<u32, (u32, usize)> = HashMap::new();
        for &((a, b), f) in &border {
            outgoing.insert(a, (b, f));
            incoming.insert(b, (a, f));
        }
        let mut new_index: HashMap<u32, u32> = HashMap::new();
        let mut verts: Vec<u32> = outgoing.keys().copied().collect();
        verts.sort();
        for v in verts {
            let (Some(&(next, f_out)), Some(&(prev, f_in))) = (outgoing.get(&v), incoming.get(&v)) else { continue };
            let p = orig.positions[v as usize];
            let d_in = (p - orig.positions[prev as usize]).normalize_or_zero();
            let d_out = (orig.positions[next as usize] - p).normalize_or_zero();
            // Normal x direction points to the left of the edge, i.e. into the (counter-clockwise) face.
            let n_in = orig.face_normal(f_in).cross(d_in);
            let n_out = orig.face_normal(f_out).cross(d_out);
            let offset = (n_in + n_out) / (1.0 + n_in.dot(n_out)).max(0.05) * t;
            new_index.insert(v, self.positions.len() as u32);
            self.positions.push(p + offset);
        }
        for &f in faces {
            for k in 0..orig.faces[f].verts.len() {
                if let Some(&nv) = new_index.get(&orig.faces[f].verts[k]) {
                    self.faces[f].verts[k] = nv;
                    self.faces[f].uvs[k] = orig.corner_uv(f, k, self.positions[nv as usize]);
                }
            }
        }
        for &((a, b), f) in &border {
            let (Some(&na), Some(&nb)) = (new_index.get(&a), new_index.get(&b)) else { continue };
            let ka = orig.faces[f].verts.iter().zip(orig.faces[f].verts.iter().cycle().skip(1)).position(|(&x, &y)| x == a && y == b).unwrap();
            let kb = (ka + 1) % orig.faces[f].verts.len();
            self.faces.push(Face {
                verts: vec![a, b, nb, na],
                uvs: vec![orig.faces[f].uvs[ka], orig.faces[f].uvs[kb], self.faces[f].uvs[kb], self.faces[f].uvs[ka]],
            });
        }
    }

    /// Chamfer edges: every given edge is replaced by a flat strip of width
    /// `w` (measured along the neighbouring faces), with a small cap where three
    /// or more bevelled edges meet. Edges that do not have exactly two faces are
    /// skipped. Returns the indices of the new faces.
    pub fn bevel_edges(&mut self, edges: &BTreeSet<(u32, u32)>, w: f32) -> BTreeSet<usize> {
        if w <= 0.0 {
            return BTreeSet::new();
        }
        let orig = self.clone();
        let dir = orig.directed_edge_faces();
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        let bev: BTreeSet<(u32, u32)> = edges.iter()
            .filter(|&&(a, b)| dir.contains_key(&(a, b)) && dir.contains_key(&(b, a)))
            .map(|&(a, b)| key(a, b))
            .collect();
        if bev.is_empty() {
            return BTreeSet::new();
        }
        let is_bev = |a: u32, b: u32| bev.contains(&key(a, b));
        let bev_verts: BTreeSet<u32> = bev.iter().flat_map(|&(a, b)| [a, b]).collect();

        // Face corners sitting on each bevel vertex: (face, corner index).
        let mut corners: HashMap<u32, Vec<(usize, usize)>> = HashMap::new();
        for (fi, f) in orig.faces.iter().enumerate() {
            for (k, v) in f.verts.iter().enumerate() {
                if bev_verts.contains(v) {
                    corners.entry(*v).or_default().push((fi, k));
                }
            }
        }

        // New vertex for every (face, old vertex) corner; corners in the same group share one.
        let mut corner_vertex: HashMap<(usize, u32), u32> = HashMap::new();
        let mut caps: Vec<(Vec<u32>, Vec2)> = Vec::new();
        // Extra corners to add to existing faces: (face, position in the original corner list, vertex, uv).
        let mut inserts: Vec<(usize, usize, u32, Vec2)> = Vec::new();
        for &v in &bev_verts {
            let cs = &corners[&v];
            let slot: HashMap<usize, usize> = cs.iter().enumerate().map(|(i, &(f, _))| (f, i)).collect();
            let prev_next = |&(f, k): &(usize, usize)| {
                let fv = &orig.faces[f].verts;
                (fv[(k + fv.len() - 1) % fv.len()], fv[(k + 1) % fv.len()])
            };
            // Group corners that are only separated by edges which stay sharp.
            let mut group: Vec<usize> = (0..cs.len()).collect();
            fn root(g: &mut Vec<usize>, mut i: usize) -> usize {
                while g[i] != i {
                    g[i] = g[g[i]];
                    i = g[i];
                }
                i
            }
            for (i, c) in cs.iter().enumerate() {
                let (_, next) = prev_next(c);
                if !is_bev(v, next) {
                    if let Some(j) = dir.get(&(next, v)).and_then(|g| slot.get(g)) {
                        let (a, b) = (root(&mut group, i), root(&mut group, *j));
                        group[a] = b;
                    }
                }
            }
            let roots: Vec<usize> = (0..cs.len()).map(|i| root(&mut group, i)).collect();
            let distinct: BTreeSet<usize> = roots.iter().copied().collect();
            let p = orig.positions[v as usize];

            // A single bevelled edge ending at this vertex: the strip's two corners
            // slide out along the neighbouring sharp edges and the faces in between
            // absorb them (plus a small triangle when there are several such faces).
            if distinct.len() == 1 {
                let ends: Vec<u32> = bev.iter().filter_map(|&(a, b)| if a == v { Some(b) } else if b == v { Some(a) } else { None }).collect();
                if let [x] = ends[..] {
                    let (f1, f2) = (dir[&(v, x)], dir[&(x, v)]);
                    // Faces between f2 and f1, walking the fan across each face's "next" edge.
                    let mut between = Vec::new();
                    let mut cur = f2;
                    let mut closed = false;
                    for _ in 0..cs.len() {
                        let (_, next) = prev_next(&cs[slot[&cur]]);
                        match dir.get(&(next, v)) {
                            Some(&g) if g == f1 => {
                                closed = true;
                                break;
                            }
                            Some(&g) => {
                                between.push(g);
                                cur = g;
                            }
                            None => break,
                        }
                    }
                    if closed && !between.is_empty() {
                        let unit = |to: u32| (orig.positions[to as usize] - p).normalize_or_zero();
                        let slide = |along: Vec3| p + along * w / along.cross(unit(x)).length().max(0.05);
                        let (k1, k2) = (cs[slot[&f1]].1, cs[slot[&f2]].1);
                        let (prev1, _) = prev_next(&(f1, k1));
                        let (_, next2) = prev_next(&(f2, k2));
                        let p1 = self.positions.len() as u32;
                        self.positions.push(slide(unit(prev1)));
                        let p2 = self.positions.len() as u32;
                        self.positions.push(slide(unit(next2)));
                        for (f, k, nv) in [(f1, k1, p1), (f2, k2, p2)] {
                            self.faces[f].verts[k] = nv;
                            self.faces[f].uvs[k] = orig.corner_uv(f, k, self.positions[nv as usize]);
                        }
                        for &(f, _) in cs {
                            corner_vertex.insert((f, v), v);
                        }
                        corner_vertex.insert((f1, v), p1);
                        corner_vertex.insert((f2, v), p2);
                        let (g_first, g_last) = (between[0], *between.last().unwrap());
                        let (kf, kl) = (cs[slot[&g_first]].1, cs[slot[&g_last]].1);
                        let uv_first = orig.corner_uv(g_first, kf, self.positions[p2 as usize]);
                        let uv_last = orig.corner_uv(g_last, kl, self.positions[p1 as usize]);
                        if between.len() == 1 {
                            // The one face in between swaps the old corner for both new points.
                            self.faces[g_first].verts[kf] = p2;
                            self.faces[g_first].uvs[kf] = uv_first;
                            inserts.push((g_first, kf + 1, p1, uv_last));
                        } else {
                            inserts.push((g_first, kf, p2, uv_first));
                            inserts.push((g_last, kl + 1, p1, uv_last));
                            caps.push((vec![v, p2, p1], orig.faces[g_first].uvs[kf]));
                        }
                        continue;
                    }
                }
            }

            let mut vertex_of_root: HashMap<usize, u32> = HashMap::new();
            for &r in &distinct {
                if distinct.len() == 1 {
                    // A bevel that simply ends here: the strip tapers into the old vertex.
                    vertex_of_root.insert(r, v);
                    continue;
                }
                let members: Vec<&(usize, usize)> = cs.iter().enumerate().filter(|(i, _)| roots[*i] == r).map(|(_, c)| c).collect();
                let unit = |to: u32| (orig.positions[to as usize] - p).normalize_or_zero();
                let offset = if members.len() == 1 {
                    let c = members[0];
                    let (prev, next) = prev_next(c);
                    let normal = orig.face_normal(c.0);
                    let (d_in, d_out) = (-unit(prev), unit(next));
                    match (is_bev(prev, v), is_bev(v, next)) {
                        // Both edges of this corner are bevelled: move inwards from both, like an inset corner.
                        (true, true) => {
                            let (n_in, n_out) = (normal.cross(d_in), normal.cross(d_out));
                            (n_in + n_out) / (1.0 + n_in.dot(n_out)).max(0.05) * w
                        }
                        // Only one is: slide along the other edge until `w` away from the bevelled one.
                        (true, false) => d_out * w / d_in.cross(d_out).length().max(0.05),
                        (false, true) => -d_in * w / d_in.cross(d_out).length().max(0.05),
                        (false, false) => Vec3::ZERO,
                    }
                } else {
                    // Several faces share this corner: slide along the sharp edge(s) between them.
                    let mut along = Vec3::ZERO;
                    let mut bounding = Vec::new();
                    for c in &members {
                        let (prev, next) = prev_next(c);
                        if is_bev(v, next) {
                            bounding.push(unit(next));
                        } else if dir.contains_key(&(next, v)) {
                            along += unit(next);
                        }
                        if is_bev(prev, v) {
                            bounding.push(unit(prev));
                        }
                    }
                    let along = along.normalize_or_zero();
                    let dist = bounding.iter().map(|e| w / along.cross(*e).length().max(0.05)).sum::<f32>() / bounding.len().max(1) as f32;
                    along * dist
                };
                vertex_of_root.insert(r, self.positions.len() as u32);
                self.positions.push(p + offset);
            }
            for (i, &(f, k)) in cs.iter().enumerate() {
                let nv = vertex_of_root[&roots[i]];
                corner_vertex.insert((f, v), nv);
                if nv != v {
                    self.faces[f].verts[k] = nv;
                    self.faces[f].uvs[k] = orig.corner_uv(f, k, self.positions[nv as usize]);
                }
            }
            // Three or more groups leave a hole at the old vertex: close it with a cap.
            if distinct.len() >= 3 {
                // Crossing a face's "next" edge walks the fan clockwise (seen from outside).
                let mut ring = Vec::new();
                let mut i = 0;
                let mut closed = false;
                for _ in 0..cs.len() {
                    let nv = vertex_of_root[&roots[i]];
                    if ring.last() != Some(&nv) {
                        ring.push(nv);
                    }
                    let (_, next) = prev_next(&cs[i]);
                    match dir.get(&(next, v)).and_then(|g| slot.get(g)) {
                        Some(&j) => i = j,
                        None => break,
                    }
                    if i == 0 {
                        closed = true;
                        break;
                    }
                }
                if ring.len() > 1 && ring.first() == ring.last() {
                    ring.pop();
                }
                if closed && ring.len() >= 3 {
                    ring.reverse();
                    // One texel just inside the first face, so the cap is a single flat colour.
                    let (f0, k0) = cs[0];
                    caps.push((ring, self.faces[f0].uvs[k0]));
                }
            }
        }

        let first_new = self.faces.len();
        for &(u, v) in &bev {
            let (fa, fb) = (dir[&(u, v)], dir[&(v, u)]);
            let old_uv = |f: usize, vert: u32| orig.faces[f].uvs[orig.faces[f].verts.iter().position(|&x| x == vert).unwrap()];
            let new_uv = |f: usize, vert: u32| {
                let k = orig.faces[f].verts.iter().position(|&x| x == vert).unwrap();
                self.faces[f].uvs[k]
            };
            // The strip takes the sliver of texture that face A just gave up.
            let quad = [
                (corner_vertex[&(fa, v)], new_uv(fa, v)),
                (corner_vertex[&(fa, u)], new_uv(fa, u)),
                (corner_vertex[&(fb, u)], old_uv(fa, u)),
                (corner_vertex[&(fb, v)], old_uv(fa, v)),
            ];
            let mut verts = Vec::new();
            let mut uvs = Vec::new();
            for (vert, uv) in quad {
                if verts.last() != Some(&vert) && verts.first() != Some(&vert) {
                    verts.push(vert);
                    uvs.push(uv);
                }
            }
            if verts.len() >= 3 {
                self.faces.push(Face { verts, uvs });
            }
        }
        // Applied last and from the back, so the corner indices used above stay valid.
        inserts.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
        for (f, at, vert, uv) in inserts {
            self.faces[f].verts.insert(at, vert);
            self.faces[f].uvs.insert(at, uv);
        }
        for (ring, uv) in caps {
            let uvs = vec![uv; ring.len()];
            self.faces.push(Face { verts: ring, uvs });
        }
        let new_faces = (first_new..self.faces.len()).collect();
        self.remove_unused_verts();
        new_faces
    }

    /// The ring of edges a loop cut through edge (a, b) would cross: it runs
    /// from quad to quad over opposite edges until it closes or meets a face
    /// that is not a quad.
    pub fn edge_ring(&self, a: u32, b: u32) -> EdgeRing {
        let dir = self.directed_edge_faces();
        let mut ring = EdgeRing { edges: vec![(a, b)], closed: false, quads: Vec::new(), ends: Vec::new() };
        let mut visited: BTreeSet<usize> = BTreeSet::new();

        // Forwards: through faces that contain the edge as a -> b.
        let mut cur = (a, b);
        while let Some(&f) = dir.get(&cur) {
            let fv = &self.faces[f].verts;
            if fv.len() != 4 {
                ring.ends.push((f, cur));
                break;
            }
            if !visited.insert(f) {
                break;
            }
            let i = (0..4).find(|&i| fv[i] == cur.0 && fv[(i + 1) % 4] == cur.1).unwrap();
            let next = (fv[(i + 3) % 4], fv[(i + 2) % 4]);
            ring.quads.push((f, cur, next));
            if next == (a, b) {
                ring.closed = true;
                break;
            }
            ring.edges.push(next);
            cur = next;
        }
        // Backwards: through faces that contain it as b -> a.
        if !ring.closed {
            let mut cur = (a, b);
            while let Some(&f) = dir.get(&(cur.1, cur.0)) {
                let fv = &self.faces[f].verts;
                if fv.len() != 4 {
                    ring.ends.push((f, cur));
                    break;
                }
                if !visited.insert(f) {
                    break;
                }
                let i = (0..4).find(|&i| fv[i] == cur.1 && fv[(i + 1) % 4] == cur.0).unwrap();
                let next = (fv[(i + 2) % 4], fv[(i + 3) % 4]);
                ring.quads.push((f, cur, next));
                ring.edges.insert(0, next);
                cur = next;
            }
        }
        ring
    }

    /// Cut every edge of the ring at the given fractions (0..1, measured from
    /// the first vertex of each ring edge) and split the quads in between.
    /// Returns the new vertices.
    pub fn loop_cut(&mut self, ring: &EdgeRing, fractions: &[f32]) -> Vec<u32> {
        let n = fractions.len();
        if n == 0 {
            return Vec::new();
        }
        let mut cut: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        let mut created = Vec::new();
        for &(a, b) in &ring.edges {
            let (pa, pb) = (self.positions[a as usize], self.positions[b as usize]);
            let ids: Vec<u32> = fractions.iter().map(|&t| {
                self.positions.push(pa.lerp(pb, t));
                self.positions.len() as u32 - 1
            }).collect();
            created.extend(&ids);
            cut.insert((a, b), ids);
        }
        for &(f, e_in, e_out) in &ring.quads {
            let face = self.faces[f].clone();
            let uv_of = |v: u32| face.uvs[face.verts.iter().position(|&x| x == v).unwrap()];
            // Points from the "a" end to the "b" end of an edge, with their UVs in this face.
            let rail = |e: (u32, u32)| -> Vec<(u32, Vec2)> {
                let (ua, ub) = (uv_of(e.0), uv_of(e.1));
                let mut pts = vec![(e.0, ua)];
                pts.extend(cut[&e].iter().zip(fractions).map(|(&v, &t)| (v, ua.lerp(ub, t))));
                pts.push((e.1, ub));
                pts
            };
            let (xs, ys) = (rail(e_in), rail(e_out));
            let i = face.verts.iter().position(|&x| x == e_in.0).unwrap();
            let forward = face.verts[(i + 1) % 4] == e_in.1;
            for j in 0..=n {
                let c = if forward { [xs[j], xs[j + 1], ys[j + 1], ys[j]] } else { [xs[j + 1], xs[j], ys[j], ys[j + 1]] };
                let piece = Face { verts: c.iter().map(|x| x.0).collect(), uvs: c.iter().map(|x| x.1).collect() };
                if j == 0 {
                    self.faces[f] = piece;
                } else {
                    self.faces.push(piece);
                }
            }
        }
        // Faces at the ends of an open ring just get the new vertices added to their edge.
        for &(f, e) in &ring.ends {
            let face = &mut self.faces[f];
            let len = face.verts.len();
            let Some(i) = (0..len).find(|&i| {
                let (x, y) = (face.verts[i], face.verts[(i + 1) % len]);
                (x, y) == e || (y, x) == e
            }) else { continue };
            let forward = face.verts[i] == e.0;
            let (uv_i, uv_next) = (face.uvs[i], face.uvs[(i + 1) % len]);
            let (ua, ub) = if forward { (uv_i, uv_next) } else { (uv_next, uv_i) };
            let mut extra: Vec<(u32, Vec2)> = cut[&e].iter().zip(fractions).map(|(&v, &t)| (v, ua.lerp(ub, t))).collect();
            if !forward {
                extra.reverse();
            }
            for (k, (v, uv)) in extra.into_iter().enumerate() {
                face.verts.insert(i + 1 + k, v);
                face.uvs.insert(i + 1 + k, uv);
            }
        }
        created
    }

    /// This mesh plus its reflection in the plane through `point` with the given
    /// normal. Vertices lying on the plane (within `eps`) are shared by both
    /// halves, so the seam is welded; faces lying wholly in the plane are not doubled.
    pub fn mirrored(&self, point: Vec3, normal: Vec3, eps: f32) -> Mesh {
        let n = normal.normalize_or_zero();
        if n == Vec3::ZERO {
            return self.clone();
        }
        let mut out = self.clone();
        let twin: Vec<u32> = self.positions.iter().enumerate().map(|(i, &p)| {
            let d = (p - point).dot(n);
            if d.abs() <= eps {
                i as u32
            } else {
                out.positions.push(p - 2.0 * d * n);
                out.positions.len() as u32 - 1
            }
        }).collect();
        for face in &self.faces {
            if face.verts.iter().all(|&v| twin[v as usize] == v) {
                continue;
            }
            // A reflection turns faces inside out, so the copy is wound the other way.
            out.faces.push(Face {
                verts: face.verts.iter().rev().map(|&v| twin[v as usize]).collect(),
                uvs: face.uvs.iter().rev().copied().collect(),
            });
        }
        out
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

/// Result of [`Mesh::edge_ring`]: what a loop cut would cross.
#[derive(Clone, Debug)]
pub struct EdgeRing {
    /// Ring edges, all oriented the same way round.
    pub edges: Vec<(u32, u32)>,
    pub closed: bool,
    /// (face, edge in, edge out) for every quad the ring passes through.
    quads: Vec<(usize, (u32, u32), (u32, u32))>,
    /// Non-quad faces touching the first / last edge of an open ring.
    ends: Vec<(usize, (u32, u32))>,
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
    fn extrude_region_leaves_no_loose_vertices() {
        let mut m = Mesh::cube();
        m.extrude_faces(&[0, 4].into()); // two faces sharing an edge
        let used: BTreeSet<u32> = m.faces.iter().flat_map(|f| f.verts.iter().copied()).collect();
        assert_eq!(used.len(), m.positions.len());
        assert!(is_closed(&m));
    }

    fn face_area(m: &Mesh, f: usize) -> f32 {
        let v = &m.faces[f].verts;
        let p0 = m.positions[v[0] as usize];
        (1..v.len() - 1).map(|i| (m.positions[v[i] as usize] - p0).cross(m.positions[v[i + 1] as usize] - p0).length() * 0.5).sum()
    }

    #[test]
    fn inset_keeps_mesh_closed_and_texture_in_place() {
        let mut m = Mesh::cube();
        m.inset_faces(&[4].into(), 0.25);
        assert_eq!((m.faces.len(), m.positions.len()), (10, 12));
        assert!(is_closed(&m) && normals_point_out(&m));
        assert!((face_area(&m, 4) - 0.25).abs() < 1e-5, "inner face should be 0.5 x 0.5");
        let (lo, hi) = m.uv_bounds(&[4].into()).unwrap();
        assert!(lo.abs_diff_eq(Vec2::splat(0.25), 1e-5) && hi.abs_diff_eq(Vec2::splat(0.75), 1e-5));

        // Region inset over an edge: two faces, six border edges.
        let mut m = Mesh::cube();
        m.inset_faces(&[0, 4].into(), 0.2);
        assert_eq!(m.faces.len(), 12);
        assert!(is_closed(&m) && normals_point_out(&m));
    }

    /// Signed volume; positive when faces are wound outwards.
    fn volume(m: &Mesh) -> f32 {
        m.faces.iter().map(|f| {
            let p0 = m.positions[f.verts[0] as usize];
            (1..f.verts.len() - 1).map(|i| p0.dot(m.positions[f.verts[i] as usize].cross(m.positions[f.verts[i + 1] as usize])) / 6.0).sum::<f32>()
        }).sum()
    }

    fn all_edges(m: &Mesh) -> BTreeSet<(u32, u32)> {
        m.edges().into_iter().collect()
    }

    #[test]
    fn bevel_all_edges_of_cube() {
        let mut m = Mesh::cube();
        let new = m.bevel_edges(&all_edges(&m), 0.1);
        assert_eq!(new.len(), 12 + 8);
        assert_eq!((m.faces.len(), m.positions.len()), (26, 24));
        assert!(is_closed(&m) && normals_point_out(&m));
        assert!((face_area(&m, 4) - 0.64).abs() < 1e-5, "each face shrinks by the bevel width on every side");
    }

    #[test]
    fn bevel_single_edge_and_face_loop() {
        // One edge: its ends slide out along the neighbouring edges; the end faces become pentagons.
        let mut m = Mesh::cube();
        let e = (m.faces[0].verts[0], m.faces[0].verts[1]);
        let new = m.bevel_edges(&[e].into(), 0.2);
        assert_eq!(new.len(), 1);
        assert_eq!((m.faces.len(), m.positions.len()), (7, 10));
        assert!(is_closed(&m) && normals_point_out(&m));
        assert_eq!(m.faces.iter().filter(|f| f.verts.len() == 5).count(), 2);
        assert!(m.faces.iter().all(|f| f.verts.len() == f.uvs.len()));

        // One spoke of a pyramid tip (four faces meet there): the far side gets a small triangle.
        let mut m = Mesh::cylinder(4, 0.5, 0.0, 1.0);
        let before = volume(&m);
        let new = m.bevel_edges(&[(0, 4)].into(), 0.1);
        assert_eq!(new.len(), 2);
        // Closed with consistent winding, and a sliver of volume shaved off.
        assert!(is_closed(&m));
        assert!(volume(&m) > before * 0.9 && volume(&m) < before);

        // The four edges around the top face.
        let mut m = Mesh::cube();
        let top = m.faces[4].verts.clone();
        let edges: BTreeSet<(u32, u32)> = (0..4).map(|i| (top[i], top[(i + 1) % 4])).collect();
        let new = m.bevel_edges(&edges, 0.2);
        assert_eq!(new.len(), 4);
        assert!(is_closed(&m) && normals_point_out(&m));
        assert!((face_area(&m, 4) - 0.36).abs() < 1e-5);
        // Side faces keep four corners and lose 0.2 of their height.
        assert!((face_area(&m, 0) - 0.8).abs() < 1e-5);
    }

    #[test]
    fn loop_cut_cube_and_cylinder() {
        let mut m = Mesh::cube();
        // A vertical edge: the ring runs around the four side faces.
        let ring = m.edge_ring(1, 2);
        assert!(ring.closed);
        assert_eq!(ring.edges.len(), 4);
        let new = m.loop_cut(&ring, &[0.25, 0.75]);
        assert_eq!(new.len(), 8);
        assert_eq!((m.faces.len(), m.positions.len()), (14, 16));
        assert!(is_closed(&m) && normals_point_out(&m));
        for i in 0..4 {
            // First cut sits a quarter of the way along every ring edge, and they all agree on which end is the start.
            let y = m.positions[new[2 * i] as usize].y;
            assert!((y - m.positions[new[0] as usize].y).abs() < 1e-6 && ((y - 0.25).abs() < 1e-6 || (y - 0.75).abs() < 1e-6));
        }
        // UVs of the split faces still cover the original square exactly once.
        let sides: f32 = (0..m.faces.len()).filter(|&f| m.face_normal(f).y.abs() < 0.5).map(|f| {
            let (lo, hi) = m.uv_bounds(&[f].into()).unwrap();
            (hi - lo).x * (hi - lo).y
        }).sum();
        assert!((sides - 4.0).abs() < 1e-4);

        // Cylinder: a rim edge leads across one side quad and ends in the two caps.
        let mut m = Mesh::cylinder(8, 0.5, 0.5, 1.0);
        let ring = m.edge_ring(0, 1);
        assert!(!ring.closed);
        assert_eq!(ring.edges.len(), 2);
        m.loop_cut(&ring, &[0.5]);
        assert_eq!((m.faces.len(), m.positions.len()), (11, 18));
        assert!(is_closed(&m) && normals_point_out(&m));
        assert!(m.faces.iter().all(|f| f.verts.len() == f.uvs.len()));
    }

    #[test]
    fn mirror_welds_the_seam() {
        // Half a box: a unit cube with its -X side removed, mirrored across the plane of that side.
        let mut half = Mesh::cube();
        let open_side: BTreeSet<usize> = (0..6).filter(|&f| half.face_normal(f).x < -0.9).collect();
        half.delete_faces(&open_side);
        assert!(!is_closed(&half));
        let whole = half.mirrored(Vec3::new(-0.5, 0.0, 0.0), Vec3::X, 1e-4);
        assert_eq!((whole.faces.len(), whole.positions.len()), (10, 12)); // 4 seam vertices shared
        assert!(is_closed(&whole));
        assert!((volume(&whole) - 2.0).abs() < 1e-5);
        assert!(whole.positions.iter().any(|p| (p.x + 1.5).abs() < 1e-6));
        // UVs come along unchanged.
        assert!(whole.faces.iter().all(|f| f.verts.len() == f.uvs.len()));
        // A face lying in the mirror plane is not duplicated.
        let plane = Mesh::plane().mirrored(Vec3::ZERO, Vec3::Y, 1e-4);
        assert_eq!((plane.faces.len(), plane.positions.len()), (1, 4));
        // Mirroring twice (two axes) gives four copies.
        let quarter = Mesh::plane().translated(Vec3::new(1.0, 0.0, 1.0));
        let four = quarter.mirrored(Vec3::ZERO, Vec3::X, 1e-4).mirrored(Vec3::ZERO, Vec3::Z, 1e-4);
        assert_eq!(four.faces.len(), 4);
        assert!((0..4).all(|f| four.face_normal(f).abs_diff_eq(Vec3::Y, 1e-6)));
    }

    #[test]
    fn edge_loops() {
        // Around the middle of a cube after a loop cut: four edges, closed.
        let mut m = Mesh::cube();
        let ring = m.edge_ring(1, 2);
        let new = m.loop_cut(&ring, &[0.5]);
        let l = m.edge_loop(new[0], new[1]);
        assert_eq!(l.len(), 4);
        assert!(l.iter().all(|(a, b)| new.contains(a) && new.contains(b)));
        // A plain cube edge ends at both corners (three edges meet there).
        assert_eq!(Mesh::cube().edge_loop(0, 1).len(), 1);
        // Rim of a cylinder: all the way round, not down the sides.
        let c = Mesh::cylinder(8, 0.5, 0.5, 1.0);
        let rim = c.edge_loop(0, 1);
        assert_eq!(rim.len(), 8);
        assert!(rim.iter().all(|&(a, b)| a < 8 && b < 8));
        // Face loop round the cylinder's side.
        assert_eq!(c.edge_ring(0, 8).faces().len(), 8);
    }

    #[test]
    fn merge_faces_into_ngons() {
        // Undo a loop cut on one side: two quads become one again, mesh stays closed.
        let mut m = Mesh::cube();
        let ring = m.edge_ring(1, 2);
        m.loop_cut(&ring, &[0.5]);
        assert_eq!(m.faces.len(), 10);
        let front: BTreeSet<usize> = (0..m.faces.len()).filter(|&f| m.face_normal(f).z > 0.9).collect();
        assert_eq!(front.len(), 2);
        let (new, skipped) = m.merge_faces(&front);
        assert_eq!((new.len(), skipped, m.faces.len()), (1, 0, 9));
        let f = *new.iter().next().unwrap();
        // The cut vertices are still used by the neighbouring sides, so they stay: a hexagon.
        assert_eq!(m.faces[f].verts.len(), 6);
        assert!((face_area(&m, f) - 1.0).abs() < 1e-5);
        assert!(is_closed(&m) && normals_point_out(&m));
        let (lo, hi) = m.uv_bounds(&new).unwrap();
        assert!(lo.abs_diff_eq(Vec2::ZERO, 1e-6) && hi.abs_diff_eq(Vec2::ONE, 1e-6));

        // A flat 2x1 strip with nothing around it: the middle vertices disappear.
        let mut strip = Mesh::plane();
        strip.append(&Mesh::plane().translated(Vec3::X));
        strip.merge_by_distance(1e-4);
        let (new, _) = strip.merge_faces(&[0, 1].into());
        assert_eq!((strip.faces.len(), strip.positions.len()), (1, 4));
        assert_eq!(strip.faces[*new.iter().next().unwrap()].verts.len(), 4);

        // Two separate pairs are merged separately; faces that only touch at a corner are not merged.
        let mut m = Mesh::cube();
        let (new, skipped) = m.merge_faces(&[0, 1].into()); // front and back: not connected
        assert_eq!((new.len(), skipped, m.faces.len()), (0, 0, 6));
        // Three faces round a corner form one simple outline.
        let (new, skipped) = m.merge_faces(&[0, 2, 4].into());
        assert_eq!((new.len(), skipped, m.faces.len()), (1, 0, 4));
        assert!(is_closed(&m));
    }

    #[test]
    fn triangulate_concave_polygon() {
        // An L shape: fan triangulation from corner 0 would spill outside it.
        let mut m = Mesh::default();
        let pts = [(0., 0.), (2., 0.), (2., 1.), (1., 1.), (1., 2.), (0., 2.)];
        m.positions = pts.iter().map(|&(x, z)| Vec3::new(x, 0.0, -z)).collect();
        m.faces.push(Face { verts: (0..6).collect(), uvs: vec![Vec2::ZERO; 6] });
        let tris = m.triangulate(0);
        assert_eq!(tris.len(), 4);
        let area: f32 = tris.iter().map(|t| {
            let [a, b, c] = t.map(|i| m.positions[i]);
            (b - a).cross(c - a).y * 0.5
        }).sum();
        assert!((area - 3.0).abs() < 1e-5);
        assert!(tris.iter().all(|t| { let [a, b, c] = t.map(|i| m.positions[i]); (b - a).cross(c - a).y > 0.0 }));
        // The notch is empty: a ray through it misses, one through the L hits.
        assert!(m.raycast(Vec3::new(1.5, 1.0, -1.5), Vec3::NEG_Y).is_none());
        assert!(m.raycast(Vec3::new(0.5, 1.0, -1.5), Vec3::NEG_Y).is_some());
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
