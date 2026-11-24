//! Further mesh operations: subdivide, fill, bridge, and look-ups used by
//! painting on the model and the texel density check.

use crate::mesh::{face_basis, normalize_points, quad_uvs, ray_triangle, Face, Mesh};
use glam::{Vec2, Vec3};
use std::collections::{BTreeMap, BTreeSet, HashMap};

type Edge = (u32, u32);

fn key(a: u32, b: u32) -> Edge {
    (a.min(b), a.max(b))
}

/// Sort a set of edges into chains of vertices. Each chain says whether it closes
/// on itself. None when some vertex has more than two of the edges (a branch).
pub fn edge_chains(edges: &BTreeSet<Edge>) -> Option<Vec<(Vec<u32>, bool)>> {
    let mut next: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &(a, b) in edges {
        if a == b {
            continue;
        }
        next.entry(a).or_default().push(b);
        next.entry(b).or_default().push(a);
    }
    if next.values().any(|n| n.len() > 2) {
        return None;
    }
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut chains = Vec::new();
    // Open chains first (they start at a vertex with one edge), then the closed loops.
    let starts: Vec<u32> = next.iter().filter(|(_, n)| n.len() == 1).map(|(&v, _)| v)
        .chain(next.keys().copied())
        .collect();
    for start in starts {
        if seen.contains(&start) {
            continue;
        }
        let mut chain = vec![start];
        seen.insert(start);
        let mut cur = start;
        while let Some(&n) = next[&cur].iter().find(|n| !seen.contains(n)) {
            chain.push(n);
            seen.insert(n);
            cur = n;
        }
        let closed = chain.len() > 2 && next[&cur].contains(&start) && next[&start].len() == 2;
        chains.push((chain, closed));
    }
    Some(chains)
}

impl Mesh {
    /// Ray cast that also returns the texture coordinate at the hit point.
    pub fn raycast_uv(&self, origin: Vec3, dir: Vec3) -> Option<(usize, f32, Vec2)> {
        let mut best: Option<(usize, f32, [usize; 3])> = None;
        for (fi, f) in self.faces.iter().enumerate() {
            for tri in self.triangulate(fi) {
                let [p0, p1, p2] = tri.map(|c| self.positions[f.verts[c] as usize]);
                if let Some(t) = ray_triangle(origin, dir, p0, p1, p2) {
                    if best.is_none_or(|(_, bt, _)| t < bt) {
                        best = Some((fi, t, tri));
                    }
                }
            }
        }
        let (fi, t, tri) = best?;
        let f = &self.faces[fi];
        let [a, b, c] = tri.map(|k| self.positions[f.verts[k] as usize]);
        let p = origin + dir * t;
        // Barycentric coordinates of the hit point in its triangle.
        let (v0, v1, v2) = (b - a, c - a, p - a);
        let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
        let denom = d00 * d11 - d01 * d01;
        if denom.abs() < 1e-12 {
            return Some((fi, t, f.uvs[tri[0]]));
        }
        let v = (d11 * d20 - d01 * d21) / denom;
        let w = (d00 * d21 - d01 * d20) / denom;
        let uv = f.uvs[tri[0]] * (1.0 - v - w) + f.uvs[tri[1]] * v + f.uvs[tri[2]] * w;
        Some((fi, t, uv))
    }

    /// Texture pixels per unit of length on face `f` (None for a face without area).
    pub fn face_density(&self, f: usize, tex: Vec2) -> Option<f32> {
        let face = &self.faces[f];
        let n = face.verts.len();
        let mut world = Vec3::ZERO;
        let mut uv = 0.0;
        for i in 0..n {
            let (a, b) = (self.positions[face.verts[i] as usize], self.positions[face.verts[(i + 1) % n] as usize]);
            world += a.cross(b);
            let (ua, ub) = (face.uvs[i] * tex, face.uvs[(i + 1) % n] * tex);
            uv += ua.x * ub.y - ub.x * ua.y;
        }
        let world = world.length() * 0.5;
        (world > 1e-9).then(|| (uv.abs() * 0.5 / world).sqrt())
    }

