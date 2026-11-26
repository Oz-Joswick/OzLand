//! Heightfield terrain: a regular grid of heights, the triangle mesh built
//! from it, and exact height queries for walking.
//!
//! Each grid cell is split into two triangles along the (x0,z0)-(x1,z1)
//! diagonal. `height_at`, the mesh and the GPU (`terrain_height` in the
//! shader) all use that same split, so what you see is exactly what you
//! walk on.

use glam::{Vec2, Vec3};

use crate::mesh::{MeshData, Vertex};

pub struct Terrain {
    /// The grid covers -half_size..half_size on X and Z.
    pub half_size: f32,
    /// Distance between grid points, meters.
    pub cell: f32,
    /// Grid points per side.
    pub res: usize,
    /// Row-major (z, then x) heights.
    pub heights: Vec<f32>,
    pub water_level: f32,
    /// The heights as the map was made (craters can't dig deeper than
    /// [`MAX_DENT`] below them), and how scorched each point is (0..1).
    original: Vec<f32>,
    pub scorch: Vec<f32>,
}

/// Craters dig at most this deep below the ground as built.
pub const MAX_DENT: f32 = 3.5;

impl Terrain {
    /// Builds the grid by evaluating `height(x, z)` at every point (in
    /// parallel, row by row).
    pub fn from_fn(half_size: f32, cell: f32, water_level: f32, height: impl Fn(f32, f32) -> f32 + Sync) -> Self {
        let res = (2.0 * half_size / cell).round() as usize + 1;
        let mut heights = vec![0.0; res * res];
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let rows_per = res.div_ceil(threads);
        std::thread::scope(|s| {
            for (chunk_index, chunk) in heights.chunks_mut(rows_per * res).enumerate() {
                let height = &height;
                s.spawn(move || {
                    for (i, h) in chunk.iter_mut().enumerate() {
                        let index = chunk_index * rows_per * res + i;
                        let (ix, iz) = (index % res, index / res);
                        *h = height(-half_size + ix as f32 * cell, -half_size + iz as f32 * cell);
                    }
                });
            }
        });
        Self {
            half_size,
            cell,
            res,
            original: heights.clone(),
            scorch: vec![0.0; heights.len()],
            heights,
            water_level,
        }
    }

    /// Blasts a crater: a bowl `radius` across and up to `depth` deep with a
    /// low rim, the ground round it scorched. Points under something built
    /// on the ground (`blocked(x, z, height)`) are left alone, so houses
    /// don't end up floating, as is anything already at the deepest a
    /// crater may go or that would sink below the water. Returns the grid
    /// rows changed (first, last), if any.
    pub fn dent(&mut self, center: Vec3, radius: f32, depth: f32, blocked: impl Fn(f32, f32, f32) -> bool) -> Option<(usize, usize)> {
        let reach = radius * 1.35;
        let to_index = |v: f32| ((v + self.half_size) / self.cell).round() as i64;
        let max = self.res as i64 - 1;
        let (x0, x1) = (to_index(center.x - reach).clamp(0, max), to_index(center.x + reach).clamp(0, max));
        let (z0, z1) = (to_index(center.z - reach).clamp(0, max), to_index(center.z + reach).clamp(0, max));
        let mut rows: Option<(usize, usize)> = None;
        for iz in z0..=z1 {
            for ix in x0..=x1 {
                let (x, z) = (-self.half_size + ix as f32 * self.cell, -self.half_size + iz as f32 * self.cell);
                let d = Vec2::new(x - center.x, z - center.z).length() / radius;
                if d >= 1.35 {
                    continue;
                }
                let i = iz as usize * self.res + ix as usize;
                let h = self.heights[i];
                if blocked(x, z, h) {
                    continue;
                }
                // A smooth bowl, then a lip of thrown-up earth round it.
                let change = if d < 1.0 { -depth * (1.0 - d * d).powi(2) } else { depth * 0.18 * (1.0 - (d - 1.0) / 0.35) };
                let floor = (self.original[i] - MAX_DENT).max(self.water_level + 0.15);
                let new = if change < 0.0 { (h + change).max(floor.min(h)) } else { h + change };
                self.scorch[i] = self.scorch[i].max((1.15 - d).clamp(0.0, 1.0));
                if new != h {
                    self.heights[i] = new;
                }
                let r = iz as usize;
                rows = Some(rows.map_or((r, r), |(a, b)| (a.min(r), b.max(r))));
            }
        }
        rows
    }

    /// The mesh vertex for grid point (ix, iz); its alpha is how unscorched
    /// it is (the terrain shader darkens scorched ground).
    pub fn vertex(&self, ix: usize, iz: usize) -> Vertex {
        let (gx, gz) = (ix as i64, iz as i64);
        let x = -self.half_size + ix as f32 * self.cell;
        let z = -self.half_size + iz as f32 * self.cell;
        let h = self.grid(gx, gz);
        let dx = self.grid(gx + 1, gz) - self.grid(gx - 1, gz);
        let dz = self.grid(gx, gz + 1) - self.grid(gx, gz - 1);
        let n = Vec3::new(-dx, 2.0 * self.cell, -dz).normalize();
        // Tangent along +X (texture U), made perpendicular to the normal.
        let t = (Vec3::X - n * n.x).normalize();
        Vertex {
            position: [x, h, z],
            normal: n.to_array(),
            uv: (Vec2::new(x, z) / 4.0).to_array(),
            color: [1.0, 1.0, 1.0, 1.0 - self.scorch[iz * self.res + ix]],
            tangent: [t.x, t.y, t.z, 1.0],
        }
    }

