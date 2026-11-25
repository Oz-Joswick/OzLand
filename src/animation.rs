//! Skeletal animation for glTF characters, done on the CPU.
//!
//! Loads the node hierarchy, skins (joints + inverse bind matrices),
//! skinned and rigid mesh parts, and animation clips. Each frame
//! [`AnimatedModel::pose`] samples one or two clips (blended, for smooth
//! transitions) into world-space node matrices, and [`AnimatedModel::skin`]
//! deforms a part's vertices with them. Characters are a few thousand
//! vertices, so this is cheap enough to do every frame and keeps the
//! renderer simple.

use std::path::Path;

use anyhow::{Context, Result};
use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

use crate::mesh::{DEFAULT_TANGENT, Vertex};
use crate::model::MaterialData;

/// A node's local transform.
#[derive(Clone, Copy)]
struct Trs {
    t: Vec3,
    r: Quat,
    s: Vec3,
}

impl Trs {
    fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.s, self.r, self.t)
    }
    fn blend(&self, other: &Trs, w: f32) -> Trs {
        Trs {
            t: self.t.lerp(other.t, w),
            r: self.r.slerp(other.r, w),
            s: self.s.lerp(other.s, w),
        }
    }
}

enum Values {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
}

struct Channel {
    node: usize,
    times: Vec<f32>,
    values: Values,
    step: bool,
}

pub struct Clip {
    pub name: String,
    pub duration: f32,
    channels: Vec<Channel>,
}

struct Skin {
    joints: Vec<usize>,
    inverse_bind: Vec<Mat4>,
}

/// One drawable piece of the character.
pub struct Part {
    /// Rest-pose vertices (in the mesh's own space).
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub material: MaterialData,
    node: usize,
    skin: Option<usize>,
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
}

pub struct AnimatedModel {
    pub parts: Vec<Part>,
    pub clips: Vec<Clip>,
    rest: Vec<Trs>,
    parents: Vec<Option<usize>>,
    /// Nodes ordered so every parent comes before its children.
    order: Vec<usize>,
    skins: Vec<Skin>,
}

/// Where in `times` `t` falls: (index of the key before, blend to the next).
fn find_key(times: &[f32], t: f32) -> (usize, f32) {
    if times.len() < 2 || t <= times[0] {
        return (0, 0.0);
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return (last, 0.0);
    }
    let i = times.partition_point(|&k| k <= t) - 1;
    let span = times[i + 1] - times[i];
    (i, if span > 0.0 { (t - times[i]) / span } else { 0.0 })
}