    /// Split the given faces evenly: a quad becomes four quads, a triangle four triangles,
    /// a larger polygon one quad per corner. Faces next to them receive the new edge
    /// midpoints so the mesh stays closed. Returns the faces that replace the selection.
    pub fn subdivide_faces(&mut self, faces: &BTreeSet<usize>) -> BTreeSet<usize> {
        let orig = self.clone();
        let mut mids: HashMap<Edge, u32> = HashMap::new();
        let mut out = BTreeSet::new();
        for &f in faces {
            let Some(face) = orig.faces.get(f) else { continue };
            let n = face.verts.len();
            let mid: Vec<(u32, Vec2)> = (0..n).map(|i| {
                let (a, b) = (face.verts[i], face.verts[(i + 1) % n]);
                let v = *mids.entry(key(a, b)).or_insert_with(|| {
                    self.positions.push((orig.positions[a as usize] + orig.positions[b as usize]) * 0.5);
                    (self.positions.len() - 1) as u32
                });
                (v, (face.uvs[i] + face.uvs[(i + 1) % n]) * 0.5)
            }).collect();
            let corner = |i: usize| (face.verts[i], face.uvs[i]);
            let mut parts: Vec<Vec<(u32, Vec2)>> = Vec::new();
            if n == 3 {
                parts.push(vec![corner(0), mid[0], mid[2]]);
                parts.push(vec![mid[0], corner(1), mid[1]]);
                parts.push(vec![mid[2], mid[1], corner(2)]);
                parts.push(vec![mid[0], mid[1], mid[2]]);
            } else {
                let centre = face.verts.iter().map(|&v| orig.positions[v as usize]).sum::<Vec3>() / n as f32;
                let centre_uv = face.uvs.iter().copied().sum::<Vec2>() / n as f32;
                self.positions.push(centre);
                let c = ((self.positions.len() - 1) as u32, centre_uv);
                for i in 0..n {
                    parts.push(vec![corner(i), mid[i], c, mid[(i + n - 1) % n]]);
                }
            }
            for (i, part) in parts.into_iter().enumerate() {
                let new = Face { verts: part.iter().map(|p| p.0).collect(), uvs: part.iter().map(|p| p.1).collect() };
                if i == 0 {
                    self.faces[f] = new;
                    out.insert(f);
                } else {
                    out.insert(self.faces.len());
                    self.faces.push(new);
                }
            }
        }
        // Neighbours of the split faces get the midpoints as extra corners.
        for (fi, face) in orig.faces.iter().enumerate() {
            if faces.contains(&fi) {
                continue;
            }
            let n = face.verts.len();
            if !(0..n).any(|i| mids.contains_key(&key(face.verts[i], face.verts[(i + 1) % n]))) {
                continue;
            }
            let mut new = Face { verts: Vec::new(), uvs: Vec::new() };
            for i in 0..n {
                new.verts.push(face.verts[i]);
                new.uvs.push(face.uvs[i]);
                if let Some(&m) = mids.get(&key(face.verts[i], face.verts[(i + 1) % n])) {
                    new.verts.push(m);
                    new.uvs.push((face.uvs[i] + face.uvs[(i + 1) % n]) * 0.5);
                }
            }
            self.faces[fi] = new;
        }
        out
    }

