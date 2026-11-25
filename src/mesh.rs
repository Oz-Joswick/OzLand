//! CPU-side triangle mesh data: vertices plus a u32 index buffer.
//!
//! Everything the renderer draws, whether loaded from a file or built in
//! code, ends up as a [`MeshData`] and is uploaded to the GPU as a vertex
//! buffer and an index buffer.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

use crate::collision::Aabb;

/// One vertex as it is laid out in the GPU vertex buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Linear RGBA, multiplied with the material color.
    pub color: [f32; 4],
    /// Direction of increasing U, plus the handedness of the bitangent in
    /// `w` (bitangent = cross(normal, tangent) * w points along increasing V).
    /// Used for normal mapping.
    pub tangent: [f32; 4],
}

pub const DEFAULT_TANGENT: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// How texture coordinates are laid out on a box.
#[derive(Clone, Copy)]
pub enum BoxUv {
    /// The texture repeats every `meters`, aligned to world space, so walls
    /// of any size show the same brick size. Texture "up" is world up.
    World(f32),
    /// The whole texture is stretched over each face (e.g. a crate side).
    PerFace,
}

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// Normal, tangent and whether the corners must be reversed to face `outward`.
fn face_frame(p: [Vec3; 3], uv: [Vec2; 3], outward: Vec3) -> (Vec3, [f32; 4], bool) {
    let mut n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
    let flip = n.dot(outward) < 0.0;
    if flip {
        n = -n;
    }
    let (dp1, dp2) = (p[1] - p[0], p[2] - p[0]);
    let (du1, du2) = (uv[1] - uv[0], uv[2] - uv[0]);
    let det = du1.x * du2.y - du2.x * du1.y;
    if det.abs() < 1e-12 {
        return (n, DEFAULT_TANGENT, flip);
    }
    let r = 1.0 / det;
    let t = (dp1 * du2.y - dp2 * du1.y) * r;
    let b = (dp2 * du1.x - dp1 * du2.x) * r;
    let t = (t - n * n.dot(t)).normalize_or_zero();
    (n, [t.x, t.y, t.z, handedness(n, t, b)], flip)
}

/// glTF convention: the bitangent cross(n, t) * w points towards
/// *decreasing* V ("up" in the image), given `b` pointing towards increasing V.
fn handedness(n: Vec3, t: Vec3, b: Vec3) -> f32 {
    if n.cross(t).dot(b) > 0.0 { -1.0 } else { 1.0 }
}

impl MeshData {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Bounds of all vertex positions, or `None` for an empty mesh.
    pub fn bounds(&self) -> Option<Aabb> {
        let mut iter = self.vertices.iter().map(|v| Vec3::from(v.position));
        let first = iter.next()?;
        Some(iter.fold(Aabb { min: first, max: first }, |b, p| Aabb {
            min: b.min.min(p),
            max: b.max.max(p),
        }))
    }