    /// The vertices of grid rows `first..=last` (as laid out in the mesh).
    pub fn row_vertices(&self, first: usize, last: usize) -> Vec<Vertex> {
        (first..=last.min(self.res - 1)).flat_map(|iz| (0..self.res).map(move |ix| self.vertex(ix, iz))).collect()
    }

    /// Flat ground at height 0 (for tests).
    #[cfg(test)]
    pub fn flat(half_size: f32) -> Self {
        Self::from_fn(half_size, half_size, -100.0, |_, _| 0.0)
    }

    fn grid(&self, ix: i64, iz: i64) -> f32 {
        let max = self.res as i64 - 1;
        self.heights[(iz.clamp(0, max) * self.res as i64 + ix.clamp(0, max)) as usize]
    }

    /// Exact height of the rendered surface at (x, z).
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let gx = ((x + self.half_size) / self.cell).clamp(0.0, (self.res - 1) as f32 - 1e-4);
        let gz = ((z + self.half_size) / self.cell).clamp(0.0, (self.res - 1) as f32 - 1e-4);
        let (ix, iz) = (gx.floor() as i64, gz.floor() as i64);
        let (fx, fz) = (gx - ix as f32, gz - iz as f32);
        let h00 = self.grid(ix, iz);
        let h10 = self.grid(ix + 1, iz);
        let h01 = self.grid(ix, iz + 1);
        let h11 = self.grid(ix + 1, iz + 1);
        if fx >= fz {
            h00 + (h10 - h00) * fx + (h11 - h10) * fz
        } else {
            h00 + (h11 - h01) * fx + (h01 - h00) * fz
        }
    }

    /// Smoothed surface normal at (x, z).
    pub fn normal_at(&self, x: f32, z: f32) -> Vec3 {
        let e = self.cell;
        let dx = self.height_at(x + e, z) - self.height_at(x - e, z);
        let dz = self.height_at(x, z + e) - self.height_at(x, z - e);
        Vec3::new(-dx, 2.0 * e, -dz).normalize()
    }

    pub fn min_max(&self) -> (f32, f32) {
        self.heights
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)))
    }

    /// The renderable mesh: one vertex per grid point with smooth normals.
    pub fn mesh(&self) -> MeshData {
        let res = self.res;
        let vertices = self.row_vertices(0, res - 1);
        let mut indices = Vec::with_capacity((res - 1) * (res - 1) * 6);
        for iz in 0..res - 1 {
            for ix in 0..res - 1 {
                let i00 = (iz * res + ix) as u32;
                let i10 = i00 + 1;
                let i01 = i00 + res as u32;
                let i11 = i01 + 1;
                // Counter-clockwise seen from above, split along 00-11.
                indices.extend_from_slice(&[i00, i11, i10, i00, i01, i11]);
            }
        }
        MeshData { vertices, indices }
    }
}

/// Distances to a set of shapes (polylines with a radius), rasterized onto
/// a grid so lookups are cheap. Stores `distance - radius`, so values below
/// zero are inside a shape.
pub struct DistanceField {
    origin: Vec2,
    cell: f32,
    size: usize,
    data: Vec<f32>,
}

impl DistanceField {
    pub fn new(half_size: f32, cell: f32) -> Self {
        let size = (2.0 * half_size / cell).ceil() as usize + 1;
        Self {
            origin: Vec2::splat(-half_size),
            cell,
            size,
            data: vec![f32::MAX; size * size],
        }
    }

    /// Adds a polyline (or a single point) of the given radius. Only cells
    /// within `reach` of the shape's edge get a value.
    pub fn add(&mut self, points: &[Vec2], radius: f32, reach: f32) {
        let segments: Vec<(Vec2, Vec2)> = if points.len() == 1 {
            vec![(points[0], points[0])]
        } else {
            points.windows(2).map(|w| (w[0], w[1])).collect()
        };
        let extent = radius + reach;
        for (a, b) in segments {
            let lo = ((a.min(b) - Vec2::splat(extent) - self.origin) / self.cell).floor();
            let hi = ((a.max(b) + Vec2::splat(extent) - self.origin) / self.cell).ceil();
            let max = self.size as i64 - 1;
            for gz in (lo.y as i64).max(0)..=(hi.y as i64).min(max) {
                for gx in (lo.x as i64).max(0)..=(hi.x as i64).min(max) {
                    let p = self.origin + Vec2::new(gx as f32, gz as f32) * self.cell;
                    let d = distance_to_segment(p, a, b) - radius;
                    let cell = &mut self.data[gz as usize * self.size + gx as usize];
                    *cell = cell.min(d);
                }
            }
        }
    }