impl AnimatedModel {
    pub fn load(path: &Path) -> Result<Self> {
        let (doc, buffers, _images) =
            gltf::import(path).with_context(|| format!("loading character {}", path.display()))?;
        let buffer = |b: gltf::Buffer| Some(&buffers[b.index()][..]);

        let node_count = doc.nodes().len();
        let mut rest = vec![
            Trs {
                t: Vec3::ZERO,
                r: Quat::IDENTITY,
                s: Vec3::ONE,
            };
            node_count
        ];
        let mut parents = vec![None; node_count];
        for node in doc.nodes() {
            let (t, r, s) = node.transform().decomposed();
            rest[node.index()] = Trs {
                t: Vec3::from(t),
                r: Quat::from_array(r),
                s: Vec3::from(s),
            };
            for child in node.children() {
                parents[child.index()] = Some(node.index());
            }
        }
        let mut order = Vec::with_capacity(node_count);
        let mut stack: Vec<usize> = (0..node_count).filter(|&i| parents[i].is_none()).collect();
        while let Some(n) = stack.pop() {
            order.push(n);
            stack.extend(doc.nodes().nth(n).unwrap().children().map(|c| c.index()));
        }

        let skins = doc
            .skins()
            .map(|skin| {
                let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
                let inverse_bind = skin
                    .reader(buffer)
                    .read_inverse_bind_matrices()
                    .map(|m| m.map(|c| Mat4::from_cols_array_2d(&c)).collect())
                    .unwrap_or_else(|| vec![Mat4::IDENTITY; joints.len()]);
                Skin { joints, inverse_bind }
            })
            .collect();

        let mut parts = Vec::new();
        for node in doc.nodes() {
            let Some(mesh) = node.mesh() else { continue };
            for prim in mesh.primitives() {
                let reader = prim.reader(buffer);
                let Some(positions) = reader.read_positions() else { continue };
                let positions: Vec<[f32; 3]> = positions.collect();
                let normals: Vec<[f32; 3]> = reader
                    .read_normals()
                    .map(|n| n.collect())
                    .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
                let uvs: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|t| t.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.0; 2]; positions.len()]);
                let vertices = (0..positions.len())
                    .map(|i| Vertex {
                        position: positions[i],
                        normal: normals[i],
                        uv: uvs[i],
                        color: [1.0; 4],
                        tangent: DEFAULT_TANGENT,
                    })
                    .collect();
                let indices: Vec<u32> = reader
                    .read_indices()
                    .map(|i| i.into_u32().collect())
                    .unwrap_or_else(|| (0..positions.len() as u32).collect());
                let joints: Vec<[u16; 4]> = reader.read_joints(0).map(|j| j.into_u16().collect()).unwrap_or_default();
                let weights: Vec<[f32; 4]> = reader.read_weights(0).map(|w| w.into_f32().collect()).unwrap_or_default();
                let pbr = prim.material().pbr_metallic_roughness();
                let roughness = pbr.roughness_factor();
                let material = MaterialData {
                    base_color: Vec4::from(pbr.base_color_factor()),
                    specular: 0.6 * (1.0 - roughness) + 0.05,
                    shininess: 4.0 + 120.0 * (1.0 - roughness).powi(2),
                    ground_occlusion: 0.0,
                    ..Default::default()
                };
                let skinned = node.skin().is_some() && !joints.is_empty() && !weights.is_empty();
                parts.push(Part {
                    vertices,
                    indices,
                    material,
                    node: node.index(),
                    skin: if skinned { node.skin().map(|s| s.index()) } else { None },
                    joints,
                    weights,
                });
            }
        }

        let clips = doc
            .animations()
            .map(|anim| {
                let mut channels = Vec::new();
                let mut duration = 0.0f32;
                for channel in anim.channels() {
                    let reader = channel.reader(buffer);
                    let Some(times) = reader.read_inputs() else { continue };
                    let times: Vec<f32> = times.collect();
                    duration = duration.max(times.last().copied().unwrap_or(0.0));
                    let interpolation = channel.sampler().interpolation();
                    let cubic = interpolation == gltf::animation::Interpolation::CubicSpline;
                    // Cubic-spline keys store (in-tangent, value, out-tangent): keep the values.
                    let keep = |i: usize| !cubic || i % 3 == 1;
                    use gltf::animation::util::ReadOutputs;
                    let values = match reader.read_outputs() {
                        Some(ReadOutputs::Translations(v)) => Values::Translation(
                            v.enumerate().filter(|(i, _)| keep(*i)).map(|(_, x)| Vec3::from(x)).collect(),
                        ),
                        Some(ReadOutputs::Rotations(v)) => Values::Rotation(
                            v.into_f32()
                                .enumerate()
                                .filter(|(i, _)| keep(*i))
                                .map(|(_, x)| Quat::from_array(x).normalize())
                                .collect(),
                        ),
                        Some(ReadOutputs::Scales(v)) => Values::Scale(
                            v.enumerate().filter(|(i, _)| keep(*i)).map(|(_, x)| Vec3::from(x)).collect(),
                        ),
                        _ => continue, // morph target weights: not supported
                    };
                    channels.push(Channel {
                        node: channel.target().node().index(),
                        times,
                        values,
                        step: interpolation == gltf::animation::Interpolation::Step,
                    });
                }
                Clip {
                    name: anim.name().unwrap_or("").to_owned(),
                    duration,
                    channels,
                }
            })
            .collect();

        Ok(Self {
            parts,
            clips,
            rest,
            parents,
            order,
            skins,
        })
    }

    pub fn clip(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|c| c.name == name)
    }

    /// Local transforms of every node for `clip` at `time` seconds.
    fn sample(&self, clip: Option<usize>, time: f32) -> Vec<Trs> {
        let mut local = self.rest.clone();
        let Some(clip) = clip.and_then(|c| self.clips.get(c)) else {
            return local;
        };
        for ch in &clip.channels {
            let (i, mut w) = find_key(&ch.times, time);
            if ch.step {
                w = 0.0;
            }
            let j = (i + 1).min(ch.times.len() - 1);
            let trs = &mut local[ch.node];
            match &ch.values {
                Values::Translation(v) if i < v.len() => trs.t = v[i].lerp(v[j.min(v.len() - 1)], w),
                Values::Rotation(v) if i < v.len() => trs.r = v[i].slerp(v[j.min(v.len() - 1)], w),
                Values::Scale(v) if i < v.len() => trs.s = v[i].lerp(v[j.min(v.len() - 1)], w),
                _ => {}
            }
        }
        local
    }

    /// World-space node matrices for `clip` at `time`, optionally blended
    /// towards `blend = (clip, time, weight)` (weight 1 = fully the second).
    pub fn pose(&self, clip: Option<usize>, time: f32, blend: Option<(usize, f32, f32)>) -> Vec<Mat4> {
        let mut local = self.sample(clip, time);
        if let Some((other, other_time, w)) = blend {
            let second = self.sample(Some(other), other_time);
            for (a, b) in local.iter_mut().zip(&second) {
                *a = a.blend(b, w);
            }
        }
        let mut global = vec![Mat4::IDENTITY; local.len()];
        for &n in &self.order {
            let m = local[n].matrix();
            global[n] = match self.parents[n] {
                Some(p) => global[p] * m,
                None => m,
            };
        }
        global
    }

    /// The vertices of `part` deformed by a pose from [`Self::pose`], in
    /// model space.
    pub fn skin(&self, part: &Part, pose: &[Mat4]) -> Vec<Vertex> {
        let joint_matrices: Option<Vec<Mat4>> = part.skin.map(|s| {
            let skin = &self.skins[s];
            skin.joints
                .iter()
                .zip(&skin.inverse_bind)
                .map(|(&j, ib)| pose[j] * *ib)
                .collect()
        });
        part.vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let m = match &joint_matrices {
                    Some(mats) => {
                        let (j, w) = (part.joints[i], part.weights[i]);
                        (0..4).fold(Mat4::ZERO, |acc, k| acc + mats[j[k] as usize % mats.len()] * w[k])
                    }
                    None => pose[part.node],
                };
                let n = (Mat3::from_mat4(m) * Vec3::from(v.normal)).normalize_or_zero();
                Vertex {
                    position: m.transform_point3(v.position.into()).to_array(),
                    normal: n.to_array(),
                    ..*v
                }
            })
            .collect()
    }

    /// Bounds of the model in its rest pose (feet, height).
    pub fn rest_bounds(&self) -> (Vec3, Vec3) {
        let pose = self.pose(None, 0.0, None);
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for part in &self.parts {
            for v in self.skin(part, &pose) {
                lo = lo.min(v.position.into());
                hi = hi.max(v.position.into());
            }
        }
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_robot_with_its_animations() {
        let path = crate::model::asset_root().join("characters/robot/RobotExpressive.glb");
        if !path.exists() {
            return; // downloaded asset not present
        }
        let m = AnimatedModel::load(&path).unwrap();
        for name in ["Idle", "Walking", "Running", "Jump"] {
            assert!(m.clip(name).is_some(), "missing clip {name}");
        }
        let (lo, hi) = m.rest_bounds();
        assert!((hi - lo).y > 0.5, "unexpected size {lo:?}..{hi:?}");
        // Walking actually moves the vertices.
        let walk = m.clip("Walking").unwrap();
        let a = m.pose(Some(walk), 0.0, None);
        let b = m.pose(Some(walk), m.clips[walk].duration * 0.5, None);
        let part = m.parts.iter().find(|p| p.skin.is_some()).expect("a skinned part");
        let moved = m
            .skin(part, &a)
            .iter()
            .zip(m.skin(part, &b))
            .any(|(x, y)| (Vec3::from(x.position) - Vec3::from(y.position)).length() > 0.01);
        assert!(moved);
    }

    #[test]
    fn finds_keys() {
        assert_eq!(find_key(&[0.0, 1.0, 2.0], 1.5), (1, 0.5));
        assert_eq!(find_key(&[0.0, 1.0], -1.0), (0, 0.0));
        assert_eq!(find_key(&[0.0, 1.0], 5.0), (1, 0.0));
    }
}