    /// Close holes: every loop (or open run of three or more vertices) among `edges`
    /// becomes a new face. Returns the new faces.
    pub fn fill_edges(&mut self, edges: &BTreeSet<Edge>) -> Result<BTreeSet<usize>, &'static str> {
        let chains = edge_chains(edges).ok_or("the selected edges branch; select one loop per hole")?;
        let dir = self.directed_edge_faces();
        let first = self.faces.len();
        for (mut chain, _) in chains {
            if chain.len() < 3 {
                continue;
            }
            // Wind the new face against its neighbour across the first edge that has one.
            let n = chain.len();
            let has = |a: u32, b: u32| dir.contains_key(&(a, b));
            let same = (0..n).filter(|&i| has(chain[i], chain[(i + 1) % n])).count();
            let opposite = (0..n).filter(|&i| has(chain[(i + 1) % n], chain[i])).count();
            if same > opposite {
                chain.reverse();
            }
            let pts: Vec<Vec3> = chain.iter().map(|&v| self.positions[v as usize]).collect();
            let normal = (0..n).fold(Vec3::ZERO, |acc, i| acc + pts[i].cross(pts[(i + 1) % n])).normalize_or(Vec3::Y);
            let (t, b) = face_basis(normal);
            let flat: Vec<Vec2> = pts.iter().map(|p| Vec2::new(p.dot(t), p.dot(b))).collect();
            self.faces.push(Face { verts: chain, uvs: normalize_points(&flat, true) });
        }
        if self.faces.len() == first {
            return Err("nothing to fill: select the edges around a hole (three or more vertices)");
        }
        Ok((first..self.faces.len()).collect())
    }

    /// Join two runs of vertices with a band of quads. `ignore` lists faces that are about
    /// to be deleted (bridging two faces), so they do not count when choosing the winding.
    pub fn bridge(&mut self, a: &[u32], b: &[u32], closed: bool, ignore: &BTreeSet<usize>) -> Result<BTreeSet<usize>, &'static str> {
        let n = a.len();
        if n < 2 || b.len() != n {
            return Err("bridge needs two edge loops with the same number of vertices");
        }
        let pos = |v: u32| self.positions[v as usize];
        // Line the second run up with the first: the pairing with the shortest connections.
        let mut best: Option<(f32, Vec<u32>)> = None;
        for reversed in [false, true] {
            for shift in 0..if closed { n } else { 1 } {
                let cand: Vec<u32> = (0..n).map(|i| {
                    let j = if reversed { (n - 1 - i + shift) % n } else { (i + shift) % n };
                    b[j]
                }).collect();
                let cost: f32 = (0..n).map(|i| pos(a[i]).distance_squared(pos(cand[i]))).sum();
                if best.as_ref().is_none_or(|(c, _)| cost < *c) {
                    best = Some((cost, cand));
                }
            }
        }
        let b = best.unwrap().1;
        let mut dir: HashMap<Edge, usize> = self.directed_edge_faces();
        dir.retain(|_, f| !ignore.contains(f));
        let count = if closed { n } else { n - 1 };
        // Quads as (a_i, a_i+1, b_i+1, b_i); flipped if that would run the same way as a neighbour.
        let same: usize = (0..count).filter(|&i| dir.contains_key(&(a[i], a[(i + 1) % n])) || dir.contains_key(&(b[(i + 1) % n], b[i]))).count();
        let opposite: usize = (0..count).filter(|&i| dir.contains_key(&(a[(i + 1) % n], a[i])) || dir.contains_key(&(b[i], b[(i + 1) % n]))).count();
        let flip = same > opposite;
        let first = self.faces.len();
        for i in 0..count {
            let j = (i + 1) % n;
            let mut verts = vec![a[i], a[j], b[j], b[i]];
            if flip {
                verts.reverse();
            }
            self.faces.push(Face { verts, uvs: quad_uvs() });
        }
        Ok((first..self.faces.len()).collect())
    }

    /// Bridge the two loops (or two open runs) found among `edges`.
    pub fn bridge_edges(&mut self, edges: &BTreeSet<Edge>) -> Result<BTreeSet<usize>, &'static str> {
        let chains = edge_chains(edges).ok_or("the selected edges branch; select two separate loops")?;
        let [(a, closed_a), (b, closed_b)] = &chains[..] else {
            return Err("bridge needs exactly two separate edge loops selected");
        };
        if closed_a != closed_b {
            return Err("bridge needs two loops of the same kind (both closed or both open)");
        }
        self.bridge(&a.clone(), &b.clone(), *closed_a, &BTreeSet::new())
    }

    /// Bridge two faces: both are removed and their outlines joined.
    pub fn bridge_faces(&mut self, fa: usize, fb: usize) -> Result<BTreeSet<usize>, &'static str> {
        let (a, b) = (self.faces[fa].verts.clone(), self.faces[fb].verts.clone());
        if a.iter().any(|v| b.contains(v)) {
            return Err("the two faces touch; bridge needs two separate faces");
        }
        let ignore: BTreeSet<usize> = [fa, fb].into();
        let count = self.bridge(&a, &b, true, &ignore)?.len();
        self.delete_faces(&ignore);
        Ok((self.faces.len() - count..self.faces.len()).collect())
    }
    /// Put a new vertex on the edge between `a` and `b`, `t` of the way from `a` (0..1).
    /// Every face that uses the edge receives it, with its UV interpolated. Returns the vertex.
    pub fn split_edge(&mut self, a: u32, b: u32, t: f32) -> u32 {
        let v = self.positions.len() as u32;
        self.positions.push(self.positions[a as usize].lerp(self.positions[b as usize], t));
        for face in &mut self.faces {
            let n = face.verts.len();
            for i in 0..n {
                let (x, y) = (face.verts[i], face.verts[(i + 1) % n]);
                if (x, y) != (a, b) && (x, y) != (b, a) {
                    continue;
                }
                let from_x = if x == a { t } else { 1.0 - t };
                let uv = face.uvs[i].lerp(face.uvs[(i + 1) % n], from_x);
                face.verts.insert(i + 1, v);
                face.uvs.insert(i + 1, uv);
                break;
            }
        }
        v
    }

    /// Cut a face in two with a new edge between two of its vertices. The face is the one
    /// that has both (and does not already have an edge between them). Returns the new face;
    /// the other half keeps the old face's index.
    pub fn connect_verts(&mut self, a: u32, b: u32) -> Result<usize, &'static str> {
        let mut joined = false;
        for fi in 0..self.faces.len() {
            let verts = &self.faces[fi].verts;
            let n = verts.len();
            let (Some(i), Some(j)) = (verts.iter().position(|&v| v == a), verts.iter().position(|&v| v == b)) else { continue };
            let (i, j) = (i.min(j), i.max(j));
            if j - i == 1 || j - i == n - 1 {
                joined = true;
                continue;
            }
            let face = self.faces[fi].clone();
            let part = |range: Vec<usize>| Face { verts: range.iter().map(|&k| face.verts[k]).collect(), uvs: range.iter().map(|&k| face.uvs[k]).collect() };
            self.faces[fi] = part((i..=j).collect());
            self.faces.push(part((j..n).chain(0..=i).collect()));
            return Ok(self.faces.len() - 1);
        }
        Err(if joined { "those two points already have an edge between them" } else { "those two points are not on the same face" })
    }

    /// Make a face from loose vertices. They are taken in the order of the edges that
    /// already join them when those form one run; otherwise in order round their middle.
    /// The face winds against its neighbours, or faces `toward` (the viewer) when it has none.
    pub fn face_from_verts(&mut self, verts: &BTreeSet<u32>, toward: Vec3) -> Result<usize, &'static str> {
        let n = verts.len();
        if n < 3 {
            return Err("select at least three vertices");
        }
        if self.faces.iter().any(|f| f.verts.len() == n && f.verts.iter().all(|v| verts.contains(v))) {
            return Err("those vertices already make a face");
        }
        let between: BTreeSet<Edge> = self.edges().into_iter().filter(|(a, b)| verts.contains(a) && verts.contains(b)).map(|(a, b)| key(a, b)).collect();
        let chain = edge_chains(&between).filter(|c| c.len() == 1 && c[0].0.len() == n).map(|mut c| c.remove(0).0);
        let mut order: Vec<u32> = match chain {
            Some(chain) => chain,
            None => {
                let pts: Vec<(u32, Vec3)> = verts.iter().map(|&v| (v, self.positions[v as usize])).collect();
                let centre = pts.iter().map(|p| p.1).sum::<Vec3>() / n as f32;
                // The plane the points lie in, from the widest pair of spokes.
                let mut normal = Vec3::ZERO;
                for (i, a) in pts.iter().enumerate() {
                    for b in &pts[i + 1..] {
                        let c = (a.1 - centre).cross(b.1 - centre);
                        if c.length_squared() > normal.length_squared() {
                            normal = c;
                        }
                    }
                }
                if normal.length_squared() < 1e-12 {
                    return Err("those vertices lie on one line");
                }
                let (t, b) = face_basis(normal.normalize());
                let angle = |p: Vec3| (p - centre).dot(b).atan2((p - centre).dot(t));
                let mut sorted = pts.clone();
                sorted.sort_by(|x, y| angle(x.1).total_cmp(&angle(y.1)));
                sorted.into_iter().map(|p| p.0).collect()
            }
        };
        let dir = self.directed_edge_faces();
        let same = (0..n).filter(|&i| dir.contains_key(&(order[i], order[(i + 1) % n]))).count();
        let opposite = (0..n).filter(|&i| dir.contains_key(&(order[(i + 1) % n], order[i]))).count();
        let pts = |order: &[u32]| -> Vec<Vec3> { order.iter().map(|&v| self.positions[v as usize]).collect() };
        let newell = |p: &[Vec3]| (0..n).fold(Vec3::ZERO, |acc, i| acc + p[i].cross(p[(i + 1) % n]));
        let facing_away = newell(&pts(&order)).dot(toward) < 0.0;
        if same > opposite || (same == opposite && facing_away) {
            order.reverse();
        }
        let p = pts(&order);
        let normal = newell(&p).normalize_or(Vec3::Y);
        let (t, b) = face_basis(normal);
        let flat: Vec<Vec2> = p.iter().map(|p| Vec2::new(p.dot(t), p.dot(b))).collect();
        self.faces.push(Face { verts: order, uvs: normalize_points(&flat, true) });
        Ok(self.faces.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_closed(m: &Mesh) -> bool {
        let dir = m.directed_edge_faces();
        let count: usize = m.faces.iter().map(|f| f.verts.len()).sum();
        dir.len() == count && dir.keys().all(|&(a, b)| dir.contains_key(&(b, a)))
    }

    fn volume(m: &Mesh) -> f32 {
        (0..m.faces.len()).flat_map(|f| m.triangulate(f).into_iter().map(move |t| (f, t))).map(|(f, t)| {
            let [a, b, c] = t.map(|k| m.positions[m.faces[f].verts[k] as usize]);
            a.dot(b.cross(c)) / 6.0
        }).sum()
    }

    #[test]
    fn subdivide_keeps_the_mesh_closed() {
        let mut m = Mesh::cube();
        let new = m.subdivide_faces(&[0].into());
        assert_eq!(new.len(), 4);
        assert_eq!(m.faces.len(), 9);
        assert!(is_closed(&m));
        assert!((volume(&m) - 1.0).abs() < 1e-4);
        // All faces at once: 24 quads.
        let mut m = Mesh::cube();
        let all: BTreeSet<usize> = (0..6).collect();
        assert_eq!(m.subdivide_faces(&all).len(), 24);
        assert!(is_closed(&m));
        assert_eq!(m.positions.len(), 8 + 12 + 6);
        // Triangles split into four.
        let mut cone = Mesh::cylinder(6, 0.5, 0.0, 1.0);
        let tri = (0..cone.faces.len()).find(|&f| cone.faces[f].verts.len() == 3).unwrap();
        assert_eq!(cone.subdivide_faces(&[tri].into()).len(), 4);
        assert!(is_closed(&cone));
    }

    #[test]
    fn fill_closes_a_hole_facing_outwards() {
        let mut m = Mesh::cube();
        let before = volume(&m);
        let hole: Vec<u32> = m.faces[2].verts.clone();
        m.delete_faces(&[2].into());
        assert!(!is_closed(&m));
        // delete_faces keeps the vertices: they are still used by the neighbours.
        let edges: BTreeSet<Edge> = (0..4).map(|i| key(hole[i], hole[(i + 1) % 4])).collect();
        let new = m.fill_edges(&edges).unwrap();
        assert_eq!(new.len(), 1);
        assert!(is_closed(&m));
        assert!((volume(&m) - before).abs() < 1e-4);
        assert!(m.fill_edges(&[(0, 1)].into()).is_err());
    }

    #[test]
    fn bridge_joins_two_faces_into_a_tube() {
        // Two cubes side by side; bridging the facing sides makes one closed solid.
        let mut m = Mesh::cube();
        m.append(&Mesh::cube().translated(Vec3::new(3.0, 0.0, 0.0)));
        let facing = |m: &Mesh, n: Vec3, range: std::ops::Range<usize>| range.into_iter().find(|&f| m.face_normal(f).dot(n) > 0.9).unwrap();
        let (fa, fb) = (facing(&m, Vec3::X, 0..6), facing(&m, -Vec3::X, 6..12));
        let new = m.bridge_faces(fa, fb).unwrap();
        assert_eq!(new.len(), 4);
        assert_eq!(m.faces.len(), 14);
        assert!(is_closed(&m));
        // 1 + 1 + the 2 x 1 x 1 gap between them.
        assert!((volume(&m) - 4.0).abs() < 1e-4, "{}", volume(&m));

        // The same from edge loops, after deleting the two faces by hand.
        let mut m = Mesh::cube();
        m.append(&Mesh::cube().translated(Vec3::new(3.0, 0.0, 0.0)));
        let (la, lb) = (m.faces[fa].verts.clone(), m.faces[fb].verts.clone());
        m.delete_faces(&[fa, fb].into());
        let edges: BTreeSet<Edge> = [la, lb].iter().flat_map(|l| (0..4).map(|i| key(l[i], l[(i + 1) % 4])).collect::<Vec<_>>()).collect();
        assert_eq!(m.bridge_edges(&edges).unwrap().len(), 4);
        assert!(is_closed(&m));
        assert!((volume(&m) - 4.0).abs() < 1e-4);
    }

    #[test]
    fn rounded_bevel_stays_closed_and_removes_less() {
        let all = |m: &Mesh| -> BTreeSet<Edge> { m.edges().into_iter().collect() };
        let mut flat = Mesh::cube();
        flat.bevel_edges(&all(&Mesh::cube()), 0.2);
        for segments in [2, 3, 4] {
            let mut m = Mesh::cube();
            m.bevel_edges_rounded(&all(&Mesh::cube()), 0.2, segments);
            assert!(is_closed(&m), "{segments} segments");
            // A rounded corner keeps more material than a flat chamfer, and less than the sharp cube.
            assert!(volume(&m) > volume(&flat) && volume(&m) < 1.0, "{segments}: {} vs {}", volume(&m), volume(&flat));
        }
        // One edge, and a loop of four edges.
        let mut m = Mesh::cube();
        let e = m.edges()[0];
        m.bevel_edges_rounded(&[e].into(), 0.2, 3);
        assert!(is_closed(&m));
        let mut m = Mesh::cube();
        let top = (0..6).find(|&f| m.face_normal(f).y > 0.9).unwrap();
        let v = m.faces[top].verts.clone();
        let ring: BTreeSet<Edge> = (0..4).map(|i| key(v[i], v[(i + 1) % 4])).collect();
        m.bevel_edges_rounded(&ring, 0.2, 3);
        assert!(is_closed(&m));
        assert!(volume(&m) < 1.0);
    }

    #[test]
    fn raycast_uv_and_density() {
        let m = Mesh::plane();
        let (lo, hi) = m.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let centre = (lo + hi) * 0.5;
        let (_, _, uv) = m.raycast_uv(centre + Vec3::Y * 5.0, -Vec3::Y).unwrap();
        assert!((uv - Vec2::splat(0.5)).length() < 1e-4, "{uv}");
        // Towards one corner the UV follows that corner's.
        let corner = m.positions[m.faces[0].verts[0] as usize];
        let p = centre.lerp(corner, 0.9);
        let (_, _, uv) = m.raycast_uv(p + Vec3::Y * 5.0, -Vec3::Y).unwrap();
        assert!((uv - Vec2::splat(0.5).lerp(m.faces[0].uvs[0], 0.9)).length() < 1e-4);
        // The whole 32 px texture over a face of side s: 32 / s pixels per unit.
        let side = hi.x - lo.x;
        let d = m.face_density(0, Vec2::splat(32.0)).unwrap();
        assert!((d - 32.0 / side).abs() < 1e-3, "{d}");
    }

    #[test]
    fn chains() {
        let square: BTreeSet<Edge> = [(0, 1), (1, 2), (2, 3), (0, 3)].into();
        let c = edge_chains(&square).unwrap();
        assert_eq!(c.len(), 1);
        assert!(c[0].1 && c[0].0.len() == 4);
        let open: BTreeSet<Edge> = [(0, 1), (1, 2), (5, 6)].into();
        let c = edge_chains(&open).unwrap();
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|c| !c.1));
        assert!(edge_chains(&[(0, 1), (0, 2), (0, 3)].into()).is_none());
    }

    #[test]
    fn knife_cut_splits_a_face_and_keeps_the_mesh_closed() {
        let mut m = Mesh::cube();
        let before = volume(&m);
        let v = m.faces[0].verts.clone();
        // Midpoints of two opposite edges of face 0, then join them.
        let a = m.split_edge(v[0], v[1], 0.5);
        let b = m.split_edge(v[2], v[3], 0.25);
        assert!(is_closed(&m));
        assert_eq!(m.faces[0].verts.len(), 6);
        let new = m.connect_verts(a, b).unwrap();
        assert_eq!(m.faces.len(), 7);
        assert_eq!(m.faces[0].verts.len() + m.faces[new].verts.len(), 8);
        assert!(is_closed(&m));
        assert!((volume(&m) - before).abs() < 1e-4);
        // UVs of the new vertex are interpolated in every face.
        let k = m.faces[0].verts.iter().position(|&x| x == a).unwrap();
        assert!(m.faces[0].uvs[k].x >= 0.0 && m.faces[0].uvs[k].x <= 1.0);
        assert!(m.connect_verts(a, b).is_err());
        assert!(m.connect_verts(a, v[0]).is_err());
        // Corner to corner works too.
        let mut m = Mesh::cube();
        let v = m.faces[1].verts.clone();
        m.connect_verts(v[0], v[2]).unwrap();
        assert_eq!(m.faces.len(), 7);
        assert!(is_closed(&m));
    }

    #[test]
    fn face_from_vertices() {
        // Refill a hole from its four corner vertices: faces outwards again.
        let mut m = Mesh::cube();
        let before = volume(&m);
        let hole: BTreeSet<u32> = m.faces[3].verts.iter().copied().collect();
        m.delete_faces(&[3].into());
        let f = m.face_from_verts(&hole, Vec3::ZERO).unwrap();
        assert_eq!(m.faces[f].verts.len(), 4);
        assert!(is_closed(&m));
        assert!((volume(&m) - before).abs() < 1e-4);
        assert!(m.face_from_verts(&hole, Vec3::ZERO).is_err());
        // Loose points with no edges: ordered round their middle, facing the viewer.
        let mut m = Mesh::default();
        m.positions = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
        let f = m.face_from_verts(&(0..4).collect(), Vec3::Z).unwrap();
        assert!(m.face_normal(f).z > 0.9);
        // A proper square, not a bow tie: its area is 1.
        let area: f32 = m.triangulate(f).iter().map(|t| {
            let [a, b, c] = t.map(|k| m.positions[m.faces[f].verts[k] as usize]);
            (b - a).cross(c - a).length() * 0.5
        }).sum();
        assert!((area - 1.0).abs() < 1e-4, "{area}");
        let mut m2 = Mesh::default();
        m2.positions = m.positions.clone();
        let f = m2.face_from_verts(&(0..4).collect(), -Vec3::Z).unwrap();
        assert!(m2.face_normal(f).z < -0.9);
        assert!(m2.face_from_verts(&[0, 1].into(), Vec3::Z).is_err());
    }
}