    /// Bilinearly interpolated `distance - radius` (f32::MAX far from shapes).
    pub fn at(&self, p: Vec2) -> f32 {
        let g = ((p - self.origin) / self.cell).clamp(Vec2::ZERO, Vec2::splat((self.size - 1) as f32 - 1e-4));
        let (ix, iz) = (g.x as usize, g.y as usize);
        let f = g - Vec2::new(ix as f32, iz as f32);
        let v = |x: usize, z: usize| self.data[z * self.size + x];
        let (a, b, c, d) = (v(ix, iz), v(ix + 1, iz), v(ix, iz + 1), v(ix + 1, iz + 1));
        if a.max(b).max(c).max(d) == f32::MAX {
            return a.min(b).min(c).min(d);
        }
        let top = a + (b - a) * f.x;
        let bottom = c + (d - c) * f.x;
        top + (bottom - top) * f.y
    }
}

pub fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
}

/// Samples a smooth Catmull-Rom curve through `points`, roughly every `step` meters.
pub fn spline(points: &[Vec2], step: f32) -> Vec<Vec2> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let mut out = Vec::new();
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = points[(i + 2).min(points.len() - 1)];
        let n = ((p2 - p1).length() / step).ceil().max(1.0) as usize;
        for k in 0..n {
            let t = k as f32 / n as f32;
            let (t2, t3) = (t * t, t * t * t);
            out.push(
                0.5 * ((2.0 * p1)
                    + (p2 - p0) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3),
            );
        }
    }
    out.push(*points.last().unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bumpy() -> Terrain {
        Terrain::from_fn(10.0, 1.0, 0.0, |x, z| (x * 0.7).sin() * 2.0 + (z * 0.4).cos() * 3.0)
    }

    #[test]
    fn craters_dig_a_scorched_bowl_but_spare_buildings_and_stop_at_a_limit() {
        let mut t = Terrain::from_fn(20.0, 1.0, -50.0, |_, _| 5.0);
        let rows = t.dent(Vec3::new(0.0, 5.0, 0.0), 4.0, 2.0, |x, _, _| x > 2.5);
        assert!(rows.is_some());
        assert!((t.height_at(0.0, 0.0) - 3.0).abs() < 1e-3, "2 m deep in the middle: {}", t.height_at(0.0, 0.0));
        assert!(t.height_at(-4.6, 0.0) > 5.0, "a raised lip round it");
        assert_eq!(t.height_at(3.0, 0.0), 5.0, "a house standing there keeps its ground");
        assert!(t.vertex(20, 20).color[3] < 0.5, "scorched in the middle");
        for _ in 0..10 {
            t.dent(Vec3::new(0.0, 5.0, 0.0), 4.0, 2.0, |_, _, _| false);
        }
        assert!((t.height_at(0.0, 0.0) - (5.0 - MAX_DENT)).abs() < 1e-3, "no deeper than the limit");
        assert!(t.dent(Vec3::new(100.0, 0.0, 100.0), 4.0, 2.0, |_, _, _| false).is_none(), "off the map: nothing");
        // Near the lake, craters stop just above the water.
        let mut shore = Terrain::from_fn(20.0, 1.0, 4.5, |_, _| 5.0);
        shore.dent(Vec3::new(0.0, 5.0, 0.0), 4.0, 2.0, |_, _, _| false);
        assert!(shore.height_at(0.0, 0.0) >= 4.5 + 0.15 - 1e-4);
    }

    #[test]
    fn height_matches_grid_points_and_mesh_triangles() {
        let t = bumpy();
        // At grid points.
        for (x, z) in [(-10.0, -10.0), (0.0, 0.0), (3.0, -7.0)] {
            let expected = (x * 0.7f32).sin() * 2.0 + (z * 0.4f32).cos() * 3.0;
            assert!((t.height_at(x, z) - expected).abs() < 1e-4);
        }
        // Inside triangles: the point must lie on the plane of the mesh
        // triangle that contains it.
        let mesh = t.mesh();
        for tri in mesh.indices.chunks_exact(3).step_by(37) {
            let p = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(mesh.vertices[i as usize].position));
            let c = (p[0] + p[1] + p[2]) / 3.0;
            assert!((t.height_at(c.x, c.z) - c.y).abs() < 1e-4, "{c:?}");
            // And triangles face up.
            assert!((p[1] - p[0]).cross(p[2] - p[0]).y > 0.0);
        }
    }

    #[test]
    fn distance_field_measures_polylines() {
        let mut f = DistanceField::new(20.0, 0.5);
        f.add(&[Vec2::new(-10.0, 0.0), Vec2::new(10.0, 0.0)], 1.0, 5.0);
        assert!((f.at(Vec2::new(0.0, 3.0)) - 2.0).abs() < 0.05);
        assert!(f.at(Vec2::new(0.0, 0.5)) < 0.0);
        assert_eq!(f.at(Vec2::new(0.0, 18.0)), f32::MAX);
    }

    #[test]
    fn spline_passes_through_control_points() {
        let pts = [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)];
        let s = spline(&pts, 1.0);
        for p in pts {
            assert!(s.iter().any(|q| (*q - p).length() < 1e-4));
        }
    }
}