    /// Bakes `transform` into positions, normals and tangents.
    pub fn transform(&mut self, transform: Mat4) {
        let linear = Mat3::from_mat4(transform);
        let normal_matrix = linear.inverse().transpose();
        let mirrored = transform.determinant() < 0.0;
        for v in &mut self.vertices {
            v.position = transform.transform_point3(v.position.into()).to_array();
            v.normal = (normal_matrix * Vec3::from(v.normal))
                .normalize_or_zero()
                .to_array();
            let t = (linear * Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2])).normalize_or_zero();
            let w = if mirrored { -v.tangent[3] } else { v.tangent[3] };
            v.tangent = [t.x, t.y, t.z, w];
        }
        // A mirroring transform turns triangles inside out; restore the winding.
        if mirrored {
            for tri in self.indices.chunks_exact_mut(3) {
                tri.swap(1, 2);
            }
        }
    }

    /// Replaces the normals with per-face normals computed from the triangle
    /// winding. Vertices are un-shared so each face gets its own normal; used
    /// when a model file has no normals.
    pub fn compute_flat_normals(&mut self) {
        let mut vertices = Vec::with_capacity(self.indices.len());
        for tri in self.indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| self.vertices[i as usize]);
            let (pa, pb, pc) = (Vec3::from(a.position), Vec3::from(b.position), Vec3::from(c.position));
            let n = (pb - pa).cross(pc - pa).normalize_or_zero().to_array();
            for mut v in [a, b, c] {
                v.normal = n;
                vertices.push(v);
            }
        }
        self.indices = (0..vertices.len() as u32).collect();
        self.vertices = vertices;
    }

    /// Computes per-vertex tangents from the UVs (for normal mapping), for
    /// meshes whose file did not provide them.
    pub fn compute_tangents(&mut self) {
        let mut tangents = vec![Vec3::ZERO; self.vertices.len()];
        let mut bitangents = vec![Vec3::ZERO; self.vertices.len()];
        for tri in self.indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| i as usize);
            let p = [a, b, c].map(|i| Vec3::from(self.vertices[i].position));
            let uv = [a, b, c].map(|i| Vec2::from(self.vertices[i].uv));
            let (dp1, dp2) = (p[1] - p[0], p[2] - p[0]);
            let (du1, du2) = (uv[1] - uv[0], uv[2] - uv[0]);
            let det = du1.x * du2.y - du2.x * du1.y;
            if det.abs() < 1e-12 {
                continue;
            }
            let r = 1.0 / det;
            let t = (dp1 * du2.y - dp2 * du1.y) * r;
            let bt = (dp2 * du1.x - dp1 * du2.x) * r;
            for i in [a, b, c] {
                tangents[i] += t;
                bitangents[i] += bt;
            }
        }
        for (i, v) in self.vertices.iter_mut().enumerate() {
            let n = Vec3::from(v.normal);
            let mut t = (tangents[i] - n * n.dot(tangents[i])).normalize_or_zero();
            if t == Vec3::ZERO {
                t = n.any_orthonormal_vector();
            }
            v.tangent = [t.x, t.y, t.z, handedness(n, t, bitangents[i])];
        }
    }

    /// Appends a flat quad (corners in order around the edge). The winding is
    /// fixed up so the front faces `outward`; normal and tangent are derived
    /// from the corners and UVs.
    pub fn push_quad(&mut self, p: [Vec3; 4], uv: [Vec2; 4], color: Vec4, outward: Vec3) {
        let (n, tangent, flip) = face_frame([p[0], p[1], p[2]], [uv[0], uv[1], uv[2]], outward);
        let base = self.vertices.len() as u32;
        for i in 0..4 {
            self.vertices.push(Vertex {
                position: p[i].to_array(),
                normal: n.to_array(),
                uv: uv[i].to_array(),
                color: color.to_array(),
                tangent,
            });
        }
        let idx = if flip { [0, 2, 1, 0, 3, 2] } else { [0, 1, 2, 0, 2, 3] };
        self.indices.extend(idx.map(|i| base + i));
    }

    /// Appends a single triangle facing `outward`.
    pub fn push_tri(&mut self, p: [Vec3; 3], uv: [Vec2; 3], color: Vec4, outward: Vec3) {
        let (n, tangent, flip) = face_frame(p, uv, outward);
        let base = self.vertices.len() as u32;
        for i in 0..3 {
            self.vertices.push(Vertex {
                position: p[i].to_array(),
                normal: n.to_array(),
                uv: uv[i].to_array(),
                color: color.to_array(),
                tangent,
            });
        }
        let idx = if flip { [0, 2, 1] } else { [0, 1, 2] };
        self.indices.extend(idx.map(|i| base + i));
    }

    /// Appends a smooth-sided cylinder from `base` along `axis` (any
    /// direction), with flat end caps. The texture wraps around it, repeating
    /// every `meters`, with texture "up" along the axis.
    #[allow(clippy::too_many_arguments)]
    pub fn push_cylinder(&mut self, base: Vec3, axis: Vec3, radius: f32, length: f32, segments: u32, color: Vec4, meters: f32) {
        let a = axis.normalize();
        let u = a.any_orthonormal_vector();
        let v = a.cross(u);
        let dir = |i: u32| {
            let t = i as f32 / segments as f32 * std::f32::consts::TAU;
            (u * t.cos() + v * t.sin(), -u * t.sin() + v * t.cos())
        };
        let circumference = std::f32::consts::TAU * radius;
        let first = self.vertices.len() as u32;
        for i in 0..=segments {
            let (d, t) = dir(i);
            let s = i as f32 / segments as f32 * circumference / meters;
            for h in [0.0, length] {
                self.vertices.push(Vertex {
                    position: (base + d * radius + a * h).to_array(),
                    normal: d.to_array(),
                    uv: [s, (length - h) / meters],
                    color: color.to_array(),
                    tangent: [t.x, t.y, t.z, 1.0],
                });
            }
        }
        for i in 0..segments {
            let b = first + i * 2;
            self.indices.extend_from_slice(&[b, b + 2, b + 3, b, b + 3, b + 1]);
        }
        for (center, outward) in [(base, -a), (base + a * length, a)] {
            for i in 0..segments {
                let (d0, _) = dir(i);
                let (d1, _) = dir(i + 1);
                let p = [center, center + d0 * radius, center + d1 * radius];
                let uv = p.map(|q| Vec2::new((q - center).dot(u), (q - center).dot(v)) / meters);
                self.push_tri(p, uv, color, outward);
            }
        }
    }

    /// Appends an axis-aligned box with outward-facing normals.
    pub fn push_box(&mut self, center: Vec3, size: Vec3, color: Vec4, uv_mode: BoxUv) {
        let h = size * 0.5;
        for n in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
            // Texture axes: on walls U runs left-to-right as seen from
            // outside and V runs downwards; on top/bottom faces they follow X/Z.
            let (u_axis, v_axis) = if n.y.abs() > 0.5 {
                (Vec3::X, Vec3::Z)
            } else {
                (Vec3::Y.cross(n), Vec3::NEG_Y)
            };
            let (a, b) = (u_axis * u_axis.dot(h).abs(), v_axis * v_axis.dot(h).abs());
            let face_center = center + n * n.dot(h).abs();
            let corners = [
                face_center - a - b,
                face_center + a - b,
                face_center + a + b,
                face_center - a + b,
            ];
            let uv = corners.map(|p| match uv_mode {
                BoxUv::World(meters) => Vec2::new(p.dot(u_axis), p.dot(v_axis)) / meters,
                BoxUv::PerFace => {
                    let o = p - center;
                    Vec2::new(
                        o.dot(u_axis) / (2.0 * u_axis.dot(h).abs()) + 0.5,
                        o.dot(v_axis) / (2.0 * v_axis.dot(h).abs()) + 0.5,
                    )
                }
            });
            self.push_quad(corners, uv, color, n);
        }
    }

    /// Appends a horizontal square at height `y`, facing up, textured every
    /// `meters`.
    #[cfg(test)]
    pub fn push_ground(&mut self, half_size: f32, y: f32, meters: f32, color: Vec4) {
        let s = half_size;
        let corners = [
            Vec3::new(-s, y, -s),
            Vec3::new(s, y, -s),
            Vec3::new(s, y, s),
            Vec3::new(-s, y, s),
        ];
        let uv = corners.map(|p| Vec2::new(p.x, p.z) / meters);
        self.push_quad(corners, uv, color, Vec3::Y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_faces_point_outward_with_valid_tangents() {
        let mut m = MeshData::default();
        let center = Vec3::new(1.0, 2.0, 3.0);
        m.push_box(center, Vec3::new(2.0, 1.0, 4.0), Vec4::ONE, BoxUv::World(1.0));
        assert_eq!(m.indices.len(), 36);
        for tri in m.indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| m.vertices[i as usize]);
            let (pa, pb, pc) = (Vec3::from(a.position), Vec3::from(b.position), Vec3::from(c.position));
            let winding = (pb - pa).cross(pc - pa);
            let n = Vec3::from(a.normal);
            assert!(winding.dot(n) > 0.0, "front face must match the normal");
            assert!(n.dot((pa + pb + pc) / 3.0 - center) > 0.0, "normal must point outward");
            let t = Vec3::new(a.tangent[0], a.tangent[1], a.tangent[2]);
            assert!((t.length() - 1.0).abs() < 1e-4 && t.dot(n).abs() < 1e-4);
            assert!(a.tangent[3].abs() == 1.0);
        }
    }

    #[test]
    fn computed_tangents_match_quad_tangents() {
        let mut quad = MeshData::default();
        quad.push_ground(1.0, 0.0, 1.0, Vec4::ONE);
        let expected = quad.vertices[0].tangent;
        quad.compute_tangents();
        for v in &quad.vertices {
            for i in 0..4 {
                assert!((v.tangent[i] - expected[i]).abs() < 1e-5);
            }
        }
    }
}
